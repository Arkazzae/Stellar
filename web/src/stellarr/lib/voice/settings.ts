import type { MessageKey } from "@/i18n";
import { ChangeNotifier } from "@/lib/realtime/emitter";
import { usesWorkletSuppression } from "./audioProfiles";
import { AUTO_CEILING_PROFILE, OPUS_PROFILE_KBPS } from "./opusRates";
import {
  CAMERA_PROFILES,
  DEFAULT_CAMERA_QUALITY,
  type CameraQualityId,
} from "./video/camera";
import {
  DEFAULT_SCREEN_SHARE_QUALITY,
  SCREEN_SHARE_PROFILES,
  type ScreenShareQualityId,
} from "./video/screenShare";

/** The encoder is built from these, and the settings name them under each stop. */
export { AUTO_CEILING_PROFILE, OPUS_PROFILE_KBPS };

/**
 * Bump when the shape changes incompatibly. A stored object from another
 * version is discarded wholesale rather than migrated field by field: these are
 * a few preferences that are quick to set again, and a half-migrated audio
 * pipeline is a far worse outcome than a reset one.
 */
export const SETTINGS_VERSION = 1;

const STORAGE_PREFIX = "stellarr.voice.settings";

/**
 * Device-local settings still belong to one signed-in account, so a second
 * account in the same browser never inherits the previous person's
 * microphone, output and voice processing.
 */
export function storageKeyForUser(userId: string): string {
  return `${STORAGE_PREFIX}.user.${encodeURIComponent(userId)}`;
}

const OPUS_PROFILE_IDS = ["economy", "balanced", "clarity", "premium", "studio"] as const;
export type OpusProfileId = (typeof OPUS_PROFILE_IDS)[number];

/**
 * What the person chooses for their voice: one of the fixed profiles, or
 * `auto`, which starts at the ceiling of `AUTO_CEILING_PROFILE` and lets the
 * adaptive controller take it as low as the economy profile's floor.
 */
const OPUS_QUALITY_IDS = ["auto", ...OPUS_PROFILE_IDS] as const;
export type OpusQualityId = (typeof OPUS_QUALITY_IDS)[number];

/** The most a choice lets the encoder send, in kb/s; `auto` is bounded by its ceiling profile. */
export function opusCeilingKbps(quality: OpusQualityId): number {
  return OPUS_PROFILE_KBPS[quality === "auto" ? AUTO_CEILING_PROFILE : quality];
}

/**
 * `rnnoise`, `gtcrn` and `speex` each run locally as a WASM binary in an
 * AudioWorklet, served from /audio/ (see `capture/noiseSuppression.ts`);
 * `browser` is the platform's own suppressor; `off` is raw.
 */
const NOISE_MODES = ["browser", "rnnoise", "gtcrn", "speex", "off"] as const;
export type NoiseSuppressionMode = (typeof NOISE_MODES)[number];

const PLAYBACK_PROFILE_IDS = ["off", "natural", "studio", "broadcast", "custom"] as const;
export type PlaybackProfileId = (typeof PLAYBACK_PROFILE_IDS)[number];

export interface PlaybackTuning {
  /** Low shelf around 160 Hz, in dB. */
  warmthDb: number;
  /** Bell around 360 Hz, in dB. */
  mudDb: number;
  /** Bell around 3.1 kHz, in dB. */
  presenceDb: number;
  /** High shelf around 8 kHz, in dB. */
  airDb: number;
  /** 0 bypasses voice levelling, 100 is the strongest useful curve. */
  compression: number;
}

export const PLAYBACK_EQ_MIN_DB = -12;
export const PLAYBACK_EQ_MAX_DB = 12;
export const PLAYBACK_EQ_STEP_DB = 0.5;

/**
 * Manual microphone gain, applied before the suppressor and the gate.
 *
 * Asymmetric because the problem is: a microphone that is too hot clips at the
 * device and cutting here cannot restore it, while a quiet headset on a laptop
 * jack routinely needs 10-20 dB to reach the level everything downstream was
 * tuned for.
 */
export const INPUT_GAIN_MIN_DB = -12;
export const INPUT_GAIN_MAX_DB = 24;
export const INPUT_GAIN_STEP_DB = 1;

function preset(tuning: PlaybackTuning): PlaybackTuning {
  return Object.freeze(tuning);
}

export const PLAYBACK_TUNING_PRESETS: Readonly<
  Record<Exclude<PlaybackProfileId, "custom">, PlaybackTuning>
