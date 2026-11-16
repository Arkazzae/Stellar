/**
 * The capture record: every fixture through every detector configuration the
 * app builds, checked against `records/capture/`, or written there when
 * STELLARR_VOICE_RECORD=1.
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { activeRms, dbfs, digest, fixed, peakAbs } from "../dsp";
import { LATENCY_PROBE, buildFixture, fixtureByName } from "../fixtures";
import {
  REMOTE_GATE_SENSITIVITY,
  meterNode,
  microphoneNode,
  remoteGateNode,
  remoteMeterNode,
  type AppDetector,
} from "./app";
import {
  compareRun,
  describeRun,
  formatRecord,
  measureLatency,
  parseRecord,
  type RecordRun,
} from "./record";
import { registeredProcessor, runProcessor, type Delivery } from "./worklet";

export const RECORD_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "records", "capture");

export const recording = process.env.STELLARR_VOICE_RECORD === "1";

/**
 * The dial positions recorded with every combination of gate and leveller:
 * the strict end, the default and the top, where pitch is no longer needed.
 */
export const SENSITIVITIES = [0, 60, 100] as const;

export interface ConfigSpec {
  label: string;
  build(): Promise<AppDetector>;
}

/**
 * Off/off is the meter: with neither stage the app builds no node in the
 * send path, and the only detector left is the one behind the meter and the
 * speaking ring. The other three are the node `buildCaptureGraph` puts
 * between the microphone and the peers.
 */
export const CONFIGS: readonly ConfigSpec[] = [
  ...SENSITIVITIES.flatMap((s): ConfigSpec[] => [
    { label: `s${s} meter`, build: () => meterNode(s) },
    { label: `s${s} gate`, build: () => microphoneNode(s, { gate: true, leveller: false }) },
    { label: `s${s} level`, build: () => microphoneNode(s, { gate: false, leveller: true }) },
    { label: `s${s} gate+level`, build: () => microphoneNode(s, { gate: true, leveller: true }) },
  ]),
  // 85 is the one place on the dial where the voicing requirement is
  // partial (0.3); the decisions are enough there.
  { label: "s85 meter", build: () => meterNode(85) },
  { label: "remote meter", build: remoteMeterNode },
  { label: `remote gate s${REMOTE_GATE_SENSITIVITY}`, build: remoteGateNode },
];

/** Things the app does to a running detector, at given times, on one fixture. */
interface Scenario {
  fixture: string;
  label: string;
  configs: string[];
  actions: Array<{ atMs: number; act(detector: AppDetector): void }>;
}

export const SCENARIOS: readonly Scenario[] = [
  {
    fixture: "speech",
    label: "dial 60>100@1400>0@3000, hang up@5600",
    configs: ["s60 meter", "s60 gate+level"],
    actions: [
      { atMs: 1400, act: (detector) => detector.setSensitivity?.(100) },
      { atMs: 3000, act: (detector) => detector.setSensitivity?.(0) },
      { atMs: 5600, act: (detector) => detector.dispose() },
    ],
  },
];

function quantumAt(ms: number): number {
  return Math.round((ms * 48) / 128);
}

/** Runs the app's actions now and schedules what they posted for their time. */
function scheduleActions(detector: AppDetector, actions: Scenario["actions"]): Delivery[] {
  const deliveries: Delivery[] = [];
  for (const { atMs, act } of actions) {
    const before = detector.inbox.length;
    act(detector);
    for (const message of detector.inbox.slice(before)) {
      deliveries.push({ quantum: quantumAt(atMs), message });
    }
  }
  return deliveries;
}

export function configByLabel(label: string): ConfigSpec {
  const config = CONFIGS.find((item) => item.label === label);
  if (!config) throw new Error(`no configuration called ${label}`);
  return config;
}

