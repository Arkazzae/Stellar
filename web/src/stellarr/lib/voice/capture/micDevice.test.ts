import { describe, expect, it, vi } from "vitest";

import {
  constraintsFor,
  openMicDevice,
  readBack,
  requestFor,
  switchOnBrowserAudio,
  type BrowserAudioRequest,
  type MicDevice,
} from "./micDevice";
import { DEFAULT_SETTINGS, type VoiceSettings } from "../settings";

type Reported = {
  echoCancellation?: boolean | "all" | "remote-only";
  noiseSuppression?: boolean;
  autoGainControl?: boolean;
};

/**
 * A device track. By default it reports back whatever it was last asked for,
 * as Chrome does; `reports` fixes what it says instead, and `apply` replaces
 * what `applyConstraints` does.
 */
function track(
  options: {
    reports?: Reported;
    modes?: Array<boolean | "all" | "remote-only">;
    apply?: (constraints: MediaTrackConstraints) => Promise<void>;
    getSettings?: () => Reported;
  } = {},
) {
  let asked: Reported = {};
  const fake = {
    kind: "audio",
    stop: vi.fn(),
    getCapabilities: () => ({ echoCancellation: options.modes ?? [true, false] }),
    getSettings: options.getSettings ?? (() => ({ ...asked, ...options.reports })),
    applyConstraints: vi.fn(async (constraints: MediaTrackConstraints) => {
      if (options.apply) return options.apply(constraints);
      asked = constraints as Reported;
    }),
    /** What `getUserMedia` handed the device, as Chrome reports it until changed. */
    opened(constraints: MediaTrackConstraints) {
      asked = constraints as Reported;
    },
  };
  return fake as unknown as MediaStreamTrack & {
    applyConstraints: ReturnType<typeof vi.fn>;
    stop: ReturnType<typeof vi.fn>;
    opened(constraints: MediaTrackConstraints): void;
  };
}

function stream(...tracks: MediaStreamTrack[]): MediaStream {
  return { getTracks: () => tracks, getAudioTracks: () => tracks } as unknown as MediaStream;
}

/** A `getUserMedia` that opens `device` with whatever it is asked. */
function opening(device: ReturnType<typeof track>) {
  return vi.fn(async (constraints: MediaStreamConstraints) => {
    device.opened(constraints.audio as MediaTrackConstraints);
    return stream(device);
  });
}

function settings(patch: Partial<VoiceSettings> = {}): VoiceSettings {
  return { ...DEFAULT_SETTINGS, ...patch };
}

const ASK_NOTHING: BrowserAudioRequest = {
  echoCancellation: false,
  noiseSuppression: false,
  autoGainControl: false,
  deviceId: null,
};

describe("what the settings ask of the browser", () => {
  it("leaves noise suppression to the browser only when it is the chosen engine", () => {
    // Two gates in series chew holes in speech, and the browser's would run
    // first: a worklet suppressor would only ever see already-damaged audio.
    for (const mode of ["rnnoise", "gtcrn", "speex", "off"] as const) {
      expect(requestFor(settings({ noiseSuppression: mode })).noiseSuppression).toBe(false);
    }
    expect(requestFor(settings({ noiseSuppression: "browser" })).noiseSuppression).toBe(true);
  });

  it("never lets the browser level the volume, whatever the setting", () => {
    // Automatic volume is our own leveller in the graph; the browser's comes
    // back only when the graph cannot level.
    for (const autoGainControl of [true, false]) {
      expect(requestFor(settings({ autoGainControl })).autoGainControl).toBe(false);
    }
  });

  it("keeps echo cancellation under the person's control, suppressor or not", () => {
    expect(requestFor(settings({ echoCancellation: false })).echoCancellation).toBe(false);
    expect(
      requestFor(settings({ echoCancellation: true, noiseSuppression: "rnnoise" }))
        .echoCancellation,
    ).toBe(true);
  });

  it("asks for mono at 48 kHz to match Opus", () => {
    const constraints = constraintsFor(requestFor(settings()));
    expect(constraints.channelCount).toBe(1);
    expect(constraints.sampleRate).toBe(48_000);
  });

  it("pins the chosen device and otherwise takes the system default", () => {
    expect(constraintsFor(requestFor(settings({ inputDeviceId: "mic-7" }))).deviceId).toEqual({
      exact: "mic-7",
    });
    expect(constraintsFor(requestFor(settings()))).not.toHaveProperty("deviceId");
  });
});

