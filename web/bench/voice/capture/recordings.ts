/**
 * Real recordings: whatever WAV files sit in web/bench/voice/recordings/.
 *
 * The folder is ignored by git, and so is everything written here beside a
 * recording: its capture record (`<name>.capture.txt`, written the first time
 * and checked every time after) and a summary of what the detector made of
 * it (`<name>.summary.txt`, rewritten on every run). Nothing derived from a
 * recording's samples leaves the folder.
 *
 * Each recording runs as it is and in four versions made here from it:
 * quieter with a quiet room under it, hotter into clipping, over a fan, and
 * with typing and breaths in its pauses.
 */
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { BLOCK, QUANTUM, SAMPLE_RATE, dbToGain, dbfs, digest, fixed, rms } from "../dsp";
import { ROOM } from "../fixtures";
import { decodeWav } from "../wav";
import { configByLabel, latencies, run, recording as rerecording } from "./check";
import { blockGains, compareRun, describeRun, formatRecord, parseRecord, type GainValue } from "./record";
import type { Post } from "./worklet";

export const RECORDINGS_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "recordings");

export function listRecordings(): string[] {
  if (!existsSync(RECORDINGS_DIR)) return [];
  return readdirSync(RECORDINGS_DIR)
    .filter((name) => name.toLowerCase().endsWith(".wav"))
    .sort();
}

interface Version {
  label: string;
  make(samples: Float32Array): Float32Array;
  configs: string[];
}

function scaled(samples: Float32Array, db: number, clip = false): Float32Array {
  const gain = dbToGain(db);
  return samples.map((value) => {
    const next = value * gain;
    return clip ? Math.max(-1, Math.min(1, next)) : next;
  });
}

function plus(samples: Float32Array, other: Float32Array): Float32Array {
  return samples.map((value, i) => value + (other[i] ?? 0));
}

const VERSIONS: Version[] = [
  {
    label: "as recorded",
    make: (samples) => samples,
    configs: ["s60 meter", "s60 gate", "s60 level", "s60 gate+level", "s100 meter", "remote meter"],
  },
  {
    label: "24 dB quieter over a -76 dBFS room",
    make: (samples) => plus(scaled(samples, -24), ROOM.room(samples.length, -76)),
    configs: ["s60 gate"],
  },
  {
    label: "10 dB hotter, clipped at full scale",
    make: (samples) => scaled(samples, 10, true),
    configs: ["s60 gate+level"],
  },
  {
    label: "10 dB quieter over a -42 dBFS fan",
    make: (samples) => plus(scaled(samples, -10), ROOM.fan(samples.length, -42)),
    configs: ["s60 gate"],
  },
  {
    label: "10 dB quieter with typing and breaths",
    make: (samples) =>
      plus(plus(scaled(samples, -10), ROOM.room(samples.length, -70)), ROOM.clicksAndBreath(samples.length)),
    configs: ["s60 gate"],
  },
];

/** A whole number of render quanta, padded with silence. */
function padded(samples: Float32Array): Float32Array {
  const length = Math.ceil(samples.length / QUANTUM) * QUANTUM;
  if (length === samples.length) return samples;
  const out = new Float32Array(length);
  out.set(samples);
  return out;
}

// ---- what the detector made of it ----

/**
 * Where the recording has speech, from its own envelope: 10 ms blocks above
 * both 15 dB over its quietest tenth and 45 dB under its loudest block.
 */
function referenceActivity(samples: Float32Array): boolean[] {
  const blocks = Math.floor(samples.length / BLOCK);
  const levels: number[] = [];
  for (let b = 0; b < blocks; b += 1) levels.push(dbfs(rms(samples, b * BLOCK, (b + 1) * BLOCK)));
  const sorted = [...levels].sort((a, b) => a - b);
  const quiet = sorted[Math.floor(sorted.length * 0.1)] ?? -120;
  const loudest = sorted[sorted.length - 1] ?? 0;
  const threshold = Math.max(quiet + 15, loudest - 45);
  return levels.map((level) => level >= threshold);
}

