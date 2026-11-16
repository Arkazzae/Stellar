import type { Samples } from "./frame";

export interface VoicingConfig {
  sampleRate: number;
  analysisRate: number;
  windowMs: number;
  minPitchHz: number;
  maxPitchHz: number;
  lowpassHz: number;
}

export const VOICING_DEFAULTS: Readonly<Omit<VoicingConfig, "sampleRate">> = Object.freeze({
  /**
   * Pitch lives below 400 Hz, so the search runs on an 8 kHz copy of the
   * input: six times fewer samples per lag, and the correlation of a 20 ms
   * window over 114 lags costs about 18 000 multiplies per 10 ms frame.
   */
  analysisRate: 8000,
  /**
   * 30 ms holds nearly two periods of the deepest voice and a dozen of the
   * highest. Longer windows push the chance peak of noise down (white noise
   * reaches 0.3, a ringing resonance about 0.55) while a vowel still
   * registers on its second frame, because the window is normalised and the
   * vowel dominates it as soon as it fills a third.
   */
  windowMs: 30,
  /** Range of the human voice; both children and bass singers fit. */
  minPitchHz: 60,
  maxPitchHz: 400,
  /** Anti-alias cut-off before decimation, kept clear of the analysis Nyquist. */
  lowpassHz: 3200,
});

/** Window energy under which periodicity is meaningless (about -95 dBFS). */
const VOICING_SILENCE = 1e-9;

/** Second-order section in transposed direct form II; allocates nothing per sample. */
class Biquad {
  private z1 = 0;
  private z2 = 0;

  constructor(
    private readonly b0: number,
    private readonly b1: number,
    private readonly b2: number,
    private readonly a1: number,
    private readonly a2: number,
  ) {}

  reset(): void {
    this.z1 = 0;
    this.z2 = 0;
  }

  process(x: number): number {
    const y = this.b0 * x + this.z1;
    this.z1 = this.b1 * x - this.a1 * y + this.z2;
    this.z2 = this.b2 * x - this.a2 * y;
    return y;
  }
}

/** RBJ cookbook low-pass, normalised so the section can be applied directly. */
function lowpassBiquad(sampleRate: number, cutoffHz: number, q: number): Biquad {
  const w0 = (2 * Math.PI * cutoffHz) / sampleRate;
  const cosW = Math.cos(w0);
  const alpha = Math.sin(w0) / (2 * q);
  const a0 = 1 + alpha;
  return new Biquad(
    (1 - cosW) / 2 / a0,
    (1 - cosW) / a0,
    (1 - cosW) / 2 / a0,
    (-2 * cosW) / a0,
    (1 - alpha) / a0,
  );
}

/**
 * How periodic the last 30 ms were, as the peak normalised autocorrelation
 * over the pitch range: 1 for a pure vowel or a tone, near 0 for noise.
 *
 * Chewing, breathing, a keyboard, a plastic bag: all of these are loud and
 * all of them last, so an energy detector opens for them. None of them repeat
 * themselves 60–400 times a second. A vowel does (that is what pitch is), so
 * a window that matches its own past one period ago is the one feature that
 * says "voice" and not merely "sound". Unvoiced consonants fail the test too,
 * which is fine: they never travel alone, and the detector's hangover carries
 * them once a vowel has opened it.
 *
 * `push` is called for every frame and does nothing but filter and decimate;
 * `peak` runs the correlation search only when the caller asks, which the
 * detector does solely for frames that already cleared the energy test.
 */
export class VoicingDetector {
  readonly decimation: number;
  readonly window: number;
  readonly lagMin: number;
  readonly lagMax: number;
  private readonly size: number;
  private readonly ring: Float32Array;
  private readonly linear: Float32Array;
  // Float64 for the running sums: a Float32 prefix over hundreds of squares
  // drifts enough to make a quiet window's energy go negative.
  private readonly prefix: Float64Array;
  private readonly stages: readonly [Biquad, Biquad];
  private position = 0;
  private filled = 0;
  private phase = 0;

  constructor(options: Partial<VoicingConfig> = {}) {
    const config = { ...VOICING_DEFAULTS, ...options };
    const sampleRate =
      typeof config.sampleRate === "number" && Number.isFinite(config.sampleRate) && config.sampleRate > 0
        ? config.sampleRate
        : 48000;
    this.decimation = Math.max(1, Math.round(sampleRate / config.analysisRate));
    const rate = sampleRate / this.decimation;
    this.window = Math.max(8, Math.round((rate * config.windowMs) / 1000));
    this.lagMin = Math.max(1, Math.round(rate / config.maxPitchHz));
    this.lagMax = Math.max(this.lagMin + 1, Math.round(rate / config.minPitchHz));
    this.size = this.window + this.lagMax;
    this.ring = new Float32Array(this.size);
    this.linear = new Float32Array(this.size);
    this.prefix = new Float64Array(this.size + 1);
    const cutoff = Math.min(config.lowpassHz, rate * 0.45);
    // Fourth-order Butterworth as two sections; 34 dB down at the analysis
    // Nyquist, which is plenty for a periodicity estimate.
    this.stages = [lowpassBiquad(sampleRate, cutoff, 0.5412), lowpassBiquad(sampleRate, cutoff, 1.3066)];
    this.reset();
  }

  reset(): void {
    this.ring.fill(0);
    this.position = 0;
    this.filled = 0;
    this.phase = 0;
    for (const stage of this.stages) stage.reset();
  }

  /**
   * Filter and decimate one chunk into the history. Cheap; call it for
   * every frame so the window is always current when `peak` is wanted.
   */
  push(frame: Samples, length = frame.length): void {
    const [first, second] = this.stages;
    for (let i = 0; i < length; i += 1) {
      let sample = frame[i] as number;
      if (!Number.isFinite(sample)) sample = 0;
      else if (sample > 1) sample = 1;
      else if (sample < -1) sample = -1;
      const filtered = second.process(first.process(sample));
      if (this.phase === 0) {
        this.ring[this.position] = filtered;
        this.position += 1;
        if (this.position === this.size) this.position = 0;
        if (this.filled < this.size) this.filled += 1;
      }
      this.phase += 1;
      if (this.phase === this.decimation) this.phase = 0;
    }
  }

  /**
   * Peak normalised autocorrelation of the newest window against its own
   * past, over the pitch lags, 0..1. 0 until enough history exists or when
   * the window is silent.
   */
  peak(): number {
    if (this.filled < this.size) return 0;
    const size = this.size;
    const x = this.linear;
    const ring = this.ring;
    // Unroll the ring oldest-first so every window is a contiguous slice.
    let position = this.position;
    for (let i = 0; i < size; i += 1) {
      x[i] = ring[position] as number;
      position += 1;
      if (position === size) position = 0;
    }
    const prefix = this.prefix;
    prefix[0] = 0;
    for (let i = 0; i < size; i += 1) prefix[i + 1] = (prefix[i] as number) + (x[i] as number) * (x[i] as number);

    const window = this.window;
    const start = size - window;
    const energy = (prefix[size] as number) - (prefix[start] as number);
    if (energy < VOICING_SILENCE) return 0;

    let best = 0;
    for (let lag = this.lagMin; lag <= this.lagMax; lag += 1) {
      const from = start - lag;
      const lagged = (prefix[size - lag] as number) - (prefix[from] as number);
      if (lagged < VOICING_SILENCE) continue;
      let dot = 0;
      for (let i = 0; i < window; i += 1) dot += (x[start + i] as number) * (x[from + i] as number);
      const r = dot / Math.sqrt(energy * lagged);
      if (r > best) best = r;
    }
    return best > 1 ? 1 : best;
  }
}
