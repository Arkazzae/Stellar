/**
 * Adaptive voice activity detection.
 *
 * Why not a fixed RMS threshold: a fixed threshold is wrong in every room. A
 * quiet room with a hot gain stage trips it constantly; a laptop fan or an open
 * window sits just under it and the indicator never fires. This tracks the noise
 * floor continuously and decides on *signal-to-noise ratio* instead, so the same
 * constants work in a silent bedroom and next to a fan.
 */
import { clamp, frameMeanSquare, framePeakAbs, meanSquareToDbfs, SILENCE_FLOOR, type Samples } from "./frame";
import { VoicingDetector } from "./voicing";

export interface VadConfig {
  /** Analysis frame length. 10 ms divides evenly into 48 kHz (480 samples). */
  frameMs: number;
  sampleRate: number;
  /** SNR needed to *start* speaking, in dB above the tracked noise floor. */
  snrOnDb: number;
  /** Lower bar to *keep* speaking: hysteresis, so syllables do not chop. */
  snrOffDb: number;
  /** Nothing below this is speech no matter how good the SNR looks. */
  absoluteFloorDb: number;
  attackFrames: number;
  transientRiseDb: number;
  transientCrestDb: number;
  transientBlockFrames: number;
  transientReleaseQuietFrames: number;
  hangoverFrames: number;
  /** Periodicity (0..1) a frame needs to open the detector; 0 disables the check. */
  voicingOn: number;
  voicedHoldFrames: number;
  maxUnvoicedFrames: number;
  noiseDownAlpha: number;
  noiseUpAlpha: number;
  noiseUpAlphaActive: number;
  noiseUpAlphaStationary: number;
  stationaryAfterFrames: number;
  stationaryDeltaDb: number;
  warmupFrames: number;
  noiseWarmupAlpha: number;
  noiseFloorInitDb: number;
  noiseFloorMinDb: number;
  trackingFloorDb: number;
  noiseFloorMaxDb: number;
  levelMinDb: number;
  levelMaxDb: number;
  levelAttack: number;
  levelRelease: number;
}