> = Object.freeze({
  off: preset({ warmthDb: 0, mudDb: 0, presenceDb: 0, airDb: 0, compression: 0 }),
  natural: preset({ warmthDb: 1, mudDb: -1.5, presenceDb: 1.5, airDb: 0.5, compression: 20 }),
  studio: preset({ warmthDb: 3.5, mudDb: -4.5, presenceDb: 5, airDb: 3, compression: 65 }),
  broadcast: preset({ warmthDb: 6, mudDb: -6, presenceDb: 4, airDb: 2, compression: 90 }),
});

export interface VoiceSettings {
  version: number;
  /** `null` means "whatever the system calls default". */
  inputDeviceId: string | null;
  /** `null` routes call audio through the operating system's default output. */
  outputDeviceId: string | null;
  /** Playback level for call audio and shared-screen audio, from 0 to 100. */
  outputVolume: number;
  /** Local playback level for soundboard clips, from 0 to 100. */
  soundboardVolume: number;
  /** Local-only EQ and dynamics applied to remote microphone streams. */
  playbackProfile: PlaybackProfileId;
  /** Effective values behind the EQ controls, including a custom curve. */
  playbackTuning: PlaybackTuning;
  /** `null` means the browser's default camera. */
  videoDeviceId: string | null;
  cameraQuality: CameraQualityId;
  opusProfile: OpusQualityId;
  screenShareQuality: ScreenShareQualityId;
  noiseSuppression: NoiseSuppressionMode;
  echoCancellation: boolean;
  /**
   * Automatic volume. Runs as our own leveller in the capture graph, which
   * learns only from voiced speech, so silence and chewing between sentences
   * are never turned up. The browser's AGC is used only when that graph cannot
   * be built.
   */
  autoGainControl: boolean;
  /** Fixed gain in dB before everything else, within the `INPUT_GAIN_*_DB` range. */
  inputGainDb: number;
  /** 0 = only loud speech counts, 100 = a whisper counts. */
  vadSensitivity: number;
  /**
   * Mute the outgoing microphone whenever the speech detector says nobody is
   * talking, which keeps chewing, breathing and a keyboard out of the call
   * between sentences. `vadSensitivity` decides how eagerly it opens.
   */
  inputGate: boolean;
}

export const DEFAULT_SETTINGS: VoiceSettings = Object.freeze({
  version: SETTINGS_VERSION,
  inputDeviceId: null,
  outputDeviceId: null,
  outputVolume: 100,
  soundboardVolume: 100,
  playbackProfile: "studio",
  playbackTuning: PLAYBACK_TUNING_PRESETS.studio,
  videoDeviceId: null,
  cameraQuality: DEFAULT_CAMERA_QUALITY,
  // The encoder follows the link from the ceiling down. A stored explicit
  // profile still wins.
  opusProfile: "auto",
  screenShareQuality: DEFAULT_SCREEN_SHARE_QUALITY,
  // The browser's own suppressor is always there and has nothing to download.
  // The three engines are opt-in, and their files are fetched only once one
  // is chosen, so no call waits on a download nobody asked for. A stored
  // explicit choice still wins.
  noiseSuppression: "browser",
  echoCancellation: true,
  autoGainControl: true,
  inputGainDb: 0,
  vadSensitivity: 60,
  inputGate: true,
});

/**
 * Catalog keys for what each choice is called and what it means. The hints
 * take `{kbps}`, the choice's ceiling from `opusCeilingKbps`.
 */
export const OPUS_PROFILE_LABEL: Record<OpusQualityId, MessageKey> = {
  auto: "voice.profile.auto",
  economy: "voice.profile.economy",
  balanced: "voice.profile.balanced",
  clarity: "voice.profile.clarity",
  premium: "voice.profile.premium",
  studio: "voice.profile.studio",
};

export const OPUS_PROFILE_HINT: Record<OpusQualityId, MessageKey> = {
  auto: "voice.profile.autoHint",
  economy: "voice.profile.economyHint",
  balanced: "voice.profile.balancedHint",
  clarity: "voice.profile.clarityHint",
  premium: "voice.profile.premiumHint",
  studio: "voice.profile.studioHint",
};

export const NOISE_MODE_LABEL: Record<NoiseSuppressionMode, MessageKey> = {
  browser: "voice.noise.browser",
  rnnoise: "voice.noise.rnnoise",
  gtcrn: "voice.noise.gtcrn",
  speex: "voice.noise.speex",
  off: "voice.noise.off",
};

export const NOISE_MODE_HINT: Record<NoiseSuppressionMode, MessageKey> = {
  browser: "voice.noise.browserHint",
  rnnoise: "voice.noise.rnnoiseHint",
  gtcrn: "voice.noise.gtcrnHint",
  speex: "voice.noise.speexHint",
  off: "voice.noise.offHint",
};

