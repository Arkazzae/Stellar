/**
 * What one analysis frame measures: its energy and its peak, both safe from
 * clipped and corrupt samples.
 *
 * Everything in `detector/` runs on the audio render thread as well as on the
 * main thread, so it allocates nothing per frame and never touches a timer.
 */

/** Smallest amplitude counted as signal: -180 dBFS, so no level is ever -Infinity. */
export const SILENCE_FLOOR = 1e-9;

export function clamp(value: number, min: number, max: number): number {
  return value < min ? min : value > max ? max : value;
}

/** Anything a frame can be read from. */
export type Samples = ArrayLike<number>;

/**
 * Mean-square energy of a frame, clipping- and NaN-safe.
 *
 * Samples are nominally -1..1 but a hot input or a badly behaved driver can
 * exceed that, and a torn buffer can contain NaN. Squaring either without
 * guarding poisons the running average permanently, so both are clamped away
 * before they reach the accumulator.
 */
export function frameMeanSquare(frame: Samples, length = frame.length): number {
  let sum = 0;
  for (let i = 0; i < length; i += 1) {
    let sample = frame[i] as number;
    if (!Number.isFinite(sample)) sample = 0;
    else if (sample > 1) sample = 1;
    else if (sample < -1) sample = -1;
    sum += sample * sample;
  }
  return length > 0 ? sum / length : 0;
}

/** Largest absolute sample in a frame, with the same corruption guards as the energy. */
export function framePeakAbs(frame: Samples, length = frame.length): number {
  let peak = 0;
  for (let i = 0; i < length; i += 1) {
    let sample = frame[i] as number;
    if (!Number.isFinite(sample)) sample = 0;
    const absolute = Math.min(1, Math.abs(sample));
    if (absolute > peak) peak = absolute;
  }
  return peak;
}

/** Full-scale decibels of a mean-square energy, floored so it never returns -Infinity. */
export function meanSquareToDbfs(meanSquare: number): number {
  return 10 * Math.log10(Math.max(meanSquare, SILENCE_FLOOR * SILENCE_FLOOR));
}
