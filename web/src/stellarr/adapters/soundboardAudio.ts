import type { SoundboardPlayedEvent, SoundboardSound } from "@/lib/api/types";
import type { VoiceSettings } from "@/lib/voice/settings";

type LegacyWindow = Window & { webkitAudioContext?: typeof AudioContext };
type RoutableContext = AudioContext & { setSinkId?: (sinkId: string) => Promise<void> };

/** Plays soundboard clips on a Web Audio clock shared by every client. */
export interface SoundboardAudio {
  /**
   * Creates the context if needed and lets it start; call from a user
   * gesture. Having run once, the context can be woken later without one, so
   * it rests until a clip plays.
   */
  prepare(): void;
  /** Fetches and decodes clips ahead of time so a press starts without a delay. */
  preload(sounds: readonly SoundboardSound[]): void;
  setDeafened(deafened: boolean): void;
  /** A percentage from 0 to 100; anything else counts as full volume. */
  setVolume(volume: number): void;
  /**
   * Starts the clip at the moment the server named, translated to this
   * device's clock. A clip that is already under way joins part-way through.
   */
  schedule(event: SoundboardPlayedEvent, serverOffsetMs: number): Promise<void>;
  /**
   * Plays the clip here alone, from its first sample, as soon as the context
   * runs. No room shares its clock, so the time taken to wake is not counted
   * as lateness.
   */
  audition(sound: SoundboardSound): Promise<void>;
  /** Releases the context for good; later calls do nothing. */
  close(): void;
}

/**
 * How long the context keeps running after the last clip ended. The blocks
 * it rendered last are still on their way to the speaker, over Bluetooth for
 * a good part of this, and suspending stops the output under them.
 */
export const REST_AFTER_MS = 1000;

function finiteOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}

/**
 * The part of a clip worth playing, in seconds. Trim values come from the
 * server and are clamped to what the decoded audio really holds.
 */
function playableWindow(
  sound: SoundboardSound,
  duration: number,
): { start: number; end: number } {
  const start = Math.min(duration, Math.max(0, finiteOr(sound.trimStartSeconds, 0)));
  const end = Math.min(duration, Math.max(start, finiteOr(sound.trimEndSeconds, duration)));
  return { start, end };
}