/** One configuration, built by the app, over `input`, with the app's actions at their times. */
export async function run(config: ConfigSpec, input: Float32Array, actions: Scenario["actions"] = []) {
  const { Processor } = await registeredProcessor();
  const detector = await config.build();
  // Anything posted while the node was being built arrives before the first quantum.
  const deliveries: Delivery[] = detector.inbox.map((message) => ({ quantum: 0, message }));
  deliveries.push(...scheduleActions(detector, actions));
  return runProcessor(Processor, detector.processorOptions, input, deliveries);
}

// ---- latency ----

let measured: Promise<Map<string, number | null>> | null = null;

/** Each configuration's latency, measured once per test file. */
export function measureLatencies(): Promise<Map<string, number | null>> {
  measured ??= measureAll();
  return measured;
}

async function measureAll(): Promise<Map<string, number | null>> {
  const probe = LATENCY_PROBE.build();
  const latencies = new Map<string, number | null>();
  for (const config of CONFIGS) {
    const result = await run(config, probe);
    latencies.set(
      config.label,
      result.wroteOutput
        ? measureLatency(probe, result.output, {
            start: LATENCY_PROBE.windowStart,
            length: LATENCY_PROBE.windowLength,
            maxDelay: LATENCY_PROBE.maxDelay,
          })
        : null,
    );
  }
  return latencies;
}

function readText(name: string): string | null {
  try {
    return readFileSync(join(RECORD_DIR, name), "utf8");
  } catch {
    return null;
  }
}

function writeText(name: string, text: string): void {
  mkdirSync(RECORD_DIR, { recursive: true });
  writeFileSync(join(RECORD_DIR, name), text);
}

const INDEX = "index.txt";

function latencyLines(latencies: Map<string, number | null>): Array<[string, string]> {
  return [...latencies].map(([label, value]) => [
    "latency",
    `${label.replace(/ /g, "_")} ${value === null ? "none" : `${value} (${fixed((value / 48), 2)} ms)`}`,
  ]);
}

/** Latencies as the index records them, keyed by configuration label. */
function recordedLatencies(): Map<string, number | null> | null {
  const text = readText(INDEX);
  if (text === null) return null;
  const latencies = new Map<string, number | null>();
  for (const line of text.split("\n")) {
    const match = /^latency (\S+) (none|\d+)/.exec(line);
    if (!match) continue;
    latencies.set(match[1]!.replace(/_/g, " "), match[2] === "none" ? null : Number(match[2]));
  }
  return latencies;
}

/** The latencies to measure gains with: recorded ones when checking, fresh ones when recording. */
export async function latencies(): Promise<Map<string, number | null>> {
  return recording ? measureLatencies() : (recordedLatencies() ?? measureLatencies());
}

export interface Outcome {
  name: string;
  problems: string[];
  /** True when this run wrote the record instead of checking it. */
  written: boolean;
}

/** The index: each configuration's latency, which every gain in the records is measured with. */
export async function checkIndex(): Promise<Outcome> {
  const latencies = await measureLatencies();
  const text = formatRecord(latencyLines(latencies), [], [
    "Capture record index: the delay of each detector configuration, measured on a probe.",
    "Every gain in the records is measured against the input delayed by this much.",
    "See web/bench/voice/README.md.",
  ]);
  if (recording) {
    writeText(INDEX, text);
    return { name: INDEX, problems: [], written: true };
  }
  const recorded = readText(INDEX);
  if (recorded === null) return { name: INDEX, problems: ["no index recorded; run npm run voice:record"], written: false };
  const before = recorded.split("\n").filter((line) => !line.startsWith("#") && line.trim() !== "");
  const after = text.split("\n").filter((line) => !line.startsWith("#") && line.trim() !== "");
  const problems: string[] = [];
  for (const line of new Set([...before, ...after])) {
    if (!before.includes(line)) problems.push(`now: ${line}`);
    else if (!after.includes(line)) problems.push(`recorded: ${line}`);
  }
  return { name: INDEX, problems, written: false };
}

