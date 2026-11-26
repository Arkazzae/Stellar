import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

import { polish } from "@/test/translators";

import { captureMicrophone, type MicCapture } from "./micPipeline";
import type * as MicPipeline from "./micPipeline";
import { DEFAULT_SETTINGS, type VoiceSettings } from "../settings";
import { VoiceStateStore } from "../store";
import type { SpeechDetector } from "./vad";
import { LocalMicrophone, TALK_RELEASE_TAIL_MS } from "./webrtcMicrophone";
import type { Link } from "../rtc/webrtcPeer";

vi.mock("./micPipeline", async (importOriginal) => ({
  ...(await importOriginal<typeof MicPipeline>()),
  captureMicrophone: vi.fn(),
}));

function fakeCapture(overrides: Partial<MicCapture> = {}): MicCapture {
  const raw = { enabled: true, contentHint: "", onended: null, stop: vi.fn() };
  return {
    track: raw,
    rawTrack: raw,
    stream: {},
    monitorStream: {},
    processed: false,
    noiseSuppression: "none",
    gateActive: false,
    autoGain: "none",
    setSensitivity: vi.fn(),
    setInputGain: vi.fn(),
    fadeIn: vi.fn(),
    resume: vi.fn(async () => true),
    dispose: vi.fn(),
    ...overrides,
  } as unknown as MicCapture;
}

let store: VoiceStateStore;
let speech: { add: ReturnType<typeof vi.fn>; remove: ReturnType<typeof vi.fn> };
let onDeviceLost: Mock<() => void>;
let microphone: LocalMicrophone;
let peers: Map<string, Link>;
let applyEncoding: Mock<() => Promise<void>>;
let rebuildLink: Mock<(link: Link) => Promise<void>>;
let talkingChanged: Mock<() => void>;
/** The stream the microphone and the camera are both sent in. */
const VOICE_STREAM = { id: "voice-a" } as MediaStream;