const SCREEN_QUALITY_IDS = Object.keys(SCREEN_SHARE_PROFILES) as ScreenShareQualityId[];
const CAMERA_QUALITY_IDS = Object.keys(CAMERA_PROFILES) as CameraQualityId[];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function oneOf<T extends string>(allowed: readonly T[], value: unknown, fallback: T): T {
  return allowed.includes(value as T) ? (value as T) : fallback;
}

function booleanOr(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function deviceIdOrNull(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function clampPercent(value: unknown, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(100, Math.max(0, Math.round(value)));
}

/** Clamps into [min, max] and snaps to the slider's step. */
function clampStep(
  value: unknown,
  min: number,
  max: number,
  step: number,
  fallback: number,
): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.round(Math.min(max, Math.max(min, value)) / step) * step;
}

export function clampInputGain(value: unknown): number {
  return clampStep(
    value,
    INPUT_GAIN_MIN_DB,
    INPUT_GAIN_MAX_DB,
    INPUT_GAIN_STEP_DB,
    DEFAULT_SETTINGS.inputGainDb,
  );
}

export function clampSensitivity(value: unknown): number {
  return clampPercent(value, DEFAULT_SETTINGS.vadSensitivity);
}

function parsePlaybackTuning(value: unknown): PlaybackTuning {
  const fallback = DEFAULT_SETTINGS.playbackTuning;
  const source = isRecord(value) ? value : {};
  const eq = (key: "warmthDb" | "mudDb" | "presenceDb" | "airDb") =>
    clampStep(
      source[key],
      PLAYBACK_EQ_MIN_DB,
      PLAYBACK_EQ_MAX_DB,
      PLAYBACK_EQ_STEP_DB,
      fallback[key],
    );
  return {
    warmthDb: eq("warmthDb"),
    mudDb: eq("mudDb"),
    presenceDb: eq("presenceDb"),
    airDb: eq("airDb"),
    compression: clampPercent(source["compression"], fallback.compression),
  };
}

/**
 * Reads whatever is in storage and returns something the audio pipeline can
 * safely act on.
 *
 * Every field is checked in isolation: one corrupt value costs its own default,
 * not the whole object. Anything from another version is dropped, see
 * `SETTINGS_VERSION`.
 */
export function parseVoiceSettings(value: unknown): VoiceSettings {
  if (!isRecord(value) || value["version"] !== SETTINGS_VERSION) {
    return { ...DEFAULT_SETTINGS };
  }

  const outputVolume = clampPercent(value["outputVolume"], DEFAULT_SETTINGS.outputVolume);
  const playbackProfile = oneOf(
    PLAYBACK_PROFILE_IDS,
    value["playbackProfile"],
    DEFAULT_SETTINGS.playbackProfile,
  );

  return {
    version: SETTINGS_VERSION,
    inputDeviceId: deviceIdOrNull(value["inputDeviceId"]),
    outputDeviceId: deviceIdOrNull(value["outputDeviceId"]),
    outputVolume,
    soundboardVolume: clampPercent(value["soundboardVolume"], DEFAULT_SETTINGS.soundboardVolume),
    playbackProfile,
    playbackTuning:
      playbackProfile === "custom"
        ? parsePlaybackTuning(value["playbackTuning"])
        : { ...PLAYBACK_TUNING_PRESETS[playbackProfile] },
    videoDeviceId: deviceIdOrNull(value["videoDeviceId"]),
    cameraQuality: oneOf(
      CAMERA_QUALITY_IDS,
      value["cameraQuality"],
      DEFAULT_SETTINGS.cameraQuality,
    ),
    opusProfile: oneOf(OPUS_QUALITY_IDS, value["opusProfile"], DEFAULT_SETTINGS.opusProfile),
    screenShareQuality: oneOf(
      SCREEN_QUALITY_IDS,
      value["screenShareQuality"],
      DEFAULT_SETTINGS.screenShareQuality,
    ),
    noiseSuppression: oneOf(
      NOISE_MODES,
      value["noiseSuppression"],
      DEFAULT_SETTINGS.noiseSuppression,
    ),
    echoCancellation: booleanOr(
      value["echoCancellation"],
      DEFAULT_SETTINGS.echoCancellation,
    ),
    autoGainControl: booleanOr(value["autoGainControl"], DEFAULT_SETTINGS.autoGainControl),
    inputGainDb: clampInputGain(value["inputGainDb"]),
    vadSensitivity: clampSensitivity(value["vadSensitivity"]),
    inputGate: booleanOr(value["inputGate"], DEFAULT_SETTINGS.inputGate),
  };
}

/**
 * True when these settings put a Web Audio graph between the device and the
 * peers: a suppressor worklet, the gate, the leveller, or a fixed gain.
 *
 * Without one the raw device track is sent as it is, so anything that needs a
 * node has to reopen the microphone to get one, see `needsRecapture`.
 */
export function wantsCaptureGraph(settings: VoiceSettings): boolean {
  return (
    usesWorkletSuppression(settings.noiseSuppression) ||
    settings.inputGate ||
    settings.autoGainControl ||
    settings.inputGainDb !== 0
  );
}

/** True when the two settings differ in a way that needs the mic re-captured. */
export function needsRecapture(a: VoiceSettings, b: VoiceSettings): boolean {
  return (
    a.inputDeviceId !== b.inputDeviceId ||
    a.noiseSuppression !== b.noiseSuppression ||
    a.echoCancellation !== b.echoCancellation ||
    a.autoGainControl !== b.autoGainControl ||
    // The gate is a node in the capture graph; adding or removing it rebuilds
    // that graph, and the recapture is what swaps the track.
    a.inputGate !== b.inputGate ||
    // A fixed gain is retuned live once its node exists. It forces a reopen only
    // when it is the sole reason a graph is needed at all.
    wantsCaptureGraph(a) !== wantsCaptureGraph(b)
  );
}

function sameSettings(a: VoiceSettings, b: VoiceSettings): boolean {
  return (Object.keys(a) as (keyof VoiceSettings)[]).every((key) =>
    key === "playbackTuning"
      ? sameTuning(a.playbackTuning, b.playbackTuning)
      : a[key] === b[key],
  );
}

function sameTuning(a: PlaybackTuning, b: PlaybackTuning): boolean {
  return (Object.keys(a) as (keyof PlaybackTuning)[]).every((key) => a[key] === b[key]);
}

/** `null` when the stored text is not JSON at all. */
function parseStored(raw: string): VoiceSettings | null {
  try {
    return parseVoiceSettings(JSON.parse(raw));
  } catch {
    return null;
  }
}

function safeStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    // Blocked by a cookie policy.
    return null;
  }
}

