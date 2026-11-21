import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { answerConfirm, renderWithConfirm } from "@/test/confirm";

import { decodeWaveform } from "@/components/settings/sound/WaveformTrimmer";
import { SoundboardPane, type SoundboardPaneProps } from "./SoundboardPane";

vi.mock("@/components/settings/sound/WaveformTrimmer", async () => {
  const actual = await vi.importActual("@/components/settings/sound/WaveformTrimmer");
  return {
    ...actual,
    decodeWaveform: vi.fn(),
  };
});

const SOUND = {
  id: "drum",
  name: "Bęben",
  glyph: "🥁",
  url: "/sounds/drum.wav",
  durationSeconds: 2.4,
  trimStartSeconds: 0.2,
  trimEndSeconds: 1.8,
};

function props(overrides: Partial<SoundboardPaneProps> = {}): SoundboardPaneProps {
  return {
    supported: true,
    sounds: [SOUND],
    everyoneCanPlay: true,
    onEveryoneCanPlayChange: vi.fn(),
    rateLimited: true,
    onRateLimitedChange: vi.fn(),
    onCreate: vi.fn().mockResolvedValue(undefined),
    onUpdate: vi.fn().mockResolvedValue(undefined),
    onDelete: vi.fn().mockResolvedValue(undefined),
    playback: { soundboardVolume: 100 },
    onReorder: vi.fn(),
    ...overrides,
  };
}

function stubSoundFetch() {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: true,
      arrayBuffer: vi.fn().mockResolvedValue(new ArrayBuffer(8)),
    }),
  );
}

function fileInput(container: HTMLElement): HTMLInputElement {
  return container.querySelector('input[type="file"]') as HTMLInputElement;
}

