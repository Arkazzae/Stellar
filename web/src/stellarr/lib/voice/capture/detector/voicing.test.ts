import { describe, expect, it } from "vitest";

import { clickFrame, continuousTone, FRAME, noise, SAMPLE_RATE } from "./testSignals";
import { VAD_DEFAULTS } from "./vad";
import { VoicingDetector } from "./voicing";

function detector() {
  return new VoicingDetector({ sampleRate: SAMPLE_RATE });
}

describe("voicing", () => {
  it("searches an 8 kHz copy over the pitch range of a human voice", () => {
    const voicing = detector();
    expect(voicing.decimation).toBe(6);
    expect(voicing.lagMin).toBe(20); // 400 Hz
    expect(voicing.lagMax).toBe(133); // 60 Hz
  });

  it("hears a pitch in a sustained tone, forgets it on reset, and hears none in noise, a click or silence", () => {
    const voiced = detector();
    for (const frame of continuousTone(140, 0.3, 8)) voiced.push(frame);
    expect(voiced.peak()).toBeGreaterThan(0.9);
    voiced.reset();
    expect(voiced.peak()).toBe(0);

    const noisy = detector();
    let worst = 0;
    for (let i = 0; i < 40; i += 1) {
      noisy.push(noise(0.3, FRAME, i + 1));
      if (i > 5) worst = Math.max(worst, noisy.peak());
    }
    expect(worst).toBeLessThan(VAD_DEFAULTS.voicingOn);

    const clicky = detector();
    for (let i = 0; i < 8; i += 1) clicky.push(i % 2 === 0 ? clickFrame() : noise(0.01, FRAME, i));
    expect(clicky.peak()).toBeLessThan(VAD_DEFAULTS.voicingOn);

    const silent = detector();
    for (let i = 0; i < 8; i += 1) silent.push(new Float32Array(FRAME));
    expect(silent.peak()).toBe(0);
  });

  it("still hears the pitch through broadband noise at 0 dB SNR", () => {
    const voicing = detector();
    const frames = continuousTone(180, 0.3, 8);
    for (let i = 0; i < frames.length; i += 1) {
      const mixed = noise(0.3 * Math.SQRT1_2, FRAME, i + 11);
      const clean = frames[i] ?? new Float32Array(FRAME);
      for (let k = 0; k < FRAME; k += 1) mixed[k] = (mixed[k] ?? 0) + (clean[k] ?? 0);
      voicing.push(mixed);
    }
    expect(voicing.peak()).toBeGreaterThan(VAD_DEFAULTS.voicingOn);
  });
});
