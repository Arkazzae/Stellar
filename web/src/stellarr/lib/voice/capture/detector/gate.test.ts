import { describe, expect, it } from "vitest";

import { GateSmoother } from "./gate";
import { SAMPLE_RATE } from "./testSignals";

function block(value: number, length = 128): Float32Array {
  return new Float32Array(length).fill(value);
}

describe("the gate", () => {
  it("starts closed and delays the signal by the lookahead", () => {
    const gate = new GateSmoother({ sampleRate: SAMPLE_RATE, lookaheadMs: 10 });
    expect(gate.delaySamples).toBe(480);
    expect(gate.gain).toBe(0);

    gate.setOpen(true);
    const out = new Float32Array(480);
    gate.process(block(1, 480), out);
    // Everything that came out was the zero-filled delay line, ramped or not.
    expect(Math.max(...out)).toBe(0);

    gate.process(block(1, 480), out);
    // Now the delayed ones arrive, on a ramp that has clearly opened.
    expect(out[0]).toBeGreaterThan(0.5);
    expect(out[479]).toBeGreaterThan(out[0] ?? 0);
    gate.process(block(1, 4800), new Float32Array(4800));
    expect(gate.gain).toBe(1);
  });

  it("ramps open within the attack time and closes within the release time", () => {
    const gate = new GateSmoother({ sampleRate: SAMPLE_RATE, lookaheadMs: 0, attackMs: 5, releaseMs: 50 });
    gate.setOpen(true);
    const out = new Float32Array(128);
    gate.process(block(1), out);
    // Not instantaneous, which would click, but clearly moving.
    expect(out[0]).toBeGreaterThan(0);
    expect(out[0]).toBeLessThan(0.1);
    gate.process(block(1, 2400), new Float32Array(2400));
    expect(gate.gain).toBe(1);

    gate.setOpen(false);
    gate.process(block(1), out);
    expect(out[127]).toBeLessThan(1);
    expect(out[127]).toBeGreaterThan(0.5);
    gate.process(block(1, 24_000), new Float32Array(24_000));
    expect(gate.gain).toBe(0);
  });

  it("passes everything when disabled, still delayed, and can run in place", () => {
    const gate = new GateSmoother({ sampleRate: SAMPLE_RATE, lookaheadMs: 1 });
    gate.setEnabled(false);
    gate.reset();
    expect(gate.gain).toBe(1);
    const out = new Float32Array(96);
    gate.process(block(0.5, 96), out);
    expect(out[47]).toBe(0);
    expect(out[48]).toBeCloseTo(0.5, 5);

    const direct = new GateSmoother({ sampleRate: SAMPLE_RATE, lookaheadMs: 0 });
    direct.setEnabled(false);
    direct.reset();
    const buffer = block(0.25);
    direct.process(buffer, buffer);
    expect(buffer[0]).toBeCloseTo(0.25, 5);
  });
});