export function createSoundboardAudio(
  getSettings: () => VoiceSettings,
): SoundboardAudio {
  const buffers = new Map<string, Promise<AudioBuffer>>();
  let context: AudioContext | null = null;
  let master: GainNode | null = null;
  let deafened = false;
  let volume = 100;
  let closed = false;
  /**
   * Clips between their schedule and their end. While there are none, the
   * context is suspended: a running one renders silence 375 times a second
   * and holds an output stream of its own open, for nothing.
   */
  let sounding = 0;
  let restTimer: ReturnType<typeof setTimeout> | null = null;

  const applyMasterGain = (): void => {
    if (master !== null) master.gain.value = deafened ? 0 : volume / 100;
  };

  const stopRestTimer = (): void => {
    if (restTimer !== null) clearTimeout(restTimer);
    restTimer = null;
  };

  /** Suspends the context a moment after the last clip, unless another starts. */
  const rest = (): void => {
    stopRestTimer();
    if (sounding > 0 || context === null) return;
    restTimer = setTimeout(() => {
      restTimer = null;
      if (sounding > 0 || context === null || context.state === "closed") return;
      void context.suspend().catch(() => undefined);
    }, REST_AFTER_MS);
  };

  const finish = (): void => {
    sounding = Math.max(0, sounding - 1);
    rest();
  };

  const ensureContext = (): AudioContext | null => {
    if (closed) return null;
    if (context !== null) return context;
    if (typeof window === "undefined") return null;
    const Constructor: typeof AudioContext | undefined =
      window.AudioContext ?? (window as LegacyWindow).webkitAudioContext;
    if (Constructor === undefined) return null;
    context = new Constructor();
    master = context.createGain();
    applyMasterGain();
    master.connect(context.destination);
    // A preload can open the context long before anything plays.
    rest();
    return context;
  };

  const routeOutput = async (target: AudioContext): Promise<void> => {
    const routable: RoutableContext = target;
    if (routable.setSinkId === undefined) return;
    // The empty id restores the system default. Skipping the call would leave
    // the context pinned to a headset that was selected earlier.
    await routable.setSinkId(getSettings().outputDeviceId ?? "").catch(() => undefined);
  };

  const decoded = (sound: SoundboardSound): Promise<AudioBuffer> => {
    const cached = buffers.get(sound.url);
    if (cached !== undefined) return cached;
    const pending = fetch(sound.url, { credentials: "same-origin" })
      .then((response) => {
        if (!response.ok) throw new Error(`Audio asset responded ${response.status}`);
        return response.arrayBuffer();
      })
      .then((bytes) => {
        // Looked up only now: a download that outlives close() must not bring
        // a context back to life.
        const target = ensureContext();
        if (target === null) throw new Error("Web Audio is unavailable");
        return target.decodeAudioData(bytes);
      });
    buffers.set(sound.url, pending);
    pending.catch(() => buffers.delete(sound.url));
    return pending;
  };

  /**
   * Wakes the context and starts `sound` at the moment `startAt` names, read
   * once the context runs. The clip counts as sounding from the call until it
   * ends, so the context cannot be put to rest under it.
   */
  const play = async (sound: SoundboardSound, startAt: () => number): Promise<void> => {
    const target = ensureContext();
    if (target === null) return;
    sounding += 1;
    stopRestTimer();
    let started = false;
    try {
      // Each await below may outlive the context, so each is followed by a check.
      const stale = (): boolean => closed || context !== target;
      await target.resume().catch(() => undefined);
      if (stale()) return;
      const targetClientMs = startAt();
      await routeOutput(target);
      if (stale()) return;
      const buffer = await decoded(sound);
      if (stale()) return;

      const now = Date.now();
      const { start, end } = playableWindow(sound, buffer.duration);
      const offset = Math.min(end, start + Math.max(0, (now - targetClientMs) / 1000));
      const remaining = end - offset;
      if (remaining <= 0) return;

      const source = target.createBufferSource();
      const gain = target.createGain();
      gain.gain.value = 1;
      source.buffer = buffer;
      source.connect(gain);
      gain.connect(master ?? target.destination);
      const release = (): void => {
        source.disconnect();
        gain.disconnect();
      };
      source.onended = () => {
        release();
        finish();
      };
      const wait = Math.max(0, (targetClientMs - now) / 1000);
      try {
        source.start(target.currentTime + wait, offset, remaining);
        started = true;
      } catch (cause) {
        source.onended = null;
        release();
        throw cause;
      }
    } finally {
      if (!started) finish();
    }
  };

  return {
    prepare() {
      const target = ensureContext();
      if (target === null) return;
      // Started inside the gesture, which is what lets a clip that arrives
      // later start without one; then it rests until a clip plays.
      void target.resume().then(rest, () => undefined);
      void routeOutput(target);
    },

    preload(sounds) {
      if (closed) return;
      for (const sound of sounds) decoded(sound).catch(() => undefined);
    },

    setDeafened(value) {
      deafened = value;
      applyMasterGain();
    },

    setVolume(value) {
      volume = Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : 100;
      applyMasterGain();
    },

    schedule(event, serverOffsetMs) {
      return play(event.sound, () => event.startsAtUnixMs - serverOffsetMs);
    },

    audition(sound) {
      return play(sound, Date.now);
    },

    close() {
      if (closed) return;
      closed = true;
      stopRestTimer();
      buffers.clear();
      const closing = context;
      context = null;
      master?.disconnect();
      master = null;
      if (closing !== null) void closing.close().catch(() => undefined);
    },
  };
}
