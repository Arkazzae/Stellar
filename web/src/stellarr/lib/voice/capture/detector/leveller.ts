import { clamp } from "./frame";

export interface LevellerConfig {
  sampleRate: number;
  targetDb: number;
  maxBoostDb: number;
  maxCutDb: number;
  levelAlpha: number;
  riseDbPerSecond: number;
  fallDbPerSecond: number;
  peakCeiling: number;
  minVoicedFrames: number;
}

export const LEVELLER_DEFAULTS: Readonly<Omit<LevellerConfig, "sampleRate">> = Object.freeze({
  /** Frame RMS that voiced speech is steered towards, in dBFS. */
  targetDb: -20,
  /** Ceiling on the boost. A microphone 18 dB too quiet is a hardware problem. */
  maxBoostDb: 18,
  /** A loud talker is only pulled down gently; the listener's compressor does the rest. */
  maxCutDb: 6,
  /** How fast the speech-level estimate follows a voiced frame. ~200 ms of vowels. */
  levelAlpha: 0.05,
  /** Gain climbs slowly, so a pause can never become a swell of room noise... */
  riseDbPerSecond: 6,
  /** ...and falls quickly, so a sudden shout does not stay a shout. */
  fallDbPerSecond: 40,
  /** No boosted sample may pass this; the guard acts before the sample plays. */
  peakCeiling: 0.95,
  /** Vowel frames heard before the first move: no decision from one "hm". */
  minVoicedFrames: 10,
});

/** What the leveller learns from: one frame's verdict from the detector. */
export interface LevellerFrame {
  dbfs: number;
  voiced: boolean;
  peak?: number;
}

/**
 * Automatic volume that learns from vowels and nothing else.
 *
 * The platform AGC measures whatever is loud. Between sentences that is the
 * room, the keyboard, or the crisps, and it duly turns them up, which is
 * exactly the sound this whole pipeline exists to remove. This one takes its
 * level from frames the detector marked voiced, so it can only ever steer
 * the voice to the target; silence and chewing teach it nothing. The gain
 * ramps in the dB domain a few decibels a second, inaudibly, and a peak
 * guard keeps a boosted shout from clipping before the ramp could react.
 *
 * Frame decisions (`observe`) come from the detector's results; the gain is
 * applied per sample (`process`) after the gate, so a closed gate stays
 * closed and an opening one is levelled from its first sample.
 */
export class Leveller {
  readonly config: LevellerConfig;
  gain = 1;
  targetGain = 1;
  speechDb: number | null = null;
  voicedFrames = 0;
  enabled = true;
  private readonly riseRatio: number;
  private readonly fallRatio: number;
  /** The most the peak guard may cut: the cut limit, as a gain. */
  private readonly minGain: number;

  constructor(options: Partial<LevellerConfig> & { sampleRate: number }) {
    const config = { ...LEVELLER_DEFAULTS, ...options };
    const sampleRate = Number.isFinite(config.sampleRate) && config.sampleRate > 0 ? config.sampleRate : 48000;
    this.config = config;
    this.riseRatio = 10 ** (Math.max(0, config.riseDbPerSecond) / 20 / sampleRate);
    this.fallRatio = 10 ** (-Math.max(0, config.fallDbPerSecond) / 20 / sampleRate);
    this.minGain = 10 ** (-Math.max(0, config.maxCutDb) / 20);
    this.reset();
  }

  reset(): void {
    this.gain = 1;
    this.targetGain = 1;
    this.speechDb = null;
    this.voicedFrames = 0;
  }

  /** A disabled leveller ramps back to unity rather than snapping. */
  setEnabled(enabled: boolean): void {
    this.enabled = Boolean(enabled);
  }

  /** The gain the ramp is heading towards right now. */
  get target(): number {
    return this.enabled ? this.targetGain : 1;
  }

  /** Learn from one detector frame. */
  observe(result: LevellerFrame): void {
    const c = this.config;
    if (this.enabled && result.voiced) {
      this.voicedFrames += 1;
      this.speechDb =
        this.speechDb === null ? result.dbfs : this.speechDb + (result.dbfs - this.speechDb) * c.levelAlpha;
      if (this.voicedFrames >= c.minVoicedFrames) {
        const wantedDb = clamp(c.targetDb - this.speechDb, -c.maxCutDb, c.maxBoostDb);
        this.targetGain = 10 ** (wantedDb / 20);
      }
    }
    // The frame just analysed is still in the gate's delay line, so pulling
    // the gain down now lands before its loudest sample is played.
    const peak = result.peak;
    if (typeof peak === "number" && peak > 0) {
      const ceiling = Math.max(this.minGain, c.peakCeiling / peak);
      if (this.gain > ceiling) this.gain = ceiling;
    }
  }

  /** `output` may be the same array as `input`. */
  process(input: Float32Array, output: Float32Array, length = input.length): void {
    const target = this.target;
    const rise = this.riseRatio;
    const fall = this.fallRatio;
    let gain = this.gain;
    for (let i = 0; i < length; i += 1) {
      if (gain < target) {
        gain *= rise;
        if (gain > target) gain = target;
      } else if (gain > target) {
        gain *= fall;
        if (gain < target) gain = target;
      }
      output[i] = (input[i] as number) * gain;
    }
    this.gain = gain;
  }
}
