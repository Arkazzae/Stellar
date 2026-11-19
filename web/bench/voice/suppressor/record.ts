/**
 * The suppressor record: what our noise suppressors' own binaries make of the
 * fixtures, checked against `records/suppressor/`, or written there when
 * STELLARR_VOICE_RECORD=1. Speex is the one engine so far.
 *
 * The binary is the one the browser loads, web/public/audio/speex.wasm, which
 * artifacts/assemble.mjs has checked against the manifest, driven in Node
 * through the worklet's own wrapper (`capture/speex/preprocessor.ts`) one
 * render quantum at a time, as the processor drives it. WebAssembly's
 * arithmetic is the same on every machine, so the output is held exactly, by
 * digest; the gains and the peak say what moved when it does not match.
 *
 * Each run is written as:
 *
 * | Line | What |
 * | --- | --- |
 * | `input` | digest of the int16 samples the binary is given, which is how far the fixture matters to it |
 * | `output` | digest of the int16 samples it gives back |
 * | `gain` | per 10 ms, RMS(out) / RMS(input delayed by the latency) in dB, as in the capture record |
 * | `peak` | the largest output sample in dBFS, and how many went past 0.95 and past 1 |
 *
 * The header holds the frame, the latency measured on the probe, and every
 * setting of the preprocessor as a new state reports it through
 * speex_preprocess_ctl, with denoise set on as the wrapper sets it.
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  SpeexPreprocessor,
  instantiateSpeex,
  type SpeexExports,
} from "../../../src/stellarr/lib/voice/capture/speex/preprocessor";
import { QUANTUM, SAMPLE_RATE, fixed } from "../dsp";
import { LATENCY_PROBE, buildFixture, fixtureByName } from "../fixtures";
import { blockGains, compareGains, encodeGains, formatPeak, formatRecord, measureLatency, parseRecord } from "../capture/record";

const here = dirname(fileURLToPath(import.meta.url));
export const RECORD_DIR = join(here, "..", "records", "suppressor");
const BINARY = join(here, "..", "..", "..", "public", "audio", "speex.wasm");

export const recording = process.env.STELLARR_VOICE_RECORD === "1";

/**
 * Steady noise alone and under speech, the quiet room, silence, impulses, a
 * pitched buzz, and samples past full scale.
 */
export const SPEEX_FIXTURES = [
  "speech",
  "speech-over-fan",
  "fan-speech",
  "fan",
  "keyboard",
  "buzz",
  "silence",
  "clipping",
] as const;

let compiled: WebAssembly.Module | null = null;

function speexModule(): WebAssembly.Module {
  compiled ??= new WebAssembly.Module(readFileSync(BINARY));
  return compiled;
}

/** `input` through a fresh preprocessor, a render quantum at a time, as the worklet runs it. */
export function runSpeex(input: Float32Array): Float32Array {
  const preprocessor = new SpeexPreprocessor(instantiateSpeex(speexModule()), QUANTUM, SAMPLE_RATE);
  const output = new Float32Array(input.length);
  for (let at = 0; at + QUANTUM <= input.length; at += QUANTUM) {
    preprocessor.run(input.subarray(at, at + QUANTUM), output.subarray(at, at + QUANTUM));
  }
  preprocessor.destroy();
  return output;
}