export const VAD_DEFAULTS: Readonly<VadConfig> = Object.freeze({
  frameMs: 10,
  sampleRate: 48000,
  snrOnDb: 7.5,
  snrOffDb: 3.5,
  absoluteFloorDb: -68,
  /** Three frames reject clicks and keyboard taps while keeping onset under 30 ms. */
  attackFrames: 3,
  /** A sudden rise this far above the previous frame may be a mechanical hit. */
  transientRiseDb: 9,
  /** Peak-to-RMS ratio that separates a short impact from sustained voice. */
  transientCrestDb: 10.5,
  /** Ignore this many onset frames after an impact; speech then opens normally. */
  transientBlockFrames: 5,
  /** A click after this much quiet closes a gate still held by hangover. */
  transientReleaseQuietFrames: 8,
  /** Frames of silence tolerated before release. 45 frames = 450 ms hangover. */
  hangoverFrames: 45,
  /**
   * Periodicity a frame needs before it may *open* the detector, 0..1.
   *
   * Energy and SNR cannot tell a vowel from a crunch: a mouthful of crisps at
   * a headset microphone is as loud as speech and lasts as long. What a vowel
   * has and a crunch has not is a pitch: the vocal folds repeat, so the
   * waveform correlates with itself one period ago. See `VoicingDetector`.
   * Unvoiced consonants (s, f, sz) have no pitch either, which is why this
   * gates only the *onset*: once open, the ordinary hangover carries the
   * detector across them to the next vowel. 0 disables the check.
   */
  voicingOn: 0.6,
  /** A voiced frame keeps counting towards an onset for this many frames. */
  voicedHoldFrames: 8,
  /**
   * Once open, close after this many frames without a voiced one, however
   * loud the input still is. Speech does not go 600 ms making sound without
   * a single vowel; chewing after the last word can go on for minutes, and
   * every crunch would otherwise renew the hangover.
   */
  maxUnvoicedFrames: 60,
  /** The floor follows a drop quickly and a rise slowly, so speech cannot raise it. */
  noiseDownAlpha: 0.35,
  noiseUpAlpha: 0.025,
  /**
   * Upward tracking while speech is held. Much slower than `noiseUpAlpha`,
   * so an utterance cannot drag the floor up behind itself, but a room that
   * genuinely gets louder mid-sentence still recovers instead of latching on
   * forever.
   */
  noiseUpAlphaActive: 0.00008,
  /**
   * A steady fan has low frame-to-frame modulation. If it has held the
   * detector open long enough, learn it quickly instead of showing a speaking
   * ring for minutes. Modulated speech keeps the conservative active alpha.
   */
  noiseUpAlphaStationary: 0.08,
  stationaryAfterFrames: 90,
  stationaryDeltaDb: 0.55,
  /**
   * Learn the room before judging it. The floor starts far below any real room,
   * so without this the first frame looks like a 25 dB SNR event, fires, and
   * then freezes the floor at its initial value: latched on for the whole call.
   * 12 frames is 120 ms: long enough for an initial estimate without making
   * the speaking indicator feel as though it missed the first word.
   */
  warmupFrames: 12,
  noiseWarmupAlpha: 0.45,
  noiseFloorInitDb: -70,
  noiseFloorMinDb: -90,
  /** Digital zero is ignored, but quiet real microphone noise still teaches the floor. */
  trackingFloorDb: -96,
  /**
   * Ceiling on the estimate. It has to sit above the noise of a genuinely loud
   * room (a fan or traffic reaches roughly -25 dBFS on a laptop mic) so the
   * detector can still release there, but below conversational speech (about
   * -20 to -10 dBFS) so a noisy room can never make it deaf to talking.
   */
  noiseFloorMaxDb: -20,
  /** dBFS window mapped onto the 0..1 level the UI meter draws. */
  levelMinDb: -60,
  levelMaxDb: -12,
  levelAttack: 0.6,
  levelRelease: 0.15,
});

export interface VadFrameResult {
  speaking: boolean;
  /** Smoothed 0..1 level for the meter. */
  level: number;
  dbfs: number;
  /** Largest absolute sample in the frame, 0..1. */
  peak: number;
  noiseFloorDb: number;
  snrDb: number;
  /** True for a short high-crest impact rejected as mechanical noise. */
  transient: boolean;
  /** Peak normalised autocorrelation over the pitch range, 0..1; 0 when not measured. */
  voicing: number;
  /** True when the frame cleared the energy test *and* had a pitch. */
  voiced: boolean;
}

/** The fields `configure` accepts; everything else is fixed at construction. */
const TUNABLE = [
  "snrOnDb",
  "snrOffDb",
  "absoluteFloorDb",
  "attackFrames",
  "hangoverFrames",
  "transientRiseDb",
  "transientCrestDb",
  "transientBlockFrames",
  "transientReleaseQuietFrames",
  "voicingOn",
  "voicedHoldFrames",
  "maxUnvoicedFrames",
] as const satisfies ReadonlyArray<keyof VadConfig>;

export class AdaptiveVad {
  config: VadConfig;
  readonly frameSize: number;
  speaking = false;
  level = 0;
  noiseFloorDb = 0;
  private readonly frame: Float32Array;
  private filled = 0;
  private readonly voicing: VoicingDetector;
  private attackCount = 0;
  private hangoverCount = 0;
  private transientBlockCount = 0;
  private quietFrames = 0;
  private framesSeen = 0;
  private previousDbfs = 0;
  private smoothedDb = 0;
  private modulationDb = 0;
  private stationaryFrames = 0;
  private voicedHold = 0;
  private framesSinceVoiced = 0;

  constructor(options: Partial<VadConfig> = {}) {
    this.config = { ...VAD_DEFAULTS, ...options };
    this.frameSize = Math.max(1, Math.round((this.config.sampleRate * this.config.frameMs) / 1000));
    this.frame = new Float32Array(this.frameSize);
    this.voicing = new VoicingDetector({ sampleRate: this.config.sampleRate });
    this.reset();
  }

