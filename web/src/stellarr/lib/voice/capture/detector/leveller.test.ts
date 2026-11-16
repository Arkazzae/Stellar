import { describe, expect, it } from "vitest";

import { Leveller, LEVELLER_DEFAULTS, type LevellerFrame } from "./leveller";
import { SAMPLE_RATE } from "./testSignals";

const quietVowel = { dbfs: -40, voiced: true, peak: 0.02 };
const loudVowel = { dbfs: -5, voiced: true, peak: 0.8 };
const roomNoise = { dbfs: -12, voiced: false, peak: 0.4 };

function heard(leveller: Leveller, frame: LevellerFrame, frames: number) {
  for (let i = 0; i < frames; i += 1) leveller.observe(frame);
}

function targetDb(leveller: Leveller): number {
  return 20 * Math.log10(leveller.target);
}

describe("the leveller", () => {
  it("waits for a run of vowels, then steers towards the target within its limits", () => {
    const quiet = new Leveller({ sampleRate: SAMPLE_RATE });
    heard(quiet, quietVowel, LEVELLER_DEFAULTS.minVoicedFrames - 1);
    expect(quiet.target).toBe(1);
    heard(quiet, quietVowel, 1);
    // -40 dBFS wants +20 dB to reach -20; the limit is +18.
    expect(targetDb(quiet)).toBeCloseTo(LEVELLER_DEFAULTS.maxBoostDb, 5);

    const loud = new Leveller({ sampleRate: SAMPLE_RATE });
    heard(loud, loudVowel, 40);
    expect(targetDb(loud)).toBeCloseTo(-LEVELLER_DEFAULTS.maxCutDb, 5);
  });

  it("learns nothing from anything that is not a vowel", () => {
    const leveller = new Leveller({ sampleRate: SAMPLE_RATE });
    heard(leveller, roomNoise, 200);
    expect(leveller.target).toBe(1);
    expect(leveller.speechDb).toBeNull();

    // Nor does a chew between two sentences move a level already learned.
    heard(leveller, quietVowel, 40);
    const learned = leveller.target;
    heard(leveller, roomNoise, 200);
    expect(leveller.target).toBe(learned);
  });

  it("ramps a few decibels a second rather than jumping", () => {
    const leveller = new Leveller({ sampleRate: SAMPLE_RATE });
    heard(leveller, quietVowel, 40);
    const input = new Float32Array(SAMPLE_RATE).fill(0.1); // one second
    const output = new Float32Array(SAMPLE_RATE);
    leveller.process(input, output);
    const gainDb = 20 * Math.log10(leveller.gain);
    expect(gainDb).toBeGreaterThan(LEVELLER_DEFAULTS.riseDbPerSecond - 0.5);
    expect(gainDb).toBeLessThan(LEVELLER_DEFAULTS.riseDbPerSecond + 0.5);
    // The first sample is barely touched; the last carries the whole ramp.
    expect(output[0]).toBeCloseTo(0.1, 3);
    expect(output[SAMPLE_RATE - 1]).toBeCloseTo(0.1 * leveller.gain, 5);
  });

  it("pulls the gain under a peak at once, before the ramp could react", () => {
    const leveller = new Leveller({ sampleRate: SAMPLE_RATE });
    heard(leveller, quietVowel, 40);
    leveller.gain = 4; // +12 dB already applied
    leveller.observe({ dbfs: -8, voiced: false, peak: 0.5 });
    expect(leveller.gain * 0.5).toBeLessThanOrEqual(LEVELLER_DEFAULTS.peakCeiling);
  });

  it("returns to unity when disabled, and resets cleanly", () => {
    const leveller = new Leveller({ sampleRate: SAMPLE_RATE });
    heard(leveller, quietVowel, 40);
    expect(leveller.target).toBeGreaterThan(1);
    leveller.setEnabled(false);
    expect(leveller.target).toBe(1);
    heard(leveller, quietVowel, 40);
    expect(leveller.target).toBe(1);
    leveller.reset();
    expect(leveller.gain).toBe(1);
    expect(leveller.speechDb).toBeNull();
  });
});