beforeEach(() => {
  vi.mocked(captureMicrophone).mockReset();
  peers = new Map();
  store = new VoiceStateStore();
  speech = { add: vi.fn(async () => true), remove: vi.fn() };
  onDeviceLost = vi.fn<() => void>();
  applyEncoding = vi.fn(async () => undefined);
  rebuildLink = vi.fn(async () => undefined);
  talkingChanged = vi.fn<() => void>();
  microphone = new LocalMicrophone({
    translate: polish,
    selfUserId: "u-a",
    store,
    speech: speech as unknown as SpeechDetector,
    voiceStream: () => VOICE_STREAM,
    links: () => peers.values(),
    applyEncoding,
    rebuildLink,
    onDeviceLost,
    talkingChanged,
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("LocalMicrophone", () => {
  it("starts sending a capture marked as speech, and listens to it before the gate", () => {
    const capture = fakeCapture();

    microphone.adopt(capture, DEFAULT_SETTINGS);

    expect(microphone.capture).toBe(capture);
    expect(capture.rawTrack.contentHint).toBe("speech");
    expect(speech.add).toHaveBeenCalledWith("u-a", capture.monitorStream, true);
  });

  it("opens a capture muted when the person is muted", () => {
    store.set({ muted: true });
    const processed = { enabled: true } as MediaStreamTrack;
    const capture = fakeCapture({ track: processed });

    microphone.adopt(capture, DEFAULT_SETTINGS);

    expect(capture.rawTrack.enabled).toBe(false);
    expect(processed.enabled).toBe(false);
  });

  it("mutes and unmutes both the device track and the processed one", () => {
    const processed = { enabled: true } as MediaStreamTrack;
    const capture = fakeCapture({ track: processed });
    microphone.adopt(capture, DEFAULT_SETTINGS);

    microphone.setMuted(true);
    expect([capture.rawTrack.enabled, processed.enabled]).toEqual([false, false]);

    microphone.setMuted(false);
    expect([capture.rawTrack.enabled, processed.enabled]).toEqual([true, true]);
  });

  it("reports what is really running rather than what was asked for", () => {
    const capture = fakeCapture({ noiseSuppression: "browser" });

    microphone.adopt(capture, { ...DEFAULT_SETTINGS, noiseSuppression: "gtcrn" });

    expect(store.getSnapshot().noiseSuppressionActive).toBe(false);
    expect(store.getSnapshot().degraded).toContain("GTCRN");

    microphone.adopt(
      fakeCapture({ noiseSuppression: "gtcrn", gateActive: true, autoGain: "worklet" }),
      { ...DEFAULT_SETTINGS, noiseSuppression: "gtcrn" },
    );
    expect(store.getSnapshot().noiseSuppressionActive).toBe(true);
    expect(store.getSnapshot().degraded).toBeNull();
  });

  it("tells its owner when the device ends under the call", () => {
    const capture = fakeCapture();
    microphone.adopt(capture, DEFAULT_SETTINGS);

    (capture.rawTrack.onended as unknown as () => void)();

    expect(onDeviceLost).toHaveBeenCalledTimes(1);
  });

  it("releases the device, the detector and the end handler", () => {
    const capture = fakeCapture();
    microphone.adopt(capture, DEFAULT_SETTINGS);

    microphone.release();

    expect(capture.dispose).toHaveBeenCalled();
    expect(capture.rawTrack.onended).toBeNull();
    expect(speech.remove).toHaveBeenCalledWith("u-a");
    expect(microphone.capture).toBeNull();
  });

  it("retunes the running capture without reopening it", () => {
    const capture = fakeCapture();
    microphone.adopt(capture, DEFAULT_SETTINGS);

    microphone.tune({ ...DEFAULT_SETTINGS, vadSensitivity: 80, inputGainDb: 6 });

    expect(capture.setSensitivity).toHaveBeenCalledWith(80);
    expect(capture.setInputGain).toHaveBeenCalledWith(6);
  });

  it("asks for a reopen only when a capture-affecting setting changed", () => {
    const settings: VoiceSettings = { ...DEFAULT_SETTINGS };
    expect(microphone.needsReopen(settings)).toBe(false);

    microphone.adopt(fakeCapture(), settings);

    expect(microphone.needsReopen({ ...settings, opusProfile: "studio" })).toBe(false);
    expect(microphone.needsReopen({ ...settings, inputDeviceId: "other" })).toBe(true);
  });

  it("is fine to resume when there is nothing to resume, or nothing processed", async () => {
    await expect(microphone.resume()).resolves.toBe(true);

    microphone.adopt(
      fakeCapture({ resume: vi.fn(async () => false), processed: false }),
      DEFAULT_SETTINGS,
    );
    await expect(microphone.resume()).resolves.toBe(true);
  });

  it("asks for a fresh capture when a processing graph stayed asleep", async () => {
    microphone.adopt(
      fakeCapture({ resume: vi.fn(async () => false), processed: true }),
      DEFAULT_SETTINGS,
    );

    await expect(microphone.resume()).resolves.toBe(false);
  });

  it("does nothing to replace a microphone that is not running", async () => {
    const superseded = vi.fn(() => false);

    const swapped = await microphone.replace(
      { ...DEFAULT_SETTINGS, inputDeviceId: "x" },
      true,
      superseded,
    );

    expect(swapped).toBe(false);
    expect(superseded).not.toHaveBeenCalled();
    expect(store.getSnapshot().error).toBeNull();
  });

  describe("replace", () => {
    const other: VoiceSettings = { ...DEFAULT_SETTINGS, inputDeviceId: "other" };

    function peerWithSender(): { sender: { replaceTrack: Mock } } {
      const sender = { replaceTrack: vi.fn(async () => undefined) };
      peers.set("u-z", { micSender: sender } as unknown as Link);
      return { sender };
    }

    it("moves the senders to the new capture and says it took over", async () => {
      const running = fakeCapture();
      const replacement = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      const { sender } = peerWithSender();
      vi.mocked(captureMicrophone).mockResolvedValue(replacement);

      const swapped = await microphone.replace(other, false, () => false);

      expect(swapped).toBe(true);
      expect(sender.replaceTrack).toHaveBeenCalledWith(replacement.track);
      expect(microphone.capture).toBe(replacement);
      expect(running.dispose).toHaveBeenCalled();
      expect(store.getSnapshot().error).toBeNull();
    });

    it("keeps the running capture when the setting does not affect it", async () => {
      const running = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);

      const swapped = await microphone.replace(
        { ...DEFAULT_SETTINGS, opusProfile: "studio" },
        false,
        () => false,
      );

      expect(swapped).toBe(false);
      expect(captureMicrophone).not.toHaveBeenCalled();
      expect(microphone.capture).toBe(running);
    });

    it("keeps the running capture when the new device cannot be opened", async () => {
      const running = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      vi.mocked(captureMicrophone).mockRejectedValue(new DOMException("gone", "NotFoundError"));

      const swapped = await microphone.replace(other, false, () => false);

      expect(swapped).toBe(false);
      expect(microphone.capture).toBe(running);
      expect(store.getSnapshot().error).toMatchObject({ recoverable: true });
    });

    it("drops a capture that a newer request made stale", async () => {
      const running = fakeCapture();
      const stale = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      const { sender } = peerWithSender();
      vi.mocked(captureMicrophone).mockResolvedValue(stale);

      const swapped = await microphone.replace(other, false, () => true);

      expect(swapped).toBe(false);
      expect(stale.dispose).toHaveBeenCalled();
      expect(sender.replaceTrack).not.toHaveBeenCalled();
      expect(microphone.capture).toBe(running);
    });

    it("keeps the running capture when the sender refuses the new one", async () => {
      const running = fakeCapture();
      const replacement = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      const { sender } = peerWithSender();
      sender.replaceTrack.mockRejectedValueOnce(new Error("sender closed"));
      vi.mocked(captureMicrophone).mockResolvedValue(replacement);

      const swapped = await microphone.replace(other, false, () => false);

      expect(swapped).toBe(false);
      expect(replacement.dispose).toHaveBeenCalled();
      expect(microphone.capture).toBe(running);
      expect(store.getSnapshot().error).toMatchObject({ kind: "mic-unavailable" });
    });

    it("drops a capture when the call was left while its senders were moving", async () => {
      const running = fakeCapture();
      const late = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      const { sender } = peerWithSender();
      vi.mocked(captureMicrophone).mockResolvedValue(late);
      // Leaving the call mid-move: the links go first, so the move succeeds
      // over an empty list while nobody is left to own the new capture.
      let left = false;
      sender.replaceTrack.mockImplementationOnce(async () => {
        peers.clear();
        microphone.release();
        left = true;
      });

      const swapped = await microphone.replace(other, false, () => left);

      expect(swapped).toBe(false);
      expect(late.dispose).toHaveBeenCalled();
      expect(sender.replaceTrack).toHaveBeenCalledOnce();
      expect(microphone.capture).toBeNull();
      expect(running.dispose).toHaveBeenCalled();
    });

    it("moves the senders back when a newer request waits after they were moved", async () => {
      const running = fakeCapture();
      const late = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      const { sender } = peerWithSender();
      vi.mocked(captureMicrophone).mockResolvedValue(late);
      // A newer settings change waits in the queue exactly as the move ends.
      // Its own reopen may yet fail, so the senders must not be left on a
      // capture that dies here: they go back to the one that survives.
      let newerWaiting = false;
      sender.replaceTrack.mockImplementationOnce(async () => {
        newerWaiting = true;
      });

      const swapped = await microphone.replace(other, false, () => newerWaiting);

      expect(swapped).toBe(false);
      expect(sender.replaceTrack).toHaveBeenNthCalledWith(1, late.track);
      expect(sender.replaceTrack).toHaveBeenNthCalledWith(2, running.track);
      expect(late.dispose).toHaveBeenCalled();
      expect(microphone.capture).toBe(running);
      expect(running.dispose).not.toHaveBeenCalled();
      expect(store.getSnapshot().error).toBeNull();
    });

    it("leaves the new call's microphone alone when the old swap finishes after rejoining", async () => {
      const running = fakeCapture();
      const late = fakeCapture();
      const rejoined = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      peerWithSender();
      vi.mocked(captureMicrophone).mockResolvedValueOnce(late);
      let finishEncoding!: () => void;
      let enteredEncoding!: () => void;
      const encoding = new Promise<void>((resolve) => { finishEncoding = resolve; });
      const encodingStarted = new Promise<void>((resolve) => { enteredEncoding = resolve; });
      applyEncoding.mockImplementationOnce(() => {
        enteredEncoding();
        return encoding;
      });
      let left = false;
      const swapping = microphone.replace(other, false, () => left);
      await encodingStarted;

      left = true;
      peers.clear();
      microphone.release();
      microphone.adopt(rejoined, other);
      const { sender } = peerWithSender();
      finishEncoding();

      expect(await swapping).toBe(false);
      expect(sender.replaceTrack).not.toHaveBeenCalled();
      expect(rebuildLink).not.toHaveBeenCalled();
      expect(microphone.capture).toBe(rejoined);
      expect(rejoined.dispose).not.toHaveBeenCalled();
      expect(late.dispose).toHaveBeenCalledOnce();
      expect(store.getSnapshot().error).toBeNull();
    });

    it("does not roll back links created after the swap began", async () => {
      const running = fakeCapture();
      const late = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      const original = peerWithSender();
      vi.mocked(captureMicrophone).mockResolvedValueOnce(late);
      let newerWaiting = false;
      const lateSender = { replaceTrack: vi.fn(async () => undefined) };
      original.sender.replaceTrack.mockImplementationOnce(async () => {
        peers.set("u-late", { micSender: lateSender } as unknown as Link);
        newerWaiting = true;
      });

      expect(await microphone.replace(other, false, () => newerWaiting)).toBe(false);

      expect(original.sender.replaceTrack).toHaveBeenNthCalledWith(2, running.track);
      expect(lateSender.replaceTrack).not.toHaveBeenCalled();
      expect(late.dispose).toHaveBeenCalledOnce();
    });

    it("stops a rollback if the call changes while restoring a sender", async () => {
      const running = fakeCapture();
      const late = fakeCapture();
      const rejoined = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      const { sender } = peerWithSender();
      const second = { replaceTrack: vi.fn(async () => undefined) };
      const secondLink = { micSender: second } as unknown as Link;
      peers.set("u-y", secondLink);
      vi.mocked(captureMicrophone).mockResolvedValueOnce(late);
      let newerWaiting = false;
      sender.replaceTrack
        .mockImplementationOnce(async () => { newerWaiting = true; })
        .mockImplementationOnce(async () => {
          peers.clear();
          microphone.release();
          microphone.adopt(rejoined, other);
          throw new Error("old connection closed");
        });

      expect(await microphone.replace(other, false, () => newerWaiting)).toBe(false);

      expect(second.replaceTrack).toHaveBeenCalledOnce();
      expect(rebuildLink).not.toHaveBeenCalled();
      expect(microphone.capture).toBe(rejoined);
      expect(rejoined.dispose).not.toHaveBeenCalled();
      expect(late.dispose).toHaveBeenCalledOnce();
    });

    it("rebuilds a current link that refuses the rollback after a partial swap", async () => {
      const running = fakeCapture();
      const late = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      const { sender } = peerWithSender();
      const movedLink = peers.get("u-z");
      sender.replaceTrack.mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("no rollback"));
      const refused = { replaceTrack: vi.fn(async () => { throw new Error("no swap"); }) };
      peers.set("u-y", { micSender: refused } as unknown as Link);
      vi.mocked(captureMicrophone).mockResolvedValueOnce(late);

      expect(await microphone.replace(other, false, () => false)).toBe(false);

      expect(sender.replaceTrack).toHaveBeenNthCalledWith(2, running.track);
      expect(refused.replaceTrack).toHaveBeenCalledOnce();
      expect(rebuildLink).toHaveBeenCalledExactlyOnceWith(movedLink);
      expect(microphone.capture).toBe(running);
      expect(running.dispose).not.toHaveBeenCalled();
      expect(late.dispose).toHaveBeenCalledOnce();
    });

    it.each(["opening", "moving"])("does not report a late %s failure in the new call", async (stage) => {
      const running = fakeCapture();
      const late = fakeCapture();
      const rejoined = fakeCapture();
      microphone.adopt(running, DEFAULT_SETTINGS);
      const { sender } = peerWithSender();
      let left = false;
      const failAfterRejoining = async () => {
        left = true;
        peers.clear();
        microphone.release();
        microphone.adopt(rejoined, other);
        throw new Error("old call ended");
      };
      if (stage === "opening") {
        vi.mocked(captureMicrophone).mockImplementationOnce(failAfterRejoining);
      } else {
        vi.mocked(captureMicrophone).mockResolvedValueOnce(late);
        sender.replaceTrack.mockImplementationOnce(failAfterRejoining);
      }

      expect(await microphone.replace(other, false, () => left)).toBe(false);

      expect(microphone.capture).toBe(rejoined);
      expect(rejoined.dispose).not.toHaveBeenCalled();
      expect(store.getSnapshot().error).toBeNull();
      expect(rebuildLink).not.toHaveBeenCalled();
      if (stage === "moving") expect(late.dispose).toHaveBeenCalledOnce();
    });
  });
});

describe("push to talk", () => {
  const PTT: VoiceSettings = { ...DEFAULT_SETTINGS, inputMode: "pushToTalk" };

  /** A capture with the gate in its path, whose holds are recorded. */
  function gated() {
    const holdGate = vi.fn<(open: boolean | null) => void>();
    const processed = { enabled: true } as MediaStreamTrack;
    const capture = fakeCapture({ gateActive: true, holdGate, track: processed });
    return { capture, holdGate, processed };
  }

  function sending(capture: MicCapture): boolean[] {
    return [capture.rawTrack.enabled, capture.track.enabled];
  }

  beforeEach(() => {
    vi.useFakeTimers();
    microphone.setInputMode("pushToTalk");
  });

  it("holds the gate shut until the key is down, and open for the tail after it comes up", () => {
    const { capture, holdGate } = gated();
    microphone.adopt(capture, PTT);
    expect(holdGate).toHaveBeenLastCalledWith(false);
    talkingChanged.mockClear();

    microphone.setTalking(true);
    expect(holdGate).toHaveBeenLastCalledWith(true);
    expect(microphone.talking).toBe(true);

    microphone.setTalking(false);
    vi.advanceTimersByTime(TALK_RELEASE_TAIL_MS - 1);
    expect(holdGate).toHaveBeenLastCalledWith(true);
    expect(microphone.talking).toBe(true);

    vi.advanceTimersByTime(1);
    expect(holdGate).toHaveBeenLastCalledWith(false);
    expect(microphone.talking).toBe(false);
    // The gate does the work: the tracks stay on, so nothing clicks.
    expect(sending(capture)).toEqual([true, true]);
    expect(talkingChanged).toHaveBeenCalledTimes(2);
  });

  it("carries on through a press within the tail, and closes at once when cut", () => {
    const { capture, holdGate } = gated();
    microphone.adopt(capture, PTT);
    microphone.setTalking(true);
    microphone.setTalking(false);
    vi.advanceTimersByTime(TALK_RELEASE_TAIL_MS / 2);
    microphone.setTalking(true);
    vi.advanceTimersByTime(TALK_RELEASE_TAIL_MS * 4);
    expect(microphone.talking).toBe(true);
    // Shut once, when it was adopted, and open ever since the first press.
    expect(holdGate.mock.calls.map(([open]) => open).lastIndexOf(false)).toBe(0);

    microphone.setTalking(false, true);
    expect(holdGate).toHaveBeenLastCalledWith(false);

    // A cut also ends a tail that is already running.
    microphone.setTalking(true);
    microphone.setTalking(false);
    microphone.setTalking(false, true);
    expect(microphone.talking).toBe(false);
    expect(holdGate).toHaveBeenLastCalledWith(false);
  });

  it("sends nothing while muted, key or not", () => {
    const { capture } = gated();
    store.set({ muted: true });
    microphone.adopt(capture, PTT);

    microphone.setTalking(true);
    expect(sending(capture)).toEqual([false, false]);

    microphone.setMuted(false);
    expect(sending(capture)).toEqual([true, true]);
  });

  it("switches the track itself where there is no gate to hold", () => {
    const processed = { enabled: true } as MediaStreamTrack;
    const holdGate = vi.fn();
    const capture = fakeCapture({ gateActive: false, holdGate, track: processed });
    microphone.adopt(capture, PTT);
    expect(sending(capture)).toEqual([false, false]);

    microphone.setTalking(true);
    expect(sending(capture)).toEqual([true, true]);

    microphone.setMuted(true);
    expect(sending(capture)).toEqual([false, false]);
    microphone.setMuted(false);

    microphone.setTalking(false);
    vi.advanceTimersByTime(TALK_RELEASE_TAIL_MS);
    expect(sending(capture)).toEqual([false, false]);
    expect(holdGate).not.toHaveBeenCalled();
  });

  it("lets the key go and hands the gate back when the mode returns to voice activity", () => {
    const { capture, holdGate } = gated();
    microphone.adopt(capture, PTT);
    microphone.setTalking(true);

    microphone.setInputMode("voiceActivity");

    expect(holdGate).toHaveBeenLastCalledWith(null);
    expect(microphone.talking).toBe(false);
    expect(sending(capture)).toEqual([true, true]);

    // A capture made for push to talk starts held shut; it is handed back too.
    const late = gated();
    microphone.adopt(late.capture, PTT);
    expect(late.holdGate).toHaveBeenCalledExactlyOnceWith(null);
  });

  it("lets the key go at the end of the call, with no tail left to run", () => {
    const { capture } = gated();
    microphone.adopt(capture, PTT);
    microphone.setTalking(true);
    microphone.setTalking(false);

    microphone.release();
    expect(microphone.talking).toBe(false);
    const reported = talkingChanged.mock.calls.length;
    vi.advanceTimersByTime(TALK_RELEASE_TAIL_MS * 2);
    expect(talkingChanged).toHaveBeenCalledTimes(reported);
  });

  it("leaves voice activity as it was: the key does nothing and the gate is never held", () => {
    microphone.setInputMode("voiceActivity");
    const { capture, holdGate } = gated();
    microphone.adopt(capture, DEFAULT_SETTINGS);

    microphone.setTalking(true);
    microphone.setMuted(true);
    microphone.setMuted(false);

    expect(holdGate).not.toHaveBeenCalled();
    expect(microphone.talking).toBe(false);
    expect(sending(capture)).toEqual([true, true]);
  });
});