  reset(): void {
    this.filled = 0;
    this.speaking = false;
    this.level = 0;
    this.attackCount = 0;
    this.hangoverCount = 0;
    this.transientBlockCount = 0;
    this.quietFrames = 0;
    this.framesSeen = 0;
    this.noiseFloorDb = this.config.noiseFloorInitDb;
    this.previousDbfs = this.config.noiseFloorInitDb;
    this.smoothedDb = this.config.noiseFloorInitDb;
    this.modulationDb = 0;
    this.stationaryFrames = 0;
    this.voicedHold = 0;
    this.framesSinceVoiced = 0;
    this.voicing.reset();
  }

  /**
   * Change thresholds without losing state.
   *
   * Only the fields present are touched, and each is validated here rather than
   * trusted from the message, because a NaN threshold would silently disable
   * detection for the rest of the call.
   */
  configure(patch: Partial<Record<keyof VadConfig, unknown>>): void {
    const next = { ...this.config };
    for (const key of TUNABLE) {
      const value = patch[key];
      if (typeof value === "number" && Number.isFinite(value)) next[key] = value;
    }
    // Keep the hysteresis ordering no matter what was asked for.
    if (next.snrOffDb >= next.snrOnDb) next.snrOffDb = next.snrOnDb - 1;
    next.attackFrames = Math.max(1, Math.round(next.attackFrames));
    next.hangoverFrames = Math.max(1, Math.round(next.hangoverFrames));
    next.transientBlockFrames = Math.max(0, Math.round(next.transientBlockFrames));
    next.transientReleaseQuietFrames = Math.max(0, Math.round(next.transientReleaseQuietFrames));
    next.voicingOn = clamp(next.voicingOn, 0, 1);
    next.voicedHoldFrames = Math.max(1, Math.round(next.voicedHoldFrames));
    next.maxUnvoicedFrames = Math.max(1, Math.round(next.maxUnvoicedFrames));
    this.config = next;
    // A tightened hangover must not leave a longer countdown running.
    if (this.hangoverCount > next.hangoverFrames) {
      this.hangoverCount = next.hangoverFrames;
    }
  }

  /** True while the floor is still converging and detection is suppressed. */
  get warmingUp(): boolean {
    return this.framesSeen < this.config.warmupFrames;
  }

  /**
   * Feed an arbitrary chunk (a 128-sample render quantum, say) and get back one
   * result per *completed* 10 ms frame: usually zero or one, never partial.
   * `out` is reused, so the hot path allocates no array.
   */
  push(chunk: Samples, out: VadFrameResult[] = []): VadFrameResult[] {
    out.length = 0;
    for (let i = 0; i < chunk.length; i += 1) {
      this.frame[this.filled] = chunk[i] as number;
      this.filled += 1;
      if (this.filled === this.frameSize) {
        this.filled = 0;
        out.push(this.processFrame(this.frame));
      }
    }
    return out;
  }

