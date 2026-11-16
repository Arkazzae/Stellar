/** Hand-made frames for the detector's tests. Deterministic, so a failure always repeats. */

export const SAMPLE_RATE = 48000;
/** One 10 ms analysis frame. */
export const FRAME = 480;

/** Pseudo-noise from a fixed LCG. */
export function noise(amplitude: number, length: number, seed = 1): Float32Array {
  const out = new Float32Array(length);
  let state = seed;
  for (let i = 0; i < length; i += 1) {
    state = (state * 1103515245 + 12345) % 2147483648;
    out[i] = ((state / 2147483648) * 2 - 1) * amplitude;
  }
  return out;
}

export function tone(amplitude: number, length: number, hz = 220): Float32Array {
  const out = new Float32Array(length);
  for (let i = 0; i < length; i += 1) {
    out[i] = Math.sin((2 * Math.PI * hz * i) / SAMPLE_RATE) * amplitude;
  }
  return out;
}

/** Frames cut from one continuous sine, so no phase jump at a frame edge. */
export function continuousTone(hz: number, amplitude: number, frames: number): Float32Array[] {
  const whole = tone(amplitude, FRAME * frames, hz);
  return Array.from({ length: frames }, (_, i) => whole.subarray(i * FRAME, (i + 1) * FRAME));
}

/** Short, decaying broadband impact like a key or a controller switch. */
export function clickFrame(amplitude = 0.8): Float32Array {
  const out = new Float32Array(FRAME);
  for (let i = 0; i < 72; i += 1) {
    out[i] = amplitude * Math.exp(-i / 18) * (i % 2 === 0 ? 1 : -0.72);
  }
  return out;
}