/** FNV-1a over int16 values: the samples' exact identity at the binary's boundary. */
function digestInt16(values: Int16Array): string {
  let hash = 0x811c9dc5;
  for (const value of values) {
    hash ^= value & 0xff;
    hash = Math.imul(hash, 0x01000193) >>> 0;
    hash ^= (value >> 8) & 0xff;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}

/** The samples the wrapper hands the binary: times 32767, stored as int16. */
function binaryInput(input: Float32Array): Int16Array {
  const values = new Int16Array(input.length);
  for (let i = 0; i < input.length; i += 1) values[i] = (input[i] as number) * 32767;
  return values;
}

/** The binary's own int16 output, back from the floats the wrapper made of it. */
function binaryOutput(output: Float32Array): Int16Array {
  const values = new Int16Array(output.length);
  for (let i = 0; i < output.length; i += 1) values[i] = Math.round((output[i] as number) * 32767);
  return values;
}

// From speex/speex_preprocess.h: the GET requests, the type each writes and how to show it.
const SETTINGS: ReadonlyArray<readonly [string, number, "int" | "float", string]> = [
  ["denoise", 1, "int", ""],
  ["noise_suppress", 19, "int", " dB"],
  ["agc", 3, "int", ""],
  ["agc_level", 7, "float", ""],
  ["agc_target", 47, "int", ""],
  ["agc_increment", 27, "int", " dB/s"],
  ["agc_decrement", 29, "int", " dB/s"],
  ["agc_max_gain", 31, "int", " dB"],
  ["vad", 5, "int", ""],
  ["prob_start", 15, "int", "%"],
  ["prob_continue", 17, "int", "%"],
  ["dereverb", 9, "int", ""],
  ["echo_suppress", 21, "int", " dB"],
  ["echo_suppress_active", 23, "int", " dB"],
];

/** Every setting a new state reports, with denoise set on as the wrapper sets it. */
function settings(): string {
  const speex: SpeexExports = instantiateSpeex(speexModule());
  const state = speex.speex_preprocess_state_init(QUANTUM, SAMPLE_RATE);
  const cell = speex.malloc(4);
  const ints = () => new Int32Array(speex.memory.buffer, cell, 1);
  const floats = () => new Float32Array(speex.memory.buffer, cell, 1);
  ints()[0] = 1;
  speex.speex_preprocess_ctl(state, 0, cell);
  const values = SETTINGS.map(([name, request, type, unit]) => {
    speex.speex_preprocess_ctl(state, request, cell);
    const value = type === "int" ? ints()[0] : floats()[0];
    return `${name} ${value}${unit}`;
  });
  speex.free(cell);
  speex.speex_preprocess_state_destroy(state);
  return values.join(", ");
}

export interface Outcome {
  name: string;
  problems: string[];
  /** True when this run wrote the record instead of checking it. */
  written: boolean;
}

const FILE = "speex.txt";

function readRecord(): string | null {
  try {
    return readFileSync(join(RECORD_DIR, FILE), "utf8");
  } catch {
    return null;
  }
}

function header(latency: number | null): Array<[string, string]> {
  return [
    ["binary", "web/public/audio/speex.wasm, through capture/speex/preprocessor.ts"],
    ["frame", `${QUANTUM} samples at ${SAMPLE_RATE} Hz (${fixed((QUANTUM * 1000) / SAMPLE_RATE, 2)} ms)`],
    ["latency", latency === null ? "none" : `${latency} (${fixed(latency / 48, 2)} ms)`],
    ["settings", settings()],
  ];
}

function measuredLatency(): number | null {
  const probe = LATENCY_PROBE.build();
  return measureLatency(probe, runSpeex(probe), {
    start: LATENCY_PROBE.windowStart,
    length: LATENCY_PROBE.windowLength,
    maxDelay: LATENCY_PROBE.maxDelay,
  });
}

function speexRun(name: string, latency: number): Array<[string, string]> {
  const input = buildFixture(fixtureByName(name));
  const output = runSpeex(input);
  return [
    ["input", digestInt16(binaryInput(input))],
    ["output", digestInt16(binaryOutput(output))],
    ["gain", encodeGains(blockGains(input, output, latency))],
    ["peak", formatPeak(output)],
  ];
}

/**
 * The whole record, checked or written. A run whose input digest moved is
 * reported as a changed fixture, not as changed sound.
 */
export function checkSpeex(): Outcome {
  const latency = measuredLatency();
  const runs = SPEEX_FIXTURES.map((name) => ({ label: name, lines: speexRun(name, latency ?? 0) }));
  const text = formatRecord(header(latency), runs, [
    "Suppressor record of Speex: what speex.wasm makes of each fixture. See web/bench/voice/README.md.",
    "Rewritten only by npm run voice:record, in a commit that names the audible change it intends.",
  ]);
  if (recording) {
    mkdirSync(RECORD_DIR, { recursive: true });
    writeFileSync(join(RECORD_DIR, FILE), text);
    return { name: FILE, problems: [], written: true };
  }
  const recordedText = readRecord();
  if (recordedText === null) return { name: FILE, problems: ["no record; run npm run voice:record"], written: false };

  const recorded = parseRecord(recordedText);
  const fresh = parseRecord(text);
  const problems: string[] = [];
  for (const [key, value] of fresh.header) {
    if (recorded.header.get(key) !== value) problems.push(`${key}: "${recorded.header.get(key)}" recorded, "${value}" now`);
  }
  const byLabel = new Map(recorded.runs.map((run) => [run.label, run]));
  for (const run of fresh.runs) {
    const before = byLabel.get(run.label);
    byLabel.delete(run.label);
    if (!before) {
      problems.push(`${run.label}: not recorded`);
      continue;
    }
    if (before.lines.get("input") !== run.lines.get("input")) {
      problems.push(`${run.label}: the fixture is not the audio it was recorded from`);
      continue;
    }
    if (before.lines.get("output") === run.lines.get("output")) continue;
    const gains = compareGains(before.lines.get("gain") ?? "", run.lines.get("gain") ?? "");
    const peak = before.lines.get("peak") === run.lines.get("peak") ? "" : `; peak "${before.lines.get("peak")}" recorded, "${run.lines.get("peak")}" now`;
    problems.push(`${run.label}: the output differs; ${gains ?? "every 10 ms block within 0.1 dB of the record"}${peak}`);
  }
  for (const label of byLabel.keys()) problems.push(`${label}: recorded, no longer run`);
  return { name: FILE, problems, written: false };
}

/** One line per problem, for an assertion message. */
export function explain(outcome: Outcome): string {
  return `${outcome.name}:\n  ${outcome.problems.join("\n  ")}`;
}