  /** Advance the state machine by exactly one frame. */
  processFrame(frame: Samples): VadFrameResult {
    const c = this.config;
    const meanSquare = frameMeanSquare(frame, this.frameSize);
    const dbfs = meanSquareToDbfs(meanSquare);
    const rms = Math.sqrt(Math.max(meanSquare, SILENCE_FLOOR * SILENCE_FLOOR));
    const peak = framePeakAbs(frame, this.frameSize);
    const crestDb = 20 * Math.log10(Math.max(peak / rms, 1));
    const warming = this.warmingUp;
    if (warming) this.framesSeen += 1;

    // Frames with no signal at all carry no information about the noise floor,
    // and letting them drag it down is actively harmful: a muted peer sends
    // digital silence, which would pull the floor to its minimum and make the
    // first frame after unmute look like a huge SNR event, a phantom
    // "speaking" flash every time someone unmutes.
    const informative = dbfs > c.trackingFloorDb;

    // A cheap modulation estimate separates steady machinery from speech. It
    // is deliberately used only to recover a detector that is already held
    // open (or held at the brink by continuous unvoiced energy); onset still
    // depends solely on level, SNR and voicing.
    const held = this.speaking || this.attackCount > 0;
    if (informative) {
      const delta = Math.abs(dbfs - this.smoothedDb);
      this.smoothedDb += (dbfs - this.smoothedDb) * 0.12;
      this.modulationDb += (delta - this.modulationDb) * 0.08;
      if (held && this.modulationDb <= c.stationaryDeltaDb) {
        this.stationaryFrames += 1;
      } else {
        this.stationaryFrames = 0;
      }
    } else {
      this.stationaryFrames = 0;
    }

    if (!informative) {
      // Leave the floor where it is.
    } else if (warming) {
      // Converge hard on whatever the room is actually doing.
      this.noiseFloorDb += (dbfs - this.noiseFloorDb) * c.noiseWarmupAlpha;
    } else if (dbfs < this.noiseFloorDb) {
      // Drops are tracked fast: the floor should sit on the quiet moments.
      this.noiseFloorDb += (dbfs - this.noiseFloorDb) * c.noiseDownAlpha;
    } else {
      // Rises are tracked slowly, and slower still while speech is held, so an
      // utterance cannot teach the detector to ignore itself.
      const alpha =
        this.stationaryFrames >= c.stationaryAfterFrames
          ? c.noiseUpAlphaStationary
          : held
            ? c.noiseUpAlphaActive
            : c.noiseUpAlpha;
      this.noiseFloorDb += (dbfs - this.noiseFloorDb) * alpha;
    }
    this.noiseFloorDb = clamp(this.noiseFloorDb, c.noiseFloorMinDb, c.noiseFloorMaxDb);

    const snrDb = dbfs - this.noiseFloorDb;
    const threshold = this.speaking ? c.snrOffDb : c.snrOnDb;
    const energyActive = !warming && dbfs > c.absoluteFloorDb && snrDb >= threshold;

    // Keyboard switches and controller buttons are impulsive: a large crest
    // arriving in one frame after a quieter frame. Energy/SNR alone cannot
    // distinguish a row of those impacts from speech. A voiced vowel or a
    // sustained consonant has a far lower peak-to-RMS ratio, so it proceeds
    // through the ordinary attack counter. The short block is deliberately
    // bounded; if an impact is immediately followed by real speech, the voice
    // still opens after at most 50 ms plus the normal attack.
    const transient =
      !warming &&
      dbfs > c.absoluteFloorDb &&
      dbfs - this.previousDbfs >= c.transientRiseDb &&
      crestDb >= c.transientCrestDb;
    const quietBeforeFrame = this.quietFrames;
    if (energyActive && !transient) this.quietFrames = 0;
    else this.quietFrames += 1;

    if (transient) {
      this.transientBlockCount = Math.max(this.transientBlockCount, c.transientBlockFrames);
      this.attackCount = 0;
      // Hangover intentionally keeps natural gaps between words open. Once a
      // gap is clearly established, however, a mechanical impact must not use
      // that old speech window to become audible.
      if (this.speaking && quietBeforeFrame >= c.transientReleaseQuietFrames) {
        this.speaking = false;
        this.hangoverCount = 0;
      }
    }

    const transientBlocked = this.transientBlockCount > 0;
    if (transientBlocked) this.transientBlockCount -= 1;
    const active = energyActive && !transientBlocked;
    this.previousDbfs = dbfs;

    // Periodicity is measured only for frames that already cleared the energy
    // test, or while the detector is open and needs to know whether the sound
    // it is passing still has a voice in it. The history is kept current for
    // every frame regardless, so the answer is available the moment it counts.
    this.voicing.push(frame, this.frameSize);
    const voicingWanted = c.voicingOn > 0 && (energyActive || this.speaking);
    const voicing = voicingWanted ? this.voicing.peak() : 0;
    const voiced = energyActive && !transient && voicing >= c.voicingOn;
    if (voiced) {
      this.voicedHold = c.voicedHoldFrames;
      this.framesSinceVoiced = 0;
    } else {
      if (this.voicedHold > 0) this.voicedHold -= 1;
      this.framesSinceVoiced += 1;
    }

    if (active) {
      this.hangoverCount = c.hangoverFrames;
      if (!this.speaking) {
        // Capped, so a long unvoiced run (a fan, a bag of crisps) waits at the
        // brink rather than counting up forever; the first vowel opens it.
        this.attackCount = Math.min(this.attackCount + 1, c.attackFrames);
        if (this.attackCount >= c.attackFrames && (c.voicingOn <= 0 || this.voicedHold > 0)) {
          this.speaking = true;
          this.attackCount = 0;
        }
      }
    } else {
      this.attackCount = 0;
      if (this.speaking) {
        this.hangoverCount -= 1;
        if (this.hangoverCount <= 0) this.speaking = false;
      }
    }
    // Loud but voiceless for too long: the sentence has ended and something
    // else is being fed to the microphone. Close, whatever the hangover says.
    if (this.speaking && c.voicingOn > 0 && this.framesSinceVoiced > c.maxUnvoicedFrames) {
      this.speaking = false;
      this.hangoverCount = 0;
    }

    const target = clamp((dbfs - c.levelMinDb) / (c.levelMaxDb - c.levelMinDb), 0, 1);
    const alpha = target > this.level ? c.levelAttack : c.levelRelease;
    this.level += (target - this.level) * alpha;

    return {
      speaking: this.speaking,
      level: this.level,
      dbfs,
      peak,
      noiseFloorDb: this.noiseFloorDb,
      snrDb,
      transient,
      voicing,
      voiced,
    };
  }
}