describe("opening the device", () => {
  it("opens once, with echo cancellation as the plain boolean, on the defaults", async () => {
    const device = track();
    const getUserMedia = opening(device);

    const opened = await openMicDevice(settings(), getUserMedia);

    expect(getUserMedia).toHaveBeenCalledOnce();
    expect(getUserMedia).toHaveBeenCalledWith({
      audio: {
        echoCancellation: true,
        autoGainControl: false,
        noiseSuppression: true,
        channelCount: 1,
        sampleRate: 48_000,
      },
      video: false,
    });
    expect(opened.track).toBe(device);
    expect(opened.active).toEqual({
      echoCancellation: "browser",
      noiseSuppression: true,
      autoGainControl: false,
    });
  });

  it.each([
    [[true, false, "remote-only", "all"]],
    [[true, false, "all"]],
    [[true, false, "remote-only"]],
  ] as const)(
    "never asks for a named echo-cancellation mode, even when the device offers %j",
    async (modes) => {
      // Chrome on macOS reports "all" and then ends the track when it may not
      // record the system's audio; "remote-only" is no more than `true`.
      const device = track({ modes: [...modes] });
      const getUserMedia = opening(device);

      const opened = await openMicDevice(settings({ echoCancellation: true }), getUserMedia);

      expect(getUserMedia).toHaveBeenCalledOnce();
      const audio = getUserMedia.mock.calls[0]?.[0].audio as MediaTrackConstraints;
      expect(audio.echoCancellation).toBe(true);
      expect(opened.track).toBe(device);
      expect(device.stop).not.toHaveBeenCalled();
    },
  );

  it("does not ask for capabilities at all", async () => {
    // Nothing is chosen from what the device advertises, so a track that
    // cannot report capabilities opens the same way.
    const device = { kind: "audio", stop: vi.fn() } as unknown as MediaStreamTrack;
    const getUserMedia = vi.fn(async () => stream(device));

    const opened = await openMicDevice(settings(), getUserMedia);

    expect(getUserMedia).toHaveBeenCalledOnce();
    expect(opened.active.echoCancellation).toBe("browser");
  });

  it("opens with echo cancellation off when it is switched off", async () => {
    const device = track();
    const getUserMedia = opening(device);

    const opened = await openMicDevice(settings({ echoCancellation: false }), getUserMedia);

    expect(getUserMedia.mock.calls[0]?.[0].audio).toMatchObject({ echoCancellation: false });
    expect(opened.active.echoCancellation).toBe("none");
  });

  it("asks for the browser's suppressor only when it is the chosen one", async () => {
    for (const [mode, expected] of [
      ["browser", true],
      ["rnnoise", false],
      ["off", false],
    ] as const) {
      const device = track();
      const getUserMedia = opening(device);
      const opened = await openMicDevice(settings({ noiseSuppression: mode }), getUserMedia);
      expect(getUserMedia.mock.calls[0]?.[0].audio).toMatchObject({ noiseSuppression: expected });
      expect(opened.active.noiseSuppression).toBe(expected);
    }
  });
});

describe("what the browser reports doing", () => {
  const asked: BrowserAudioRequest = {
    echoCancellation: true,
    noiseSuppression: true,
    autoGainControl: false,
    deviceId: null,
  };

  it.each([
    [true, "browser"],
    [false, "none"],
    ["all", "all"],
    ["remote-only", "remote-only"],
  ] as const)("reports echo cancellation the browser says is %j as %s", (reported, active) => {
    const device = track({ reports: { echoCancellation: reported } });
    expect(readBack(device, asked).echoCancellation).toBe(active);
  });

  it("takes an unreported setting as what the accepted request asked", () => {
    const device = track({ getSettings: () => ({}) });
    expect(readBack(device, asked)).toEqual({
      echoCancellation: "browser",
      noiseSuppression: true,
      autoGainControl: false,
    });
    expect(readBack(device, ASK_NOTHING)).toEqual({
      echoCancellation: "none",
      noiseSuppression: false,
      autoGainControl: false,
    });
  });

  it("takes the device's word over the request, either way", () => {
    // A device that cannot cancel echo or suppress noise says so, and one
    // that does either regardless of the request is not hidden either.
    const refusing = track({ reports: { echoCancellation: false, noiseSuppression: false } });
    expect(readBack(refusing, asked)).toMatchObject({
      echoCancellation: "none",
      noiseSuppression: false,
    });
    const insisting = track({ reports: { noiseSuppression: true, autoGainControl: true } });
    expect(readBack(insisting, ASK_NOTHING)).toMatchObject({
      noiseSuppression: true,
      autoGainControl: true,
    });
  });

  it("falls back to the request when reading the settings throws", () => {
    const device = track({
      getSettings: () => {
        throw new DOMException("gone", "InvalidStateError");
      },
    });
    expect(readBack(device, asked)).toEqual({
      echoCancellation: "browser",
      noiseSuppression: true,
      autoGainControl: false,
    });
  });
});

