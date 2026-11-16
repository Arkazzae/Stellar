/**
 * What a capture run is reduced to, as text a reviewer can diff.
 *
 * Every line is measured from outside the processor:
 *
 * - `posts`: each message the processor posted, at the end of the render
 *   quantum it was posted in, in ms with one decimal. `+` is speaking, `-` is
 *   not, and the number after it is the meter level in eighths.
 * - `speech`: the stretches between a `+` and the next `-` in `posts`.
 * - `gain`: per 10 ms of output, the gain the output carries over the input
 *   delayed by the configuration's latency, as RMS(out) / RMS(delayed in) in
 *   dB. `x` is closed (below -60 dB), `-` is a block with no input to measure
 *   against, `!` is output where there was no input. Runs are written
 *   `value*count`, straight-line ramps `from>to*count`; a decoded value is
 *   never more than 0.06 dB from the measured one.
 * - `peak`: the largest output sample in dBFS, and how many samples went
 *   past 0.95 (the leveller's ceiling) and past 1.
 * - `out none`: the processor writes no output in this configuration.
 * - `stopped`: when `process()` first returned false.
 *
 * The check compares posts, speech and stops exactly, gains within
 * GAIN_TOLERANCE_DB, peaks within PEAK_TOLERANCE_DB and the over-ceiling
 * counts within PEAK_COUNT_TOLERANCE.
 */
import { BLOCK, QUANTUM, SAMPLE_RATE, dbfs, fixed } from "../dsp";
import type { Post, ProcessorRun } from "./worklet";

export const GAIN_TOLERANCE_DB = 0.1;
export const PEAK_TOLERANCE_DB = 0.05;
/** Samples, or this fraction of the count, whichever is larger. */
export const PEAK_COUNT_TOLERANCE = { samples: 2, fraction: 0.01 };
const FIT_TOLERANCE_DB = 0.06;
const CLOSED_DB = -60;

export type GainValue = number | "x" | "-" | "!";

/** ms at the end of a render quantum. */
export function quantumEndMs(quantum: number): string {
  return fixed(((quantum + 1) * QUANTUM * 1000) / SAMPLE_RATE, 1);
}

function postToken(post: Post): string {
  const data = post.data as Record<string, unknown> | null;
  const time = quantumEndMs(post.quantum);
  if (
    data &&
    typeof data === "object" &&
    Object.keys(data).length === 2 &&
    typeof data.speaking === "boolean" &&
    typeof data.level === "number"
  ) {
    const eighths = data.level * 8;
    const level = Number.isInteger(eighths) ? String(eighths) : `${fixed(data.level, 4)}f`;
    return `${time}${data.speaking ? "+" : "-"}${level}`;
  }
  // Anything else the processor says is kept whole, so it cannot pass unseen.
  return `${time}=${JSON.stringify(data).replace(/\s/g, "")}`;
}

export function formatPosts(posts: readonly Post[]): string {
  return posts.length === 0 ? "none" : posts.map(postToken).join(" ");
}

export function formatSpeech(posts: readonly Post[]): string {
  const spans: string[] = [];
  let since: string | null = null;
  for (const post of posts) {
    const data = post.data as { speaking?: unknown } | null;
    if (!data || typeof data.speaking !== "boolean") continue;
    if (data.speaking && since === null) since = quantumEndMs(post.quantum);
    else if (!data.speaking && since !== null) {
      spans.push(`${since}-${quantumEndMs(post.quantum)}`);
      since = null;
    }
  }
  if (since !== null) spans.push(`${since}-end`);
  return spans.length === 0 ? "none" : spans.join(" ");
}

/**
 * The delay between input and output, found as the lag at which the output
 * correlates best with the input inside a window where the input is white
 * noise. Null when the output is silent there.
 */
export function measureLatency(
  input: Float32Array,
  output: Float32Array,
  window: { start: number; length: number; maxDelay: number },
): number | null {
  const { start, length, maxDelay } = window;
  let outEnergy = 0;
  for (let i = start; i < start + length; i += 1) outEnergy += (output[i] ?? 0) ** 2;
  if (outEnergy < 1e-12) return null;
  let best = -Infinity;
  let bestLag = 0;
  for (let lag = 0; lag <= Math.min(maxDelay, start); lag += 1) {
    let dot = 0;
    let inEnergy = 0;
    for (let i = start; i < start + length; i += 1) {
      const x = input[i - lag] as number;
      dot += (output[i] as number) * x;
      inEnergy += x * x;
    }
    if (inEnergy <= 0) continue;
    const score = dot / Math.sqrt(inEnergy);
    if (score > best) {
      best = score;
      bestLag = lag;
    }
  }
  return bestLag;
}