/** Speaking or not at the end of each 10 ms block, from the posts. */
function speakingPerBlock(posts: readonly Post[], blocks: number): boolean[] {
  const out: boolean[] = [];
  let index = 0;
  let speaking = false;
  for (let b = 0; b < blocks; b += 1) {
    const endSample = (b + 1) * BLOCK;
    while (index < posts.length && ((posts[index]?.quantum ?? 0) + 1) * QUANTUM <= endSample) {
      const data = posts[index]?.data as { speaking?: boolean } | undefined;
      if (typeof data?.speaking === "boolean") speaking = data.speaking;
      index += 1;
    }
    out.push(speaking);
  }
  return out;
}

function percent(part: number, whole: number): string {
  return whole === 0 ? "n/a" : `${fixed((100 * part) / whole, 1)}%`;
}

interface Coverage {
  caught: string;
  passed: string;
  energy: string;
  open: string;
  onsets: string;
}

/** How much of the speech the gate passes, measured on a gate-only run. */
function gateCoverage(
  input: Float32Array,
  output: Float32Array,
  active: boolean[],
  latency: number,
): Omit<Coverage, "caught"> {
  const LATENCY = latency;
  const gains = blockGains(input, output, LATENCY);
  const shift = Math.round(LATENCY / BLOCK);
  const gainAt = (inputBlock: number): GainValue => gains[inputBlock + shift] ?? "-";
  const isOpen = (value: GainValue) => typeof value === "number" && value >= -3;
  let speechBlocks = 0;
  let passed = 0;
  let quietBlocks = 0;
  let openQuiet = 0;
  let inEnergy = 0;
  let outEnergy = 0;
  for (let b = 0; b < active.length; b += 1) {
    const from = b * BLOCK;
    const value = gainAt(b);
    if (active[b]) {
      speechBlocks += 1;
      if (isOpen(value)) passed += 1;
      for (let i = from; i < from + BLOCK; i += 1) {
        inEnergy += (input[i] ?? 0) ** 2;
        outEnergy += (output[i + LATENCY] ?? 0) ** 2;
      }
    } else {
      quietBlocks += 1;
      if (isOpen(value)) openQuiet += 1;
    }
  }
  // Onsets: speech after at least 300 ms without, and how much of the
  // first 100 ms the gate let through.
  const onsets: Array<{ at: number; kept: number; lateMs: number }> = [];
  let silentRun = 30;
  for (let b = 0; b < active.length; b += 1) {
    if (!active[b]) {
      silentRun += 1;
      continue;
    }
    if (silentRun >= 30) {
      let a = 0;
      let c = 0;
      for (let i = b * BLOCK; i < Math.min(input.length, (b + 10) * BLOCK); i += 1) {
        a += (input[i] ?? 0) ** 2;
        c += (output[i + LATENCY] ?? 0) ** 2;
      }
      let open = b;
      while (open < active.length && !isOpen(gainAt(open))) open += 1;
      onsets.push({ at: b * 10, kept: a > 0 ? c / a : 0, lateMs: (open - b) * 10 });
    }
    silentRun = 0;
  }
  const whole = onsets.filter((onset) => onset.kept >= 0.9).length;
  const half = onsets.filter((onset) => onset.kept >= 0.5).length;
  const each = onsets
    .map((onset) => `${fixed(onset.at / 1000, 2)} s kept ${fixed(onset.kept * 100, 0)}%, open after ${onset.lateMs} ms`)
    .join("; ");
  return {
    passed: percent(passed, speechBlocks),
    energy: percent(outEnergy, inEnergy),
    open: percent(openQuiet, quietBlocks),
    onsets: `${onsets.length} after a pause: ${whole} keep >=90% of their first 100 ms, ${half} keep >=50% (${each})`,
  };
}