const built = new Map<string, Float32Array>();

function fixtureSamples(name: string): Float32Array {
  let samples = built.get(name);
  if (!samples) {
    samples = buildFixture(fixtureByName(name));
    built.set(name, samples);
  }
  return samples;
}

/**
 * The fixture is still the audio its record was made from. Kept apart from
 * the behaviour, so that an edit to fixtures.ts, or arithmetic that differs
 * on another platform, does not read as a change in the detector.
 */
export function checkFixtureDigest(name: string): Outcome {
  const fresh = digest(fixtureSamples(name));
  if (recording) return { name, problems: [], written: true };
  const recordedText = readText(`${name}.txt`);
  if (recordedText === null) return { name, problems: ["no record; run npm run voice:record"], written: false };
  const recorded = parseRecord(recordedText).header.get("digest");
  return {
    name,
    problems:
      recorded === fresh
        ? []
        : [`the fixture is not the audio it was recorded from: digest ${recorded} recorded, ${fresh} now`],
    written: false,
  };
}

/** Every configuration (and scenario) over one fixture. */
export async function checkFixture(name: string): Promise<Outcome> {
  const fixture = fixtureByName(name);
  const input = fixtureSamples(name);
  const latencies = recording ? await measureLatencies() : recordedLatencies();
  if (latencies === null) {
    return { name, problems: ["no index recorded; run npm run voice:record"], written: false };
  }

  const runs: Array<{ label: string; lines: Array<[string, string]> }> = [];
  for (const config of CONFIGS) {
    const result = await run(config, input);
    runs.push({ label: config.label, lines: describeRun(input, result, latencies.get(config.label) ?? null) });
  }
  for (const scenario of SCENARIOS.filter((item) => item.fixture === name)) {
    for (const label of scenario.configs) {
      const config = configByLabel(label);
      const result = await run(config, input, scenario.actions);
      runs.push({
        label: `${label}, ${scenario.label}`,
        lines: describeRun(input, result, latencies.get(label) ?? null),
      });
    }
  }

  const header: Array<[string, string]> = [
    ["fixture", `${name} ${fixed(fixture.seconds, 2)} s, ${input.length} samples at 48000 Hz`],
    ["digest", digest(input)],
    ["level", `active ${fixed(dbfs(activeRms(input)), 1)} dBFS, peak ${fixed(dbfs(peakAbs(input)), 1)} dBFS`],
  ];
  const text = formatRecord(header, runs, [
    `Capture record of the fixture "${name}". See web/bench/voice/README.md for the notation.`,
    fixture.purpose,
    "Rewritten only by npm run voice:record, in a commit that names the audible change it intends.",
  ]);
  const file = `${name}.txt`;
  if (recording) {
    writeText(file, text);
    return { name, problems: [], written: true };
  }
  const recordedText = readText(file);
  if (recordedText === null) return { name, problems: ["no record; run npm run voice:record"], written: false };
  return { name, problems: compareRecords(parseRecord(recordedText), parseRecord(text)), written: false };
}

export function compareRecords(
  recorded: ReturnType<typeof parseRecord>,
  fresh: ReturnType<typeof parseRecord>,
): string[] {
  const problems: string[] = [];
  const byLabel = new Map<string, RecordRun>(recorded.runs.map((item) => [item.label, item]));
  for (const run of fresh.runs) {
    const before = byLabel.get(run.label);
    if (!before) {
      problems.push(`${run.label}: not recorded`);
      continue;
    }
    byLabel.delete(run.label);
    for (const problem of compareRun(before, run)) problems.push(`${run.label}: ${problem}`);
  }
  for (const label of byLabel.keys()) problems.push(`${label}: recorded, no longer run`);
  return problems;
}

/** One line per problem, for an assertion message. */
export function explain(outcome: Outcome): string {
  return `${outcome.name}:\n  ${outcome.problems.join("\n  ")}`;
}