export interface VadThresholds {
  snrOnDb: number;
  snrOffDb: number;
  hangoverFrames: number;
  absoluteFloorDb: number;
  voicingOn: number;
}

function round1(value: number): number {
  return Math.round(value * 10) / 10;
}

function round2(value: number): number {
  return Math.round(value * 100) / 100;
}

/**
 * Maps a 0..100 sensitivity dial onto the detector's thresholds.
 *
 * Sensitivity is the only detector control worth exposing: everything else is
 * a consequence of it. 0 means "only clear speech above the room", 100 means
 * "a whisper counts". The on-threshold sweeps 14 dB down to 3 dB above the
 * tracked noise floor; the off-threshold trails it so hysteresis survives at
 * both ends, and the hangover grows slightly at high sensitivity so a quiet
 * talker is not chopped between words.
 */
export function sensitivityToThresholds(sensitivity: number): VadThresholds {
  const value = Number.isFinite(sensitivity) ? clamp(sensitivity, 0, 100) : 50;
  const t = value / 100;
  const snrOnDb = round1(14 - t * 11);
  return {
    snrOnDb,
    // Always meaningfully below the on-threshold, never below ~1.5 dB where
    // noise alone would hold the gate open.
    snrOffDb: round1(Math.max(1.5, snrOnDb - (3 + t * 2))),
    hangoverFrames: Math.round(25 + t * 20),
    // SNR alone is blind after a neural suppressor: RNNoise leaves near-digital
    // silence between words, so a -45 dBFS chewing residue over a -85 dBFS
    // floor is a 40 dB "event". The absolute floor gives the dial a second
    // meaning: at 0 only genuinely loud speech counts (-45 dBFS), at 100 the
    // original -68 dBFS whisper threshold applies.
    absoluteFloorDb: round1(-45 - t * 23),
    // Periodicity is required for an onset at every ordinary setting: it is
    // what keeps a crunch, a breath or a keyboard from opening the gate at
    // *any* loudness. Only the top of the dial, where the person has asked
    // for whispers (which have no pitch), lets the requirement go.
    voicingOn: Math.max(0, round2(t <= 0.7 ? 0.6 : 0.6 * (1 - (t - 0.7) / 0.3))),
  };
}