export interface RecordingOutcome {
  name: string;
  problems: string[];
  written: boolean;
  summary: string;
}

export async function checkRecording(name: string): Promise<RecordingOutcome> {
  const decoded = decodeWav(readFileSync(join(RECORDINGS_DIR, name)));
  if (decoded.sampleRate !== SAMPLE_RATE) {
    return {
      name,
      problems: [
        `${name} is at ${decoded.sampleRate} Hz; the record needs 48000 Hz. Convert it, e.g. afconvert -f WAVE -d LEI16@48000 -c 1 in out.wav, or ffmpeg -i in -ac 1 -ar 48000 out.wav`,
      ],
      written: false,
      summary: "",
    };
  }
  const original = padded(decoded.samples);
  const active = referenceActivity(original);
  const runs: Array<{ label: string; lines: Array<[string, string]> }> = [];
  const summary: string[] = [
    `${name}: ${fixed(original.length / SAMPLE_RATE, 2)} s, speech in ${percent(active.filter(Boolean).length, active.length)} of it by its own envelope`,
  ];

  const delays = await latencies();
  for (const version of VERSIONS) {
    const input = padded(version.make(original));
    const coverage: Partial<Coverage> = {};
    for (const label of version.configs) {
      const result = await run(configByLabel(label), input);
      const latency = delays.get(label) ?? null;
      // The versions are mixed with material from fixtures.ts; an edit there
      // shows up here by name rather than as a change in the detector.
      runs.push({
        label: `${version.label}, ${label}`,
        lines: [["input-digest", digest(input)], ...describeRun(input, result, latency)],
      });
      if (label.endsWith("meter") || label === "s60 gate") {
        const speaking = speakingPerBlock(result.posts, active.length);
        const caught = active.filter((isActive, b) => isActive && speaking[b]).length;
        const line = `${label}: speaking during ${percent(caught, active.filter(Boolean).length)} of the speech`;
        if (label === "s60 gate") {
          Object.assign(coverage, gateCoverage(input, result.output, active, latency ?? 0));
        }
        summary.push(`  ${version.label}, ${line}`);
      }
    }
    if (coverage.passed) {
      summary.push(
        `  ${version.label}, s60 gate: open (>= -3 dB) over ${coverage.passed} of the speech, ${coverage.energy} of its energy passed; open over ${coverage.open} of the rest; onsets: ${coverage.onsets}`,
      );
    }
  }

  writeFileSync(join(RECORDINGS_DIR, name.replace(/\.wav$/i, ".summary.txt")), `${summary.join("\n")}\n`);
  const text = formatRecord(
    [
      ["recording", `${name} ${fixed(original.length / SAMPLE_RATE, 2)} s`],
      ["digest", digest(original)],
    ],
    runs,
    [
      `Capture record of the recording ${name}. It stays in this ignored folder with the recording.`,
      "Delete this file to record it again.",
    ],
  );
  const recordFile = join(RECORDINGS_DIR, name.replace(/\.wav$/i, ".capture.txt"));
  if (!existsSync(recordFile) || rerecording) {
    writeFileSync(recordFile, text);
    return { name, problems: [], written: true, summary: summary.join("\n") };
  }
  const recorded = parseRecord(readFileSync(recordFile, "utf8"));
  const fresh = parseRecord(text);
  const problems: string[] = [];
  if (recorded.header.get("digest") !== fresh.header.get("digest")) {
    problems.push("the recording itself changed since its record was written; delete the .capture.txt to record it again");
  }
  const before = new Map(recorded.runs.map((item) => [item.label, item]));
  for (const item of fresh.runs) {
    const old = before.get(item.label);
    if (!old) problems.push(`${item.label}: not recorded`);
    else for (const problem of compareRun(old, item)) problems.push(`${item.label}: ${problem}`);
  }
  return { name, problems, written: false, summary: summary.join("\n") };
}
