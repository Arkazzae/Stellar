/**
 * The suppressor comparison's page. run.mjs builds it with Vite, opens it
 * in Chrome and calls `window.__suppressor`.
 *
 * "After" is the app as it is: `attachSuppressor` loads the engine's worklet
 * and binary from /audio/, exactly as a call does. "Before" is a worklet and
 * a binary the runner serves at /before/, started the way the app started
 * the published worklets: the processor name its registerProcessor call
 * gives, and `{ maxChannels: 1, wasmBinary }`. Both render the same fixture
 * in an OfflineAudioContext at 48 kHz, one mono channel.
 */
import {
  attachSuppressor,
  resetSuppressorCacheForTests,
  type NoiseSuppressorHandle,
} from "@/lib/voice/capture/noiseSuppression";
import type { WorkletSuppressor } from "@/lib/voice/audioProfiles";
import { measureLatency } from "../capture/record";
import { QUANTUM, SAMPLE_RATE } from "../dsp";
import { LATENCY_PROBE, buildFixture, fixtureByName } from "../fixtures";

declare global {
  interface Window {
    __suppressor?: unknown;
  }
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * The published worklets compile their binary asynchronously and output
 * silence until it is ready. A render that started before that would put
 * the two engines' states on different frames, so each render waits this
 * long after the nodes exist, and reports the first sample each one sounded
 * so that a start that was still too early shows.
 */
const SETTLE_MS = 500;

interface Before {
  processor: string;
  binary: ArrayBuffer;
}

let before: Before | null = null;

async function loadBefore(): Promise<Before> {
  if (before) return before;
  const code = await (await fetch("/before/worklet.js")).text();
  const processor = /registerProcessor\(\s*[`'"]([^`'"]+)[`'"]/.exec(code)?.[1];
  if (!processor) throw new Error("the worklet at /before/worklet.js registers no processor");
  const binary = await (await fetch("/before/binary.wasm")).arrayBuffer();
  before = { processor, binary };
  return before;
}

type Side = "before" | "after";

interface Rendered {
  output: Float32Array;
  firstSound: number;
}

function firstSound(samples: Float32Array): number {
  for (let i = 0; i < samples.length; i += 1) if (samples[i] !== 0) return i;
  return -1;
}

/** One side's node in `context`, fed by `source`. */
async function node(side: Side, kind: WorkletSuppressor, context: OfflineAudioContext, source: AudioNode) {
  if (side === "after") {
    const handle = await attachSuppressor(kind, context as unknown as AudioContext, source);
    if (!handle) throw new Error(`attachSuppressor gave no ${kind} suppressor`);
    return { output: handle.output as AudioWorkletNode, handle };
  }
  const { processor, binary } = await loadBefore();
  await context.audioWorklet.addModule("/before/worklet.js");
  const created = new AudioWorkletNode(context, processor, {
    processorOptions: { maxChannels: 1, wasmBinary: binary.slice(0) },
  });
  source.connect(created);
  return { output: created, handle: null as NoiseSuppressorHandle | null };
}

/**
 * `input` through one side. With `destroyAt`, the render stops there, the
 * node is sent the app's "destroy" without being disconnected, and goes on.
 */
async function render(side: Side, kind: WorkletSuppressor, input: Float32Array, destroyAt?: number): Promise<Rendered> {
  const context = new OfflineAudioContext(1, input.length, SAMPLE_RATE);
  const buffer = context.createBuffer(1, input.length, SAMPLE_RATE);
  buffer.getChannelData(0).set(input);
  const source = context.createBufferSource();
  source.buffer = buffer;
  const { output } = await node(side, kind, context, source);
  output.connect(context.destination);
  source.start();
  await sleep(SETTLE_MS);
  if (destroyAt !== undefined) {
    void context.suspend(destroyAt / SAMPLE_RATE).then(async () => {
      output.port.postMessage("destroy");
      await sleep(SETTLE_MS);
      await context.resume();
    });
  }
  const rendered = await context.startRendering();
  const samples = rendered.getChannelData(0).slice();
  return { output: samples, firstSound: firstSound(samples) };
}

async function latency(side: Side, kind: WorkletSuppressor): Promise<number | null> {
  const probe = LATENCY_PROBE.build();
  const { output } = await render(side, kind, probe);
  return measureLatency(probe, output, {
    start: LATENCY_PROBE.windowStart,
    length: LATENCY_PROBE.windowLength,
    maxDelay: LATENCY_PROBE.maxDelay,
  });
}

function energy(samples: Float32Array, from = 0, to = samples.length): number {
  let sum = 0;
  for (let i = from; i < to; i += 1) sum += (samples[i] as number) ** 2;
  return sum;
}

const db = (value: number) => (value > 0 ? 10 * Math.log10(value) : -Infinity);

export interface FixtureComparison {
  fixture: string;
  samples: number;
  firstSound: { before: number; after: number };
  /** Samples of the after side's output that differ from the before side's, aligned by latency. */
  differing: number;
  largest: number;
  /** RMS of the difference, in dBFS and in dB against the before side's output. */
  differenceDbfs: number;
  differenceDb: number;
  outputDbfs: { before: number; after: number };
}

/** Each fixture through both sides, compared sample by sample once aligned by latency. */
async function compare(kind: WorkletSuppressor, fixtures: string[]) {
  // The current side first: the runner serves the earlier binary for the
  // path the current side fetched.
  const after = await latency("after", kind);
  const latencies = { before: await latency("before", kind), after };
  const shift = (latencies.after ?? 0) - (latencies.before ?? 0);
  const results: FixtureComparison[] = [];
  for (const name of fixtures) {
    const input = buildFixture(fixtureByName(name));
    const a = await render("before", kind, input);
    const b = await render("after", kind, input);
    let differing = 0;
    let largest = 0;
    let sum = 0;
    const from = Math.max(0, -shift);
    const to = Math.min(a.output.length, b.output.length - shift);
    for (let i = from; i < to; i += 1) {
      const difference = Math.abs((b.output[i + shift] as number) - (a.output[i] as number));
      if (difference > 0) differing += 1;
      if (difference > largest) largest = difference;
      sum += difference * difference;
    }
    const count = Math.max(1, to - from);
    results.push({
      fixture: name,
      samples: input.length,
      firstSound: { before: a.firstSound, after: b.firstSound },
      differing,
      largest,
      differenceDbfs: db(sum / count),
      differenceDb: db(sum) - db(energy(a.output, from, to)),
      outputDbfs: { before: db(energy(a.output) / a.output.length), after: db(energy(b.output) / b.output.length) },
    });
  }
  return { latencies, results };
}

/**
 * Whether "destroy" silences each side: noise from the first sample, the
 * message half way through, and the output after it a quantum later.
 */
async function destroyed(kind: WorkletSuppressor) {
  const input = buildFixture(fixtureByName("fan"));
  const at = Math.floor(input.length / 2 / QUANTUM) * QUANTUM;
  const check = async (side: Side) => {
    const { output } = await render(side, kind, input, at);
    const after = output.subarray(at + QUANTUM);
    return { soundBefore: energy(output, 0, at) > 0, silentAfter: energy(after) === 0 };
  };
  return { before: await check("before"), after: await check("after") };
}

/** Whether `attachSuppressor` gives null when the server answers 404 for the binary. */
async function missingBinary(kind: WorkletSuppressor, path: string): Promise<boolean> {
  resetSuppressorCacheForTests();
  await fetch("/__missing", { method: "POST", body: path });
  try {
    const context = new OfflineAudioContext(1, QUANTUM, SAMPLE_RATE);
    const handle = await attachSuppressor(kind, context as unknown as AudioContext, context.createGain());
    return handle === null;
  } finally {
    await fetch("/__missing", { method: "DELETE" });
    resetSuppressorCacheForTests();
  }
}

window.__suppressor = { ready: true, compare, destroyed, missingBinary };
