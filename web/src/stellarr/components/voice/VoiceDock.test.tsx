import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { voiceSettings } from "@/components/account/panes/voiceTestFixtures";
import type { CallVM } from "@/components/view-models";

import { VoiceDock, type VoiceDockProps } from "./VoiceDock";

const CALL: CallVM = {
  channelId: "voice-1",
  channelName: "pogaduchy",
  startedAt: Date.now(),
  muted: false,
  deafened: false,
  cameraOn: false,
  cameraPending: false,
  sharing: false,
  latencyMs: 42,
  connectionQuality: "good",
  headline: "connected",
};

function props(overrides: Partial<VoiceDockProps> = {}): VoiceDockProps {
  return {
    call: CALL,
    sounds: [],
    settings: voiceSettings,
    onChangeSettings: vi.fn(),
    onOpenSoundSettings: vi.fn(),
    cameraSupported: true,
    onToggleMute: vi.fn(),
    onToggleDeafen: vi.fn(),
    onToggleCamera: vi.fn(),
    onToggleShare: vi.fn(),
    onLeave: vi.fn(),
    onPlaySound: vi.fn(),
    onReturnToChannel: vi.fn(),
    ...overrides,
  };
}

const status = () => document.querySelector<HTMLElement>(".dock-status")!;
const caret = () => screen.getByRole("button", { name: "Jakość i ustawienia mikrofonu" });

describe("VoiceDock status line", () => {
  it("says in words only what is wrong, in place of the time", () => {
    const cases: [Partial<CallVM>, boolean, string, string][] = [
      [{ headline: "connecting" }, false, "Łączenie…", "busy"],
      [{ headline: "requesting-mic" }, false, "Czekam na mikrofon", "busy"],
      [{ headline: "reconnecting" }, false, "Ponowne łączenie…", "warn"],
      [{ headline: "degraded" }, false, "Połączenie niestabilne", "warn"],
      [{ headline: "failed" }, false, "Brak połączenia audio", "bad"],
      [{ headline: "screen", sharing: true }, false, "Twój ekran", "acc"],
      [{ headline: "connected" }, true, "Dzwonię…", "busy"],
    ];
    for (const [call, ringing, words, tone] of cases) {
      const { unmount } = render(
        <VoiceDock {...props({ call: { ...CALL, ...call }, ringing })} />,
      );
      expect(within(status()).getByText("pogaduchy")).toBeInTheDocument();
      expect(within(status()).getByText(words)).toHaveClass("ds-time", tone);
      // The words take the time's place.
      expect(status().querySelector("time")).toBeNull();
      unmount();
    }

    render(<VoiceDock {...props()} />);
    expect(status().querySelector(".ds-time time")).not.toBeNull();
  });

  it("says the link's quality and ping on the time, and for a screen reader", () => {
    const cases: [CallVM["connectionQuality"], number | null, string][] = [
      ["good", 32, "Jakość połączenia: dobra, opóźnienie 32 ms."],
      ["fair", 140, "Jakość połączenia: średnia, opóźnienie 140 ms."],
      ["poor", 420, "Jakość połączenia: słaba, opóźnienie 420 ms."],
      ["unknown", null, "Trwa pomiar jakości połączenia."],
    ];
    for (const [connectionQuality, latencyMs, said] of cases) {
      const { unmount } = render(
        <VoiceDock {...props({ call: { ...CALL, connectionQuality, latencyMs } })} />,
      );
      expect(status().querySelector(".ds-time")).toHaveAttribute("title", said);
      expect(within(status()).getByText(said)).toHaveClass("sr-only");
      unmount();
    }

    // Until the call is up there is no quality to state.
    render(<VoiceDock {...props({ call: { ...CALL, headline: "reconnecting" } })} />);
    expect(status().querySelector(".sr-only")).toBeNull();
  });

  it("leads with the others' faces instead of bars, three at most and the rest counted", () => {
    const person = (id: string) => ({ id, displayName: id, initials: id.slice(0, 2), presence: "online" as const });
    const { unmount } = render(
      <VoiceDock {...props({ others: ["ada", "bo", "cy", "dan", "ewa"].map(person) })} />,
    );
    expect(status().querySelectorAll(".ds-face")).toHaveLength(3);
    expect(within(status()).getByText("+2")).toHaveClass("ds-more");
    expect(status().querySelector(".qbars")).toBeNull();
    unmount();

    // Alone on the channel, nothing stands before its name.
    render(<VoiceDock {...props({ others: [] })} />);
    expect(status().querySelector(".ds-faces")).toBeNull();
  });

  it("fades the faces while the call is not up", () => {
    const others = [{ id: "ada", displayName: "Ada", initials: "AD", presence: "online" as const }];
    render(<VoiceDock {...props({ others, call: { ...CALL, headline: "reconnecting" } })} />);
    expect(status().querySelector(".ds-faces")).toHaveClass("faded");
  });
});

describe("VoiceDock microphone menu", () => {
  it("opens the call bar's microphone menu from its arrow and changes the settings", async () => {
    const user = userEvent.setup();
    const p = props();
    render(<VoiceDock {...p} />);

    await user.click(caret());
    const menu = within(document.querySelector<HTMLElement>(".voice-dock .mic-menu")!);
    expect(menu.getByRole("radiogroup", { name: "Jakość dźwięku" })).toBeVisible();
    expect(menu.getByRole("radiogroup", { name: "Redukcja szumu" })).toBeVisible();
    await user.click(menu.getByRole("radio", { name: /Automatycznie/ }));
    expect(p.onChangeSettings).toHaveBeenLastCalledWith({ opusProfile: "auto" });
    await user.click(menu.getByRole("radio", { name: "RNNoise" }));
    expect(p.onChangeSettings).toHaveBeenLastCalledWith({ noiseSuppression: "rnnoise" });

    await user.click(menu.getByRole("button", { name: "Więcej ustawień dźwięku" }));
    expect(p.onOpenSoundSettings).toHaveBeenCalledOnce();
    expect(caret()).toHaveAttribute("aria-expanded", "false");
  });

  it("carries the download ring on the microphone's arrow while the chosen engine downloads", () => {
    const settings = { ...voiceSettings, noiseSuppression: "gtcrn" as const };
    const { rerender } = render(
      <VoiceDock {...props({ settings, suppressorDownload: { kind: "gtcrn", percent: 40 } })} />,
    );

    expect(caret()).toHaveAttribute("aria-busy", "true");
    expect(caret()).toHaveAttribute("title", "GTCRN: pobrano 40%");
    expect(caret().querySelector(".progress-ring")).not.toBeNull();

    rerender(<VoiceDock {...props({ settings, suppressorDownload: null })} />);
    expect(caret()).not.toHaveAttribute("aria-busy");
    expect(caret().querySelector(".progress-ring")).toBeNull();
  });
});

describe("VoiceDock with the call's view open", () => {
  it("keeps its status line and the way out, and leaves the controls to the bar", async () => {
    const user = userEvent.setup();
    const p = props({ collapsed: true });
    render(<VoiceDock {...p} />);

    expect(within(status()).getByText("pogaduchy")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Mikrofon" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Jakość i ustawienia mikrofonu" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Soundboard" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Rozłącz" }));
    expect(p.onLeave).toHaveBeenCalledOnce();
  });

  it("does not offer a way back to the view it is already showing", () => {
    render(
      <VoiceDock
        {...props({ collapsed: true, call: { ...CALL, headline: "screen", sharing: true } })}
      />,
    );

    expect(within(status()).getByText("Twój ekran")).toBeInTheDocument();
    expect(screen.queryByTitle("Wróć do kanału")).toBeNull();
  });
});
