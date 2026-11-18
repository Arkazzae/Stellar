import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_SETTINGS, type VoiceSettings } from "@/lib/voice/settings";
import { FakeVoiceClient } from "@/test/fakes";

import { NOISE_SETTLE_MS } from "./useSettledVoiceSettings";
import { useVoiceSettingsSync } from "./useVoiceSettingsSync";
import { VoiceHolder } from "./voiceHolder";

function microphone(deviceId: string): MediaDeviceInfo {
  return {
    kind: "audioinput",
    deviceId,
    groupId: "group",
    label: deviceId,
    toJSON: () => ({}),
  };
}

/** A `navigator.mediaDevices` whose device list the test replaces and announces. */
function installMediaDevices(initial: MediaDeviceInfo[]) {
  let devices = initial;
  const events = new EventTarget();
  const media = Object.assign(events, { enumerateDevices: vi.fn(async () => devices) });
  Object.defineProperty(navigator, "mediaDevices", { value: media, configurable: true });
  return {
    media,
    async replace(next: MediaDeviceInfo[]) {
      devices = next;
      events.dispatchEvent(new Event("devicechange"));
      await vi.advanceTimersByTimeAsync(500);
    },
  };
}

function setup(settings: VoiceSettings = DEFAULT_SETTINGS) {
  const voiceSettings = {
    getSnapshot: () => settings,
    subscribe: () => () => {},
    update: vi.fn(),
    reset: vi.fn(),
  };
  const soundboard = {
    setVolume: vi.fn(),
    setDeafened: vi.fn(),
    setArrivalsAudible: vi.fn(),
  };
  const holder = new VoiceHolder();
  const client = new FakeVoiceClient();
  holder.attach(client);
  const ports = { voiceSettings, soundboard } as unknown as Parameters<
    typeof useVoiceSettingsSync
  >[0]["ports"];
  return { voiceSettings, soundboard, holder, client, ports };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  Reflect.deleteProperty(navigator, "mediaDevices");
});

describe("useVoiceSettingsSync", () => {
  it("applies the persisted settings to the live client and the soundboard", () => {
    const { holder, client, ports, soundboard } = setup({ ...DEFAULT_SETTINGS, soundboardVolume: 40 });

    renderHook(() =>
      useVoiceSettingsSync({ holder, ports, userId: "me", arrivals: true, deafened: false }),
    );

    expect(client.applySettings).toHaveBeenCalledWith(expect.objectContaining({ soundboardVolume: 40 }));
    expect(soundboard.setVolume).toHaveBeenCalledWith(40);
  });

  it("tells the soundboard whether arrivals are audible, and follows the preference", () => {
    const { holder, ports, soundboard } = setup();
    const view = renderHook(
      ({ arrivals }) =>
        useVoiceSettingsSync({ holder, ports, userId: "me", arrivals, deafened: false }),
      { initialProps: { arrivals: true } },
    );
    expect(soundboard.setArrivalsAudible).toHaveBeenLastCalledWith(true);

    view.rerender({ arrivals: false });

    expect(soundboard.setArrivalsAudible).toHaveBeenLastCalledWith(false);
  });

  it("deafens the client and the soundboard", () => {
    const { holder, client, ports, soundboard } = setup();
    renderHook(() =>
      useVoiceSettingsSync({ holder, ports, userId: "me", arrivals: true, deafened: true }),
    );
    expect(client.setDeafened).toHaveBeenLastCalledWith(true);
    expect(soundboard.setDeafened).toHaveBeenLastCalledWith(true);
  });

  it("falls back to the system route when the chosen microphone is unplugged", async () => {
    const devices = installMediaDevices([microphone("usb"), microphone("built-in")]);
    const { holder, ports, voiceSettings } = setup({ ...DEFAULT_SETTINGS, inputDeviceId: "usb" });
    renderHook(() =>
      useVoiceSettingsSync({ holder, ports, userId: "me", arrivals: true, deafened: false }),
    );
    await vi.advanceTimersByTimeAsync(0);
    expect(voiceSettings.update).not.toHaveBeenCalled();

    await devices.replace([microphone("built-in")]);

    expect(voiceSettings.update).toHaveBeenCalledWith({ inputDeviceId: null });
  });

  it("keeps the choice while the chosen microphone is still there", async () => {
    const devices = installMediaDevices([microphone("usb")]);
    const { holder, ports, voiceSettings } = setup({ ...DEFAULT_SETTINGS, inputDeviceId: "usb" });
    renderHook(() =>
      useVoiceSettingsSync({ holder, ports, userId: "me", arrivals: true, deafened: false }),
    );
    await vi.advanceTimersByTimeAsync(0);

    await devices.replace([microphone("usb"), microphone("headset")]);

    expect(voiceSettings.update).not.toHaveBeenCalled();
  });

  it("does not watch devices while signed out", async () => {
    const devices = installMediaDevices([microphone("usb")]);
    const { holder, ports } = setup();

    renderHook(() =>
      useVoiceSettingsSync({ holder, ports, userId: null, arrivals: true, deafened: false }),
    );
    await vi.advanceTimersByTimeAsync(500);

    expect(devices.media.enumerateDevices).not.toHaveBeenCalled();
  });

  it("stops watching devices on unmount", async () => {
    const devices = installMediaDevices([microphone("usb")]);
    const { holder, ports, voiceSettings } = setup({ ...DEFAULT_SETTINGS, inputDeviceId: "usb" });
    const view = renderHook(() =>
      useVoiceSettingsSync({ holder, ports, userId: "me", arrivals: true, deafened: false }),
    );
    await vi.advanceTimersByTimeAsync(0);
    view.unmount();

    await devices.replace([]);

    expect(voiceSettings.update).not.toHaveBeenCalled();
  });

  it("hands the client only the noise mode that stays, not those passed on the way to it", () => {
    let current: VoiceSettings = { ...DEFAULT_SETTINGS, noiseSuppression: "off" };
    const listeners = new Set<() => void>();
    const { holder, client, soundboard } = setup();
    const voiceSettings = {
      getSnapshot: () => current,
      subscribe: (listener: () => void) => {
        listeners.add(listener);
        return () => void listeners.delete(listener);
      },
      update: vi.fn(),
      reset: vi.fn(),
    };
    const ports = { voiceSettings, soundboard } as unknown as Parameters<
      typeof useVoiceSettingsSync
    >[0]["ports"];
    const change = (patch: Partial<VoiceSettings>) =>
      act(() => {
        current = { ...current, ...patch };
        for (const listener of listeners) listener();
      });
    renderHook(() =>
      useVoiceSettingsSync({ holder, ports, userId: "me", arrivals: true, deafened: false }),
    );
    client.applySettings.mockClear();

    // The dock's button, pressed three times to reach Speex.
    change({ noiseSuppression: "rnnoise" });
    act(() => vi.advanceTimersByTime(300));
    change({ noiseSuppression: "gtcrn" });
    act(() => vi.advanceTimersByTime(300));
    change({ noiseSuppression: "speex" });
    // Anything else still goes through at once.
    change({ outputVolume: 70 });
    act(() => vi.advanceTimersByTime(NOISE_SETTLE_MS));

    const modes = client.applySettings.mock.calls.map(([applied]) => applied.noiseSuppression);
    expect(modes).not.toContain("rnnoise");
    expect(modes).not.toContain("gtcrn");
    expect(modes.at(-1)).toBe("speex");
    expect(client.applySettings).toHaveBeenCalledWith(
      expect.objectContaining({ noiseSuppression: "off", outputVolume: 70 }),
    );
  });
});
