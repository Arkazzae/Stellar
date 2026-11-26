/**
 * Push to talk's hold on the gate, through the detector worklet itself: the
 * processor the app ships, built by the app's own code for the gate alone,
 * driven in render quanta with the hold delivered as the app posts it.
 */
import { describe, expect, it } from "vitest";

import { microphoneNode } from "../../../../../../bench/voice/capture/app";
import {
  registeredProcessor,
  runProcessor,
  type Delivery,
} from "../../../../../../bench/voice/capture/worklet";
import { QUANTUM, SAMPLE_RATE } from "../../../../../../bench/voice/dsp";
import { buildFixture, fixtureByName } from "../../../../../../bench/voice/fixtures";
import { GATE_DEFAULTS } from "./gate";

/** The gate's lookahead: what comes out is the input this many samples later. */
const DELAY = Math.round((GATE_DEFAULTS.lookaheadMs * SAMPLE_RATE) / 1000);
/** Past the attack, which snaps to fully open once inaudibly close. */
const OPENED = Math.round(SAMPLE_RATE * 0.2);
/** Past the release, which snaps to fully shut once inaudibly close. */
const CLOSED = Math.round(SAMPLE_RATE * 0.95);

async function gate(fixture: string, deliveries: Delivery[] = []) {
  const { Processor } = await registeredProcessor();
  const node = await microphoneNode(60, { gate: true, leveller: false });
  const input = buildFixture(fixtureByName(fixture));
  const { output } = runProcessor(Processor, node.processorOptions, input, deliveries);
  return { input, output };
}

function hold(atSample: number, open: boolean | null): Delivery {
  return { quantum: atSample / QUANTUM, message: { type: "hold", open } };
}

/** Half way into a fixture, on a render quantum. */
function halfway(name: string): number {
  const length = Math.round(fixtureByName(name).seconds * SAMPLE_RATE);
  return Math.floor(length / 2 / QUANTUM) * QUANTUM;
}

function peak(samples: Float32Array, from = 0, to = samples.length): number {
  let most = 0;
  for (let i = from; i < to; i += 1) most = Math.max(most, Math.abs(samples[i] as number));
  return most;
}

function energy(samples: Float32Array): number {
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  return sum;
}

describe("the gate held by push to talk", () => {
  it("lets out everything while held open, also what the detector keeps out", async () => {
    // Typing over a quiet room: the detector is built to stay shut for it.
    const detected = await gate("keyboard");
    const { input, output } = await gate("keyboard", [hold(0, true)]);

    expect(energy(detected.output)).toBeLessThan(energy(output) * 0.05);
    let worst = 0;
    for (let i = DELAY + OPENED; i < output.length; i += 1) {
      worst = Math.max(worst, Math.abs((output[i] as number) - (input[i - DELAY] as number)));
    }
    // Untouched, and only as late as the lookahead always makes it.
    expect(worst).toBe(0);
  });

  it("lets out nothing while held shut, speech included", async () => {
    const detected = await gate("speech");
    const { output } = await gate("speech", [hold(0, false)]);

    expect(peak(detected.output)).toBeGreaterThan(0.01);
    expect(peak(output)).toBe(0);
  });

  it("shuts completely once let go", async () => {
    const release = halfway("keyboard");
    const { output } = await gate("keyboard", [hold(0, true), hold(release, false)]);

    expect(peak(output, DELAY + OPENED, release)).toBeGreaterThan(0);
    expect(peak(output, release + DELAY + CLOSED, output.length)).toBe(0);
  });

  it("is the detector's again when handed back, exactly as if never held", async () => {
    const untouched = await gate("speech");
    const handedBack = await gate("speech", [hold(0, null)]);
    expect(handedBack.output).toEqual(untouched.output);

    const release = halfway("speech");
    const { output } = await gate("speech", [hold(0, false), hold(release, null)]);
    expect(peak(output, 0, release)).toBe(0);
    expect(peak(output, release, output.length)).toBeGreaterThan(0.01);
  });
});
