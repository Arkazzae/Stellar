import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { NoiseSuppressionMode, SuppressorDownload } from "@/ports";

import { VoiceDock, type VoiceDockProps } from "./VoiceDock";

function props(
  noiseSuppression: NoiseSuppressionMode,
  onChangeNoiseSuppression: VoiceDockProps["onChangeNoiseSuppression"],
  noiseDownload: SuppressorDownload | null = null,
): VoiceDockProps {
  return {
    call: {
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
    },
    sounds: [],
    soundboardVolume: 100,
    onChangeSoundboardVolume: vi.fn(),
    cameraSupported: true,
    noiseSuppression,
    noiseDownload,
    onChangeNoiseSuppression,
    onToggleMute: vi.fn(),
    onToggleDeafen: vi.fn(),
    onToggleCamera: vi.fn(),
    onToggleShare: vi.fn(),
    onLeave: vi.fn(),
    onPlaySound: vi.fn(),
    onReturnToChannel: vi.fn(),
  };
}

describe("VoiceDock noise suppression", () => {
  it("shows connection quality and latency beside the connected state", () => {
    render(<VoiceDock {...props("off", vi.fn())} />);

    expect(screen.getByText("Dobra · 42 ms")).toHaveClass(
      "voice-connection-quality",
      "good",
    );
    expect(
      screen.getByLabelText("Jakość połączenia: Dobra, opóźnienie 42 ms."),
    ).toBeInTheDocument();
  });

  it("cycles off, RNNoise, GTCRN, Speex and classic suppression beside disconnect", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const { rerender } = render(<VoiceDock {...props("off", onChange)} />);

    const off = screen.getByRole("button", {
      name: "Tłumienie: wyłączone. Następne: RNNoise.",
    });
    const disconnect = screen.getByRole("button", { name: "Rozłącz" });
    expect(disconnect.previousElementSibling).toBe(off);
    expect(off.querySelector(".noise-icon.off")).toBeInTheDocument();
    await user.click(off);
    expect(onChange).toHaveBeenLastCalledWith("rnnoise");

    rerender(<VoiceDock {...props("rnnoise", onChange)} />);
    const rnnoise = screen.getByRole("button", {
      name: "Tłumienie: RNNoise. Następne: GTCRN.",
    });
    expect(rnnoise.querySelector(".noise-icon.rnnoise")).toBeInTheDocument();
    expect(rnnoise).toHaveClass("noise", "rnnoise");
    await user.click(rnnoise);
    expect(onChange).toHaveBeenLastCalledWith("gtcrn");

    // GTCRN's files still on their way: the button says how far, and is busy.
    rerender(<VoiceDock {...props("gtcrn", onChange, { kind: "gtcrn", percent: 40 })} />);
    const gtcrn = screen.getByRole("button", {
      name: "Tłumienie: GTCRN, pobrano 40%. Następne: Speex.",
    });
    expect(gtcrn).toHaveAttribute("aria-busy", "true");
    await user.click(gtcrn);
    expect(onChange).toHaveBeenLastCalledWith("speex");

    // Pressed on past it, the download is no longer this button's to show.
    rerender(<VoiceDock {...props("speex", onChange, { kind: "gtcrn", percent: 90 })} />);
    await user.click(
      screen.getByRole("button", { name: "Tłumienie: Speex. Następne: klasyczne." }),
    );
    expect(onChange).toHaveBeenLastCalledWith("browser");

    rerender(<VoiceDock {...props("browser", onChange)} />);
    const browser = screen.getByRole("button", {
      name: "Tłumienie: klasyczne. Następne: wyłączone.",
    });
    expect(browser.querySelector(".noise-icon.browser")).toBeInTheDocument();
    expect(browser).toHaveClass("noise", "browser");
    await user.click(browser);
    expect(onChange).toHaveBeenLastCalledWith("off");
  });
});
