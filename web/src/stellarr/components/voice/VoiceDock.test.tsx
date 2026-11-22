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

  it("lights the logo's bars by the link's quality and says the quality and ping on them", () => {
    const cases: [CallVM["connectionQuality"], number | null, string, string][] = [
      ["good", 32, "q3", "Jakość połączenia: dobra, opóźnienie 32 ms."],
      ["fair", 140, "q2", "Jakość połączenia: średnia, opóźnienie 140 ms."],
      ["poor", 420, "q1", "Jakość połączenia: słaba, opóźnienie 420 ms."],
      ["unknown", null, "busy", "Trwa pomiar jakości połączenia."],
    ];
    for (const [connectionQuality, latencyMs, lit, said] of cases) {
      const { unmount } = render(
        <VoiceDock {...props({ call: { ...CALL, connectionQuality, latencyMs } })} />,
      );
      const bars = screen.getByRole("img", { name: said });
      expect(bars).toHaveClass("qbars", lit);
      expect(bars).toHaveAttribute("title", said);
      unmount();
    }

    // Until the call is up there is no quality to state.
    render(<VoiceDock {...props({ call: { ...CALL, headline: "reconnecting" } })} />);
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("draws an unsteady connection as fair, whatever the last reading said", () => {
    for (const connectionQuality of ["good", "poor", "unknown"] as const) {
      const { unmount } = render(
        <VoiceDock {...props({ call: { ...CALL, headline: "degraded", connectionQuality } })} />,
      );
      expect(status().querySelector(".qbars")).toHaveClass("q2");
      unmount();
    }
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
