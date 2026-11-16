/**
 * Small, deterministic signal helpers shared by the fixture generator, the
 * capture record and the playback page.
 *
 * Everything here is plain arithmetic on numbers and typed arrays. The random
 * source is an integer generator (mulberry32), so a seed gives the same
 * samples in Node and in Chrome, on any machine.
 */

export const SAMPLE_RATE = 48_000;
/** One Web Audio render quantum. */
export const QUANTUM = 128;
/** The 10 ms block the records are written in. */
export const BLOCK = 480;

export type Rng = () => number;

/** Uniform in [0, 1). */
export function mulberry32(seed: number): Rng {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4_294_967_296;
  };
}

const TRIANGULAR_SCALE = Math.sqrt(6);

/** Noise with unit variance and a triangular distribution: the sum of two uniforms. */
export function gauss(rng: Rng): number {
  return (rng() + rng() - 1) * TRIANGULAR_SCALE;
}

export function between(rng: Rng, low: number, high: number): number {
  return low + (high - low) * rng();
}

export function dbToGain(db: number): number {
  return 10 ** (db / 20);
}

export function gainToDb(gain: number): number {
  return 20 * Math.log10(gain);
}

export function rms(samples: ArrayLike<number>, from = 0, to = samples.length): number {
  let sum = 0;
  for (let i = from; i < to; i += 1) {
    const value = samples[i] ?? 0;
    sum += value * value;
  }
  return to > from ? Math.sqrt(sum / (to - from)) : 0;
}

export function peakAbs(samples: ArrayLike<number>, from = 0, to = samples.length): number {
  let peak = 0;
  for (let i = from; i < to; i += 1) {
    const value = Math.abs(samples[i] ?? 0);
    if (value > peak) peak = value;
  }
  return peak;
}

/** dBFS of an RMS or peak value, floored at -200 so it is always a number. */
export function dbfs(value: number): number {
  return value > 1e-10 ? 20 * Math.log10(value) : -200;
}

/**
 * RMS over the loudest stretch of a signal: the 10 ms blocks within 30 dB of
 * the loudest one. This is the level a listener would call "how loud the
 * voice is", without the pauses pulling it down.
 */
export function activeRms(samples: Float32Array): number {
  const blocks = Math.floor(samples.length / BLOCK);
  const energies: number[] = [];
  let loudest = 0;
  for (let b = 0; b < blocks; b += 1) {
    const value = rms(samples, b * BLOCK, (b + 1) * BLOCK);
    energies.push(value);
    if (value > loudest) loudest = value;
  }
  const floor = loudest * dbToGain(-30);
  let sum = 0;
  let count = 0;
  for (const value of energies) {
    if (value < floor) continue;
    sum += value * value;
    count += 1;
  }
  return count > 0 ? Math.sqrt(sum / count) : 0;
}

/** Scales `samples` in place so that its active RMS sits at `targetDb` dBFS. */
export function setActiveLevel(samples: Float32Array, targetDb: number): Float32Array {
  const current = activeRms(samples);
  if (current <= 0) return samples;
  const scale = dbToGain(targetDb) / current;
  for (let i = 0; i < samples.length; i += 1) samples[i] = (samples[i] ?? 0) * scale;
  return samples;
}

/** Scales `samples` in place so that its plain RMS sits at `targetDb` dBFS. */
export function setRmsLevel(samples: Float32Array, targetDb: number, from = 0, to = samples.length): Float32Array {
  const current = rms(samples, from, to);
  if (current <= 0) return samples;
  const scale = dbToGain(targetDb) / current;
  for (let i = 0; i < samples.length; i += 1) samples[i] = (samples[i] ?? 0) * scale;
  return samples;
}

/** Adds `source` into `target` starting at sample `at`, scaled by `gain`. */
export function mixInto(target: Float32Array, source: ArrayLike<number>, at = 0, gain = 1): void {
  const end = Math.min(target.length, at + source.length);
  for (let i = Math.max(0, at); i < end; i += 1) {
    target[i] = (target[i] ?? 0) + (source[i - at] ?? 0) * gain;
  }
}

/** Raised-cosine fade in over `[from, from + length)` and silence before it. */
export function fadeIn(samples: Float32Array, from: number, length: number): void {
  for (let i = 0; i < samples.length; i += 1) {
    if (i < from) samples[i] = 0;
    else if (i < from + length) {
      samples[i] = (samples[i] ?? 0) * 0.5 * (1 - Math.cos((Math.PI * (i - from)) / length));
    }
  }
}

/**
 * FNV-1a over the samples rounded to 24-bit integers. Two fixtures with the
 * same digest are the same audio for every purpose of this harness, and the
 * rounding keeps a last-bit difference in a transcendental function from
 * reading as a different fixture.
 */
export function digest(samples: Float32Array): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < samples.length; i += 1) {
    let value = Math.round((samples[i] ?? 0) * 8_388_608) | 0;
    for (let byte = 0; byte < 4; byte += 1) {
      hash ^= value & 0xff;
      hash = Math.imul(hash, 0x01000193) >>> 0;
      value >>>= 8;
    }
  }
  return hash.toString(16).padStart(8, "0");
}

/**
 * A two-pole resonator in the form Klatt used for formants: unity gain at
 * DC, a peak at `frequency` with bandwidth `bandwidth`. Coefficients can be
 * changed between samples without a click.
 */
export class Resonator {
  private a = 1;
  private b = 0;
  private c = 0;
  private y1 = 0;
  private y2 = 0;

  constructor(frequency = 500, bandwidth = 100, private readonly sampleRate = SAMPLE_RATE) {
    this.tune(frequency, bandwidth);
  }

  tune(frequency: number, bandwidth: number): void {
    const t = 1 / this.sampleRate;
    const r = Math.exp(-Math.PI * bandwidth * t);
    this.c = -(r * r);
    this.b = 2 * r * Math.cos(2 * Math.PI * frequency * t);
    this.a = 1 - this.b - this.c;
  }

  process(x: number): number {
    const y = this.a * x + this.b * this.y1 + this.c * this.y2;
    this.y2 = this.y1;
    this.y1 = y;
    return y;
  }
}

/** One-pole low-pass, `y += (x - y) * k`. */
export class OnePole {
  private y = 0;
  private readonly k: number;

  constructor(cutoffHz: number, sampleRate = SAMPLE_RATE) {
    this.k = 1 - Math.exp((-2 * Math.PI * cutoffHz) / sampleRate);
  }

  process(x: number): number {
    this.y += (x - this.y) * this.k;
    return this.y;
  }
}

/** First difference, a gentle +6 dB/octave tilt: x[n] - 0.95 x[n-1]. */
export class Tilt {
  private previous = 0;

  constructor(private readonly amount = 0.95) {}

  process(x: number): number {
    const y = x - this.amount * this.previous;
    this.previous = x;
    return y;
  }
}

/**
 * A finite measured value, rounded to `digits` decimals, with -0 written as
 * 0 so a record never differs by a sign that means nothing.
 */
export function fixed(value: number, digits: number): string {
  const text = value.toFixed(digits);
  return /^-0\.?0*$/.test(text) ? text.slice(1) : text;
}