beforeEach(() => {
  vi.mocked(decodeWaveform).mockResolvedValue({ duration: 2.4, peaks: [0.2, 0.6, 1, 0.5] });
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("SoundboardPane editing", () => {
  it("adds a chosen file with the whole clip selected, named after the file", async () => {
    const user = userEvent.setup();
    vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:clip");
    vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const { container } = renderWithConfirm(<SoundboardPane {...props({ onCreate })} />);
    const file = new File(["audio"], "air horn.mp3", { type: "audio/mpeg" });

    await user.upload(fileInput(container), file);
    expect(await screen.findByRole("textbox", { name: "Nazwa" })).toHaveValue("air horn");
    await user.click(screen.getByRole("button", { name: "Zapisz dźwięk" }));

    expect(onCreate).toHaveBeenCalledWith({
      file,
      name: "air horn",
      glyph: "🔊",
      durationSeconds: 2.4,
      trimStartSeconds: 0,
      trimEndSeconds: 2.4,
    });
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "Nazwa" })).toBeNull());
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:clip");
  });

  it("caps the clip chosen from a long file at fifteen seconds", async () => {
    const user = userEvent.setup();
    vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:clip");
    vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
    vi.mocked(decodeWaveform).mockResolvedValue({ duration: 42, peaks: [0.5, 1] });
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const { container } = renderWithConfirm(<SoundboardPane {...props({ onCreate })} />);

    await user.upload(fileInput(container), new File(["audio"], "long.wav", { type: "audio/wav" }));
    await user.click(await screen.findByRole("button", { name: "Zapisz dźwięk" }));

    expect(onCreate).toHaveBeenCalledWith(
      expect.objectContaining({ durationSeconds: 42, trimStartSeconds: 0, trimEndSeconds: 15 }),
    );
  });

  it("saves the new name of an existing sound with its trim points", async () => {
    const user = userEvent.setup();
    stubSoundFetch();
    const onUpdate = vi.fn().mockResolvedValue(undefined);
    renderWithConfirm(<SoundboardPane {...props({ onUpdate })} />);

    await user.click(screen.getByRole("button", { name: "Edytuj dźwięk Bęben" }));
    const name = await screen.findByRole("textbox", { name: "Nazwa" });
    await user.clear(name);
    await user.type(name, "Werbel");
    await user.click(screen.getByRole("button", { name: "Zapisz dźwięk" }));

    expect(onUpdate).toHaveBeenCalledWith("drum", {
      name: "Werbel",
      glyph: "🥁",
      trimStartSeconds: 0.2,
      trimEndSeconds: 1.8,
    });
  });

  it("cannot save a sound without a name", async () => {
    const user = userEvent.setup();
    stubSoundFetch();
    renderWithConfirm(<SoundboardPane {...props()} />);

    await user.click(screen.getByRole("button", { name: "Edytuj dźwięk Bęben" }));
    await user.clear(await screen.findByRole("textbox", { name: "Nazwa" }));

    expect(screen.getByRole("button", { name: "Zapisz dźwięk" })).toBeDisabled();
  });

  it("keeps the editor open and says so when saving fails", async () => {
    const user = userEvent.setup();
    stubSoundFetch();
    const onUpdate = vi.fn().mockRejectedValue(new Error("rejected"));
    renderWithConfirm(<SoundboardPane {...props({ onUpdate })} />);

    await user.click(screen.getByRole("button", { name: "Edytuj dźwięk Bęben" }));
    await user.click(await screen.findByRole("button", { name: "Zapisz dźwięk" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Nie udało się zapisać dźwięku. Sprawdź format i zakres przycięcia.",
    );
    expect(screen.getByRole("textbox", { name: "Nazwa" })).toBeEnabled();
  });

  it("says when a chosen file cannot be read and opens no editor", async () => {
    const user = userEvent.setup();
    vi.mocked(decodeWaveform).mockRejectedValue(new Error("undecodable"));
    const { container } = renderWithConfirm(<SoundboardPane {...props()} />);

    await user.upload(fileInput(container), new File(["x"], "broken.mp3", { type: "audio/mpeg" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Nie udało się odczytać tego pliku audio.",
    );
    expect(screen.queryByRole("textbox", { name: "Nazwa" })).toBeNull();
  });

  it("says when the stored sound cannot be fetched for editing", async () => {
    const user = userEvent.setup();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false }));
    renderWithConfirm(<SoundboardPane {...props()} />);

    await user.click(screen.getByRole("button", { name: "Edytuj dźwięk Bęben" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Nie udało się wczytać wykresu tego dźwięku.",
    );
    expect(screen.queryByRole("textbox", { name: "Nazwa" })).toBeNull();
  });

  it("deletes a sound from the list and reports a failed deletion", async () => {
    const user = userEvent.setup();
    const onDelete = vi
      .fn<(soundId: string) => Promise<void>>()
      .mockRejectedValueOnce(new Error("gone"))
      .mockResolvedValue(undefined);
    renderWithConfirm(<SoundboardPane {...props({ onDelete })} />);

    await user.click(screen.getByRole("button", { name: "Usuń dźwięk Bęben" }));
    await answerConfirm(user, "Usuń");
    expect(await screen.findByRole("alert")).toHaveTextContent("Nie udało się usunąć dźwięku.");

    await user.click(screen.getByRole("button", { name: "Usuń dźwięk Bęben" }));
    await answerConfirm(user, "Usuń");
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(onDelete).toHaveBeenCalledTimes(2);
  });

  it("closes the emoji picker as soon as a chosen file starts to load", async () => {
    const user = userEvent.setup();
    stubSoundFetch();
    vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:clip");
    vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
    const { container } = renderWithConfirm(<SoundboardPane {...props()} />);
    await user.click(screen.getByRole("button", { name: "Edytuj dźwięk Bęben" }));
    await user.click(await screen.findByRole("button", { name: "Wybierz emoji dźwięku" }));
    expect(screen.getByRole("dialog", { name: "Wybierz emoji dźwięku" })).toBeVisible();

    let finishDecoding: (value: { duration: number; peaks: number[] }) => void = () => {};
    vi.mocked(decodeWaveform).mockReturnValue(
      new Promise((resolve) => {
        finishDecoding = resolve;
      }),
    );
    await user.click(screen.getByRole("button", { name: "Wszystkie dźwięki" }));
    fireEvent.change(fileInput(container), {
      target: { files: [new File(["audio"], "clap.wav", { type: "audio/wav" })] },
    });

    await act(async () => finishDecoding({ duration: 1, peaks: [0.5] }));
    expect(await screen.findByRole("textbox", { name: "Nazwa" })).toHaveValue("clap");
    expect(screen.queryByRole("dialog", { name: "Wybierz emoji dźwięku" })).toBeNull();
  });

  it("starts a fresh editor with the emoji picker closed when another stored sound is opened", async () => {
    const user = userEvent.setup();
    const second = { ...SOUND, id: "clap", name: "Klaśnięcie", glyph: "👏" };
    stubSoundFetch();
    renderWithConfirm(<SoundboardPane {...props({ sounds: [SOUND, second] })} />);
    await user.click(screen.getByRole("button", { name: "Edytuj dźwięk Bęben" }));
    await user.click(await screen.findByRole("button", { name: "Wybierz emoji dźwięku" }));
    expect(screen.getByRole("dialog", { name: "Wybierz emoji dźwięku" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Wszystkie dźwięki" }));
    await user.click(screen.getByRole("button", { name: "Edytuj dźwięk Klaśnięcie" }));

    expect(await screen.findByRole("textbox", { name: "Nazwa" })).toHaveValue("Klaśnięcie");
    expect(screen.queryByRole("dialog", { name: "Wybierz emoji dźwięku" })).toBeNull();
  });

  it("shows the error the store reports", () => {
    renderWithConfirm(<SoundboardPane {...props({ error: "Offline" })} />);

    expect(screen.getByRole("alert")).toHaveTextContent("Offline");
  });
});

describe("SoundboardPane preview", () => {
  interface FakeAudio {
    volume: number;
    muted: boolean;
    currentTime: number;
    readyState: number;
    play: ReturnType<typeof vi.fn>;
    pause: ReturnType<typeof vi.fn>;
    onloadedmetadata: (() => void) | null;
  }

  function stubAudio(play: () => Promise<void> = async () => undefined) {
    const created: FakeAudio[] = [];
    vi.stubGlobal(
      "Audio",
      vi.fn(function AudioMock(this: FakeAudio) {
        Object.assign(this, {
          volume: 1,
          muted: false,
          currentTime: 0,
          readyState: 0,
          play: vi.fn(play),
          pause: vi.fn(),
          onloadedmetadata: null,
        });
        created.push(this);
      }),
    );
    return created;
  }

  async function openPreview(overrides: Partial<SoundboardPaneProps>) {
    const user = userEvent.setup();
    stubSoundFetch();
    const view = renderWithConfirm(<SoundboardPane {...props(overrides)} />);
    await user.click(screen.getByRole("button", { name: "Edytuj dźwięk Bęben" }));
    await user.click(await screen.findByRole("button", { name: "Posłuchaj" }));
    return view;
  }

  it("plays at the soundboard level alone, on the routed speaker", async () => {
    const created = stubAudio();
    const route = vi.fn().mockResolvedValue(undefined);

    await openPreview({ playback: { soundboardVolume: 40, route } });

    const audio = created[0]!;
    expect(audio.volume).toBeCloseTo(0.4);
    expect(route).toHaveBeenCalledWith(audio);
    // Silent until it is sitting on the trim start.
    expect(audio.muted).toBe(true);
    act(() => audio.onloadedmetadata?.());
    expect(audio.muted).toBe(false);
    expect(audio.currentTime).toBe(0.2);
  });

  it("still plays when the chosen speaker cannot be routed to", async () => {
    const created = stubAudio();
    const route = vi.fn().mockRejectedValue(new Error("device gone"));

    await openPreview({ playback: { soundboardVolume: 100, route } });

    expect(created[0]!.play).toHaveBeenCalledOnce();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("follows the soundboard level while the clip is running", async () => {
    const created = stubAudio();
    const base = props({ playback: { soundboardVolume: 100 } });
    const user = userEvent.setup();
    stubSoundFetch();
    const view = renderWithConfirm(<SoundboardPane {...base} />);
    await user.click(screen.getByRole("button", { name: "Edytuj dźwięk Bęben" }));
    await user.click(await screen.findByRole("button", { name: "Posłuchaj" }));

    view.rerender(
      <ConfirmProvider>
        <SoundboardPane {...base} playback={{ soundboardVolume: 30 }} />
      </ConfirmProvider>,
    );
    expect(created[0]!.volume).toBeCloseTo(0.3);
  });

  it("says so when the browser blocks the preview", async () => {
    stubAudio(() => Promise.reject(new Error("NotAllowedError")));

    await openPreview({});

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Przeglądarka zablokowała podgląd dźwięku.",
    );
  });

  it("stops the preview when the editor is left", async () => {
    const created = stubAudio();
    const user = userEvent.setup();

    await openPreview({});
    await user.click(screen.getByRole("button", { name: "Wszystkie dźwięki" }));

    expect(created[0]!.pause).toHaveBeenCalled();
  });
});