/** Gain per 10 ms block of output against the input delayed by `latency`. */
export function blockGains(input: Float32Array, output: Float32Array, latency: number): GainValue[] {
  const blocks = Math.floor(output.length / BLOCK);
  const values: GainValue[] = [];
  for (let block = 0; block < blocks; block += 1) {
    let inEnergy = 0;
    let outEnergy = 0;
    const from = block * BLOCK;
    const to = from + BLOCK;
    for (let i = from; i < to; i += 1) {
      const y = output[i] as number;
      outEnergy += y * y;
    }
    for (let i = Math.max(from, latency); i < to; i += 1) {
      const x = input[i - latency] as number;
      inEnergy += x * x;
    }
    if (inEnergy < 1e-20) values.push(outEnergy < 1e-20 ? "-" : "!");
    else {
      const db = 10 * Math.log10(Math.max(outEnergy, 1e-30) / inEnergy);
      values.push(db <= CLOSED_DB ? "x" : db);
    }
  }
  return values;
}

function round1(value: number): number {
  return Math.round(value * 10) / 10;
}

export function encodeGains(values: readonly GainValue[]): string {
  const tokens: string[] = [];
  let i = 0;
  while (i < values.length) {
    const first = values[i];
    if (typeof first !== "number") {
      let end = i + 1;
      while (end < values.length && values[end] === first) end += 1;
      tokens.push(end - i === 1 ? String(first) : `${first}*${end - i}`);
      i = end;
      continue;
    }
    const from = round1(first);
    let best = i;
    for (let end = i + 1; end < values.length; end += 1) {
      const last = values[end];
      if (typeof last !== "number") break;
      const to = round1(last);
      let fits = true;
      for (let k = i + 1; k < end; k += 1) {
        const line = from + ((to - from) * (k - i)) / (end - i);
        if (Math.abs((values[k] as number) - line) > FIT_TOLERANCE_DB) {
          fits = false;
          break;
        }
      }
      if (!fits) break;
      best = end;
    }
    const to = round1(values[best] as number);
    const count = best - i + 1;
    if (count === 1) tokens.push(fixed(from, 1));
    else if (from === to) tokens.push(`${fixed(from, 1)}*${count}`);
    else tokens.push(`${fixed(from, 1)}>${fixed(to, 1)}*${count}`);
    i = best + 1;
  }
  return tokens.join(" ");
}

export function decodeGains(text: string): GainValue[] {
  const values: GainValue[] = [];
  for (const token of text.split(" ").filter(Boolean)) {
    const [body = "", countText] = token.split("*");
    const count = countText === undefined ? 1 : Number(countText);
    if (body === "x" || body === "-" || body === "!") {
      for (let i = 0; i < count; i += 1) values.push(body);
      continue;
    }
    const [fromText = "", toText] = body.split(">");
    const from = Number(fromText);
    const to = toText === undefined ? from : Number(toText);
    for (let i = 0; i < count; i += 1) {
      values.push(count === 1 ? from : from + ((to - from) * i) / (count - 1));
    }
  }
  return values;
}

export function formatPeak(output: Float32Array): string {
  let peak = 0;
  let hot = 0;
  let over = 0;
  for (const value of output) {
    const magnitude = Math.abs(value);
    if (magnitude > peak) peak = magnitude;
    if (magnitude > 0.95) hot += 1;
    if (magnitude > 1) over += 1;
  }
  return peak === 0 ? "none" : `${fixed(dbfs(peak), 2)} >0.95:${hot} >1:${over}`;
}

/** The lines one run contributes to the record, in order. */
export function describeRun(input: Float32Array, run: ProcessorRun, latency: number | null): Array<[string, string]> {
  const lines: Array<[string, string]> = [
    ["posts", formatPosts(run.posts)],
    ["speech", formatSpeech(run.posts)],
  ];
  if (!run.wroteOutput) lines.push(["out", "none"]);
  else {
    lines.push(["gain", encodeGains(blockGains(input, run.output, latency ?? 0))]);
    lines.push(["peak", formatPeak(run.output)]);
  }
  if (run.stoppedAt !== null) lines.push(["stopped", quantumEndMs(run.stoppedAt)]);
  return lines;
}

// ---- the record file ----

export interface RecordRun {
  label: string;
  lines: Map<string, string>;
}

export interface RecordFile {
  header: Map<string, string>;
  runs: RecordRun[];
}

export function formatRecord(header: Array<[string, string]>, runs: Array<{ label: string; lines: Array<[string, string]> }>, preamble: string[]): string {
  const out = [...preamble.map((line) => `# ${line}`), ""];
  for (const [key, value] of header) out.push(`${key} ${value}`);
  for (const run of runs) {
    out.push("", `run ${run.label}`);
    for (const [key, value] of run.lines) out.push(`  ${key} ${value}`);
  }
  return `${out.join("\n")}\n`;
}