/**
 * Settings as external state, so `useSyncExternalStore` can drive the UI and
 * the voice client can subscribe to the same source without React in between.
 *
 * Nothing is read or written until `setOwner` names the signed-in account;
 * until then the settings are the defaults, kept in memory.
 *
 * Storage failures never break a call: unreadable storage starts from defaults
 * and a refused write leaves the change in effect for this session.
 */
export class VoiceSettingsStore {
  private settings: VoiceSettings = Object.freeze({ ...DEFAULT_SETTINGS });
  private readonly notifier = new ChangeNotifier();
  private storageKey: string | null = null;

  readonly subscribe = this.notifier.subscribe;
  readonly getSnapshot = (): VoiceSettings => this.settings;

  constructor(private readonly storage: Storage | null = safeStorage()) {}

  /** Select the account namespace once bootstrap knows who is signed in. */
  setOwner(userId: string): VoiceSettings {
    const key = storageKeyForUser(userId);
    if (key === this.storageKey) return this.settings;

    this.storageKey = key;
    const loaded = Object.freeze(this.read(key));
    // Two accounts with identical values keep the same snapshot object, so
    // useSyncExternalStore does not re-render for a namespace switch alone.
    if (sameSettings(this.settings, loaded)) return this.settings;
    this.settings = loaded;
    this.notifier.notify();
    return this.settings;
  }

  update(patch: Partial<Omit<VoiceSettings, "version">>): VoiceSettings {
    const next = parseVoiceSettings({ ...this.settings, ...patch });
    return sameSettings(this.settings, next) ? this.settings : this.replace(next);
  }

  /** Always persists and notifies, so a fresh account's key holds the defaults. */
  reset(): VoiceSettings {
    return this.replace({ ...DEFAULT_SETTINGS });
  }

  private replace(next: VoiceSettings): VoiceSettings {
    this.settings = Object.freeze(next);
    this.write(next);
    this.notifier.notify();
    return this.settings;
  }

  private read(key: string): VoiceSettings {
    try {
      const raw = this.storage?.getItem(key);
      return (raw ? parseStored(raw) : null) ?? { ...DEFAULT_SETTINGS };
    } catch {
      return { ...DEFAULT_SETTINGS };
    }
  }

  private write(settings: VoiceSettings): void {
    if (this.storageKey === null) return;
    try {
      this.storage?.setItem(this.storageKey, JSON.stringify(settings));
    } catch {
      // The change stays in effect for this session.
    }
  }
}