describe("opening failures", () => {
  const fail = (name: string) => () => Promise.reject(new DOMException("x", name));

  it.each([
    ["NotAllowedError", "denied"],
    ["SecurityError", "denied"],
    ["NotFoundError", "unavailable"],
    ["OverconstrainedError", "unavailable"],
    ["AbortError", "unavailable"],
  ])("maps %s to %s", async (name, kind) => {
    await expect(openMicDevice(settings(), fail(name))).rejects.toMatchObject({ kind });
  });

  it("treats an error that is not a DOMException by its name", async () => {
    const blocked = Object.assign(new Error("blocked"), { name: "NotAllowedError" });

    await expect(openMicDevice(settings(), () => Promise.reject(blocked))).rejects.toMatchObject({
      kind: "denied",
    });
  });

  it("treats a rejection with no name as an unavailable microphone", async () => {
    await expect(openMicDevice(settings(), () => Promise.reject(undefined))).rejects.toMatchObject(
      { kind: "unavailable" },
    );
  });

  it("stops the tracks of a stream that has no audio before failing", async () => {
    const video = { stop: vi.fn() };
    const noAudio = {
      getTracks: () => [video],
      getAudioTracks: () => [],
    } as unknown as MediaStream;

    await expect(openMicDevice(settings(), async () => noAudio)).rejects.toMatchObject({
      kind: "unavailable",
    });
    expect(video.stop).toHaveBeenCalledOnce();
  });
});

describe("switching the browser's own processing back on", () => {
  async function opened(
    device: ReturnType<typeof track>,
    patch: Partial<VoiceSettings> = {},
  ): Promise<MicDevice> {
    return openMicDevice(settings({ noiseSuppression: "rnnoise", ...patch }), opening(device));
  }

  it("sends the whole request again, so echo cancellation and the device stay put", async () => {
    // applyConstraints replaces the track's constraint set: a suppressor
    // failure must not quietly drop echo cancellation or the chosen device.
    const device = track();
    const before = await opened(device, { inputDeviceId: "mic-3" });

    const after = await switchOnBrowserAudio(before, ["noiseSuppression"]);

    expect(device.applyConstraints).toHaveBeenCalledWith({
      echoCancellation: true,
      autoGainControl: false,
      noiseSuppression: true,
      channelCount: 1,
      sampleRate: 48_000,
      deviceId: { exact: "mic-3" },
    });
    expect(after.active).toEqual({
      echoCancellation: "browser",
      noiseSuppression: true,
      autoGainControl: false,
    });
    expect(after.track).toBe(device);
  });

  it("switches on only what was asked for and reports each honestly", async () => {
    const device = track();
    const after = await switchOnBrowserAudio(await opened(device), ["autoGainControl"]);

    expect(device.applyConstraints).toHaveBeenCalledWith(
      expect.objectContaining({ autoGainControl: true, noiseSuppression: false }),
    );
    expect(after.active).toMatchObject({ noiseSuppression: false, autoGainControl: true });
  });

  it("does nothing, and claims nothing, when nothing was lost", async () => {
    const device = track();
    const before = await opened(device);

    const after = await switchOnBrowserAudio(before, []);

    expect(device.applyConstraints).not.toHaveBeenCalled();
    expect(after).toBe(before);
    expect(after.active).toMatchObject({ noiseSuppression: false, autoGainControl: false });
  });

  it("does not ask again for what the device already has", async () => {
    const device = track();
    const before = await opened(device, { noiseSuppression: "browser" });

    await switchOnBrowserAudio(before, ["noiseSuppression"]);

    expect(device.applyConstraints).not.toHaveBeenCalled();
  });

  it("keeps the device as it was when the browser refuses the change", async () => {
    const device = track({
      apply: async () => {
        throw new DOMException("cannot", "OverconstrainedError");
      },
    });
    const before = await opened(device);

    const after = await switchOnBrowserAudio(before, ["noiseSuppression", "autoGainControl"]);

    expect(after).toBe(before);
    expect(after.active).toMatchObject({ noiseSuppression: false, autoGainControl: false });
    expect(device.stop).not.toHaveBeenCalled();
  });

  it("admits when the change is accepted but silently ignored", async () => {
    // Some devices resolve applyConstraints and change nothing.
    const device = track({ apply: async () => {} });
    const after = await switchOnBrowserAudio(await opened(device), ["noiseSuppression"]);

    expect(after.active.noiseSuppression).toBe(false);
  });

  it("trusts an accepted change the device does not report on", async () => {
    const device = track({ getSettings: () => ({}) });
    const after = await switchOnBrowserAudio(await opened(device), ["noiseSuppression"]);

    expect(after.active.noiseSuppression).toBe(true);
  });

  it("leaves a track that cannot take constraints as it was", async () => {
    const device = { kind: "audio", stop: vi.fn() } as unknown as MediaStreamTrack;
    const before = await openMicDevice(
      settings({ noiseSuppression: "rnnoise" }),
      async () => stream(device),
    );

    const after = await switchOnBrowserAudio(before, ["noiseSuppression"]);

    expect(after).toBe(before);
    expect(after.active.noiseSuppression).toBe(false);
  });
});