export function parseRecord(text: string): RecordFile {
  const header = new Map<string, string>();
  const runs: RecordRun[] = [];
  let current: RecordRun | null = null;
  for (const raw of text.split("\n")) {
    if (raw.startsWith("#") || raw.trim() === "") continue;
    if (raw.startsWith("run ")) {
      current = { label: raw.slice(4).trim(), lines: new Map() };
      runs.push(current);
      continue;
    }
    const line = raw.trim();
    const space = line.indexOf(" ");
    const key = space === -1 ? line : line.slice(0, space);
    const value = space === -1 ? "" : line.slice(space + 1);
    if (current && raw.startsWith("  ")) current.lines.set(key, value);
    else header.set(key, value);
  }
  return { header, runs };
}

// ---- comparison ----

function asDb(value: GainValue): number | null {
  if (value === "x") return CLOSED_DB;
  return typeof value === "number" ? Math.max(value, CLOSED_DB) : null;
}

function blockMs(block: number): string {
  return `${block * 10}-${(block + 1) * 10} ms`;
}

/** Null when the gains agree; otherwise the first disagreements, in words. */
export function compareGains(recorded: string, fresh: string, tolerance = GAIN_TOLERANCE_DB): string | null {
  const before = decodeGains(recorded);
  const after = decodeGains(fresh);
  if (before.length !== after.length) {
    return `${before.length} blocks recorded, ${after.length} now`;
  }
  const differences: string[] = [];
  let count = 0;
  let worst = 0;
  for (let i = 0; i < before.length; i += 1) {
    const a = before[i] as GainValue;
    const b = after[i] as GainValue;
    const da = asDb(a);
    const db = asDb(b);
    let differs: boolean;
    if (da === null || db === null) differs = a !== b;
    else {
      const delta = Math.abs(da - db);
      differs = delta > tolerance;
      if (differs) worst = Math.max(worst, delta);
    }
    if (!differs) continue;
    count += 1;
    if (differences.length < 4) {
      const show = (value: GainValue) => (typeof value === "number" ? `${fixed(value, 2)} dB` : value);
      differences.push(`${blockMs(i)}: ${show(a)} recorded, ${show(b)} now`);
    }
  }
  if (count === 0) return null;
  const largest = worst > 0 ? `, largest numeric difference ${fixed(worst, 2)} dB` : "";
  return `${count} of ${before.length} blocks differ${largest}; ${differences.join("; ")}`;
}

export function comparePeak(recorded: string, fresh: string): string | null {
  if (recorded === fresh) return null;
  const parse = (text: string) => {
    const match = /^(-?[\d.]+) >0\.95:(\d+) >1:(\d+)$/.exec(text);
    return match ? { db: Number(match[1]), hot: Number(match[2]), over: Number(match[3]) } : null;
  };
  const a = parse(recorded);
  const b = parse(fresh);
  if (!a || !b) return `"${recorded}" recorded, "${fresh}" now`;
  const slack = (count: number) =>
    Math.max(PEAK_COUNT_TOLERANCE.samples, Math.ceil(count * PEAK_COUNT_TOLERANCE.fraction));
  const ok =
    Math.abs(a.db - b.db) <= PEAK_TOLERANCE_DB &&
    Math.abs(a.hot - b.hot) <= slack(a.hot) &&
    Math.abs(a.over - b.over) <= slack(a.over);
  return ok ? null : `"${recorded}" recorded, "${fresh}" now`;
}

function firstDifference(recorded: string, fresh: string): string {
  const a = recorded.split(" ");
  const b = fresh.split(" ");
  for (let i = 0; i < Math.max(a.length, b.length); i += 1) {
    if (a[i] !== b[i]) {
      return `from token ${i + 1}: recorded "${a.slice(i, i + 4).join(" ")}", now "${b.slice(i, i + 4).join(" ")}"`;
    }
  }
  return "they differ";
}

/** Every disagreement between a recorded run and a fresh one, in words. */
export function compareRun(recorded: RecordRun, fresh: RecordRun): string[] {
  const problems: string[] = [];
  const keys = new Set([...recorded.lines.keys(), ...fresh.lines.keys()]);
  for (const key of keys) {
    const before = recorded.lines.get(key);
    const after = fresh.lines.get(key);
    if (before === undefined || after === undefined) {
      problems.push(`${key}: ${before === undefined ? "not recorded" : "no longer produced"}`);
      continue;
    }
    let problem: string | null = null;
    if (key === "gain") problem = compareGains(before, after);
    else if (key === "peak") problem = comparePeak(before, after);
    else if (before !== after) problem = firstDifference(before, after);
    if (problem) problems.push(`${key}: ${problem}`);
  }
  return problems;
}
