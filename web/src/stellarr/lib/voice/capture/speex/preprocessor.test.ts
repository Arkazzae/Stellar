/**
 * The real speex.wasm, the one the browser is served (web/public/audio/,
 * which the pretest hook assembles from the recipe's output), through the
 * wrapper the worklet uses. What it does to each fixture is held exactly by
 * the suppressor record in web/bench/voice/; these say what the record
 * cannot: that the output is noise suppression at all, and that a destroyed
 * state gives its memory back.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { SpeexPreprocessor, instantiateSpeex } from "./preprocessor";

const FRAME = 128;
const RATE = 48_000;

const binary = new WebAssembly.Module(readFileSync(join(process.cwd(), "public", "audio", "speex.wasm")));

/** Uniform noise from a fixed seed (mulberry32), so every run hears the same room. */
function noise(seconds: number, rms: number, seed = 7): Float32Array {
  let state = seed >>> 0;
  const out = new Float32Array(seconds * RATE);
  const scale = rms * Math.sqrt(3) * 2;
  for (let i = 0; i < out.length; i += 1) {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = Math.imul(state ^ (state >>> 15), state | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    out[i] = (((t ^ (t >>> 14)) >>> 0) / 4_294_967_296 - 0.5) * scale;
  }
  return out;
}

function rmsDb(samples: Float32Array, from: number, to: number): number {
  let sum = 0;
  for (let i = from; i < to; i += 1) sum += (samples[i] as number) ** 2;
  return 10 * Math.log10(sum / (to - from));
}

function run(preprocessor: SpeexPreprocessor, input: Float32Array): Float32Array {
  const output = new Float32Array(input.length);
  for (let at = 0; at < input.length; at += FRAME) {
    preprocessor.run(input.subarray(at, at + FRAME), output.subarray(at, at + FRAME));
  }
  return output;
}

describe("SpeexPreprocessor on speex.wasm", () => {
  it("turns steady noise down once it has learned it, and lets a tone well above it through", () => {
    // Three seconds of noise at -40 dBFS, then a 1 kHz tone 20 dB above it.
    const input = noise(4, 0.01);
    for (let i = 3 * RATE; i < input.length; i += 1) {
      input[i] = (input[i] as number) + 0.1 * Math.SQRT2 * Math.sin((2 * Math.PI * 1000 * i) / RATE);
    }
    const output = run(new SpeexPreprocessor(instantiateSpeex(binary), FRAME, RATE), input);

    const settledNoise = [2.5 * RATE, 3 * RATE] as const;
    const tone = [3.5 * RATE, 4 * RATE] as const;
    // About 13 dB down and 2 dB down: the estimator slowly takes a steady
    // tone for noise too.
    expect(rmsDb(output, ...settledNoise) - rmsDb(input, ...settledNoise)).toBeLessThan(-10);
    expect(rmsDb(output, ...tone) - rmsDb(input, ...tone)).toBeGreaterThan(-3);
  });

  it("keeps the peaks of a tone past full scale on their own side of zero", () => {
    // 440 Hz with peaks at 1.4: a sample past full scale stored as it was
    // wrapped to the other sign, and Speex cleaned the inverted peak.
    const input = new Float32Array(RATE);
    for (let i = 0; i < input.length; i += 1) input[i] = 1.4 * Math.sin((2 * Math.PI * 440 * i) / RATE);
    const output = run(new SpeexPreprocessor(instantiateSpeex(binary), FRAME, RATE), input);

    // The output follows the input one frame later.
    let peaks = 0;
    let flipped = 0;
    for (let i = RATE / 2; i < input.length - FRAME; i += 1) {
      const sample = input[i] as number;
      if (Math.abs(sample) <= 1.05) continue;
      peaks += 1;
      if (Math.sign(output[i + FRAME] as number) !== Math.sign(sample)) flipped += 1;
    }
    // Every one of the 11 025 flipped while the store wrapped.
    expect(peaks).toBeGreaterThan(1000);
    expect(flipped).toBe(0);
  });

  it("gives a destroyed state's memory back", () => {
    const speex = instantiateSpeex(binary);
    const frame = new Float32Array(FRAME);
    const cycle = () => {
      const preprocessor = new SpeexPreprocessor(speex, FRAME, RATE);
      preprocessor.run(frame, frame);
      return preprocessor;
    };
    cycle().destroy();
    const settled = speex.memory.buffer.byteLength;

    for (let i = 0; i < 2000; i += 1) cycle().destroy();
    expect(speex.memory.buffer.byteLength).toBe(settled);

    // The same states kept alive need room the memory did not have, so the
    // check above would have seen a state that was not freed.
    const alive = Array.from({ length: 200 }, cycle);
    expect(speex.memory.buffer.byteLength).toBeGreaterThan(settled);
    // The first of them was made before the memory grew, and still works.
    const output = new Float32Array(FRAME);
    (alive[0] as SpeexPreprocessor).run(noise(1, 0.01).subarray(0, FRAME), output);
    expect(output.every(Number.isFinite)).toBe(true);
    for (const preprocessor of alive) preprocessor.destroy();
  });
});
