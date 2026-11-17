/**
 * Comparing playback records, and measuring how far repeated runs spread.
 *
 * Chrome's compressor and filters are its own code, so the playback record is
 * held to tolerances rather than to the digit. Each kind of measurement has
 * its own: the larger of a floor written here and four times the largest
 * spread seen across the runs the record was made from, which the record
 * writes down. A check applies the larger of that and today's floor.
 */
import { compareGains, comparePeak, decodeGains, parseRecord, type GainValue, type RecordFile } from "../capture/record";

export interface Tolerances {
  latency: number;
  response: number;
  curve: number;
  envelope: number;
  /** dB·ms: how far the area of an envelope after its step may move; see `envelopeArea`. */
  area: number;
  ms: number;
  level: number;
}

/**
 * Below these no tolerance goes, whatever the spread.
 *
 * Repeated runs on one machine agree to the digit, so these are what holds
 * across platforms. Against the record (macOS arm64, Chrome 155), Linux gave
 * the same measurements to the digit with the record's own Chrome 155 on x64,
 * Chromium 156 on arm64 and Chrome 157 on x64: the responses and curves within
 * 0.01 dB, latency and overshoot unchanged, and the release bump of the three
 * `off` strips up to 0.26 dB lower, which moved their settling time by 2 ms
 * and the area of their release by up to 4.2 dB·ms. The envelope, area and
 * settling floors leave a
 * little over twice that; the others were already five times or more what
 * was seen.
 */
export const FLOORS: Tolerances = {
  latency: 0,
  response: 0.05,
  curve: 0.1,
  envelope: 0.6,
  area: 10,
  ms: 5,
  level: 0.1,
};

type Kind = keyof Tolerances | "gains" | "peak" | "exact";

function kindOf(key: string): Kind {
  if (key === "latency") return "latency";
  if (key.startsWith("response")) return "response";
  if (key === "curve") return "curve";
  if (key === "attack" || key === "release") return "envelope";
  if (key === "settle") return "ms";
  if (key === "level" || key === "overshoot") return "level";
  if (key === "gain") return "gains";
  if (key === "peak") return "peak";
  return "exact";
}

function pairs(text: string): Map<string, number> {
  const out = new Map<string, number>();
  for (const token of text.split(" ").filter(Boolean)) {
    const colon = token.lastIndexOf(":");
    out.set(token.slice(0, colon), Number(token.slice(colon + 1)));
  }
  return out;
}

/** Largest difference between two values of the same line, in its own unit. */
function difference(key: string, a: string, b: string): number {
  const kind = kindOf(key);
  if (kind === "latency") return Math.abs(Number(a) - Number(b));
  if (kind === "envelope" || kind === "gains") {
    const before = decodeGains(a);
    const after = decodeGains(b);
    if (before.length !== after.length) return Infinity;
    let worst = 0;
    for (let i = 0; i < before.length; i += 1) {
      const x = before[i];
      const y = after[i];
      if (typeof x === "number" && typeof y === "number") worst = Math.max(worst, Math.abs(x - y));
      else if (x !== y) return Infinity;
    }
    return worst;
  }
  if (kind === "response" || kind === "curve" || kind === "level" || kind === "ms") {
    const before = pairs(a);
    const after = pairs(b);
    let worst = 0;
    for (const [name, value] of before) {
      const other = after.get(name);
      if (other === undefined) return Infinity;
      worst = Math.max(worst, Math.abs(value - other));
    }
    return before.size === after.size ? worst : Infinity;
  }
  return a === b ? 0 : Infinity;
}

/** The largest spread of each kind over a set of runs of the same record. */
export function spread(texts: string[]): Tolerances {
  const result: Tolerances = { latency: 0, response: 0, curve: 0, envelope: 0, area: 0, ms: 0, level: 0 };
  const [first, ...rest] = texts.map(parseRecord);
  if (!first) return result;
  for (const other of rest) {
    const runs = new Map(other.runs.map((run) => [run.label, run]));
    for (const run of first.runs) {
      const twin = runs.get(run.label);
      if (!twin) continue;
      for (const [key, value] of run.lines) {
        const kind = kindOf(key);
        const otherValue = twin.lines.get(key);
        if (otherValue === undefined) continue;
        const delta = difference(key, value, otherValue);
        if (kind === "envelope") {
          result.area = Math.max(result.area, Math.abs(envelopeArea(value) - envelopeArea(otherValue)));
        }
        if (kind === "gains") result.envelope = Math.max(result.envelope, delta);
        else if (kind in result) {
          const name = kind as keyof Tolerances;
          result[name] = Math.max(result[name], delta);
        }
      }
    }
  }
  return result;
}

export function tolerancesFrom(observed: Tolerances): Tolerances {
  const out = { ...FLOORS };
  for (const key of Object.keys(out) as Array<keyof Tolerances>) {
    out[key] = Math.max(FLOORS[key], 4 * observed[key]);
  }
  return out;
}

export function formatTolerances(tolerances: Tolerances): string {
  return Object.entries(tolerances)
    .map(([key, value]) => `${key}:${Number.isFinite(value) ? Number(value.toFixed(3)) : "inf"}`)
    .join(" ");
}

export function parseTolerances(text: string | undefined): Tolerances {
  const out = { ...FLOORS };
  for (const [key, value] of pairs(text ?? "")) {
    if (key in out) out[key as keyof Tolerances] = value;
  }
  return out;
}

/**
 * The tolerances a check applies: those written into the record when it was
 * made, but never under today's floors, so a floor can be raised without
 * recording again.
 */
export function tolerancesInForce(recorded: RecordFile): Tolerances {
  const written = parseTolerances(recorded.header.get("tolerance"));
  const out = { ...FLOORS };
  for (const key of Object.keys(out) as Array<keyof Tolerances>) out[key] = Math.max(FLOORS[key], written[key]);
  return out;
}

/**
 * The 10 ms gains of the gated strip and of the capture node over speech.
 * They were the same on every platform measured; this is the record's own
 * rounding (0.06 dB) with room, and does not follow the envelope floor.
 */
const GAINS_TOLERANCE_DB = 0.25;

/** The step in each envelope sits 10 ms into it; see measure.ts. */
const ENVELOPE_LEAD_MS = 10;

/**
 * The area between an envelope and the level it ends at, from the step on,
 * in dB·ms: how far below (or above) its final level the output stays, and
 * for how long. A compressor's release or attack that is a few per cent
 * slower moves no single millisecond far, since a release is slow and
 * shallow, but it moves this sum: lengthening the studio release by 10 ms
 * moved it by 40 dB·ms where another platform moved it by 4 at most.
 */
function envelopeArea(text: string): number {
  const values = decodeGains(text).map((value) => db(value) ?? Number.NaN);
  const final = values[values.length - 1] ?? 0;
  let sum = 0;
  for (let i = ENVELOPE_LEAD_MS; i < values.length; i += 1) sum += final - (values[i] ?? final);
  return sum;
}

function db(value: GainValue | undefined): number | null {
  if (value === "x") return -60;
  return typeof value === "number" ? Math.max(value, -60) : null;
}

/**
 * How far each 1 ms block of an envelope lies from the recorded one, in level
 * only. Offline rendering is sample-accurate, and on every platform measured
 * each step lands on the same block, so a block is held to its own recorded
 * value and not to its neighbours': where a step makes the envelope steep is
 * exactly where a change to an attack or a release shows, and the latency is
 * held to the sample besides. What differs between platforms is the level of
 * the bump a compressor leaves as it releases, which the floor allows for.
 */
function envelopeExcess(recorded: string, fresh: string): { worst: number; at: number; over: (tolerance: number) => number } | string {
  const before = decodeGains(recorded);
  const after = decodeGains(fresh);
  if (before.length !== after.length) return `${before.length} ms recorded, ${after.length} now`;
  const excess: number[] = [];
  for (let i = 0; i < after.length; i += 1) {
    const now = db(after[i]);
    const then = db(before[i]);
    if (now === null || then === null) {
      if (after[i] !== before[i]) return `${i - ENVELOPE_LEAD_MS} ms: ${String(before[i])} recorded, ${String(after[i])} now`;
      excess.push(0);
      continue;
    }
    excess.push(Math.abs(now - then));
  }
  let at = 0;
  for (let i = 1; i < excess.length; i += 1) if ((excess[i] ?? 0) > (excess[at] ?? 0)) at = i;
  return {
    worst: excess[at] ?? 0,
    at: at - ENVELOPE_LEAD_MS,
    over: (tolerance) => excess.filter((value) => value > tolerance + 1e-9).length,
  };
}

/** The largest difference of one kind, against the tolerance it is held to. */
export interface Margin {
  kind: string;
  unit: string;
  worst: number;
  tolerance: number;
  where: string;
}

export interface Comparison {
  problems: string[];
  margins: Margin[];
}

const MARGINS: Array<{ kind: Kind; unit: string; tolerance: (t: Tolerances) => number; what: string }> = [
  { kind: "latency", unit: "samples", tolerance: (t) => t.latency, what: "latency" },
  { kind: "response", unit: "dB", tolerance: (t) => t.response, what: "frequency response, low and hot" },
  { kind: "curve", unit: "dB", tolerance: (t) => t.curve, what: "output against input level" },
  { kind: "envelope", unit: "dB", tolerance: (t) => t.envelope, what: "attack and release envelopes, per ms" },
  { kind: "area", unit: "dB·ms", tolerance: (t) => t.area, what: "attack and release areas" },
  { kind: "ms", unit: "ms", tolerance: (t) => t.ms, what: "settling times" },
  { kind: "level", unit: "dB", tolerance: (t) => t.level, what: "overshoot and the sample's levels" },
  { kind: "gains", unit: "dB", tolerance: () => GAINS_TOLERANCE_DB, what: "gains of the gated strip and the capture node" },
];

/** Every disagreement between a recorded playback record and a fresh one, and the margin of each kind. */
export function compare(recorded: RecordFile, fresh: RecordFile, tolerances: Tolerances): Comparison {
  const problems: string[] = [];
  const worst = new Map<Kind, { value: number; where: string }>();
  const note = (kind: Kind, value: number, where: string) => {
    if (value > (worst.get(kind)?.value ?? -1)) worst.set(kind, { value, where });
  };
  const runs = new Map(fresh.runs.map((run) => [run.label, run]));
  for (const run of recorded.runs) {
    const twin = runs.get(run.label);
    if (!twin) {
      problems.push(`${run.label}: recorded, no longer measured`);
      continue;
    }
    runs.delete(run.label);
    for (const [key, value] of run.lines) {
      const now = twin.lines.get(key);
      if (now === undefined) {
        problems.push(`${run.label}: ${key} no longer measured`);
        continue;
      }
      const kind = kindOf(key);
      const where = `${run.label}, ${key}`;
      let problem: string | null = null;
      if (kind === "latency") {
        const delta = Math.abs(Number(value) - Number(now));
        note(kind, delta, where);
        if (delta > tolerances.latency) problem = `${value} recorded, ${now} now`;
      } else if (kind === "envelope") {
        const result = envelopeExcess(value, now);
        if (typeof result === "string") problem = result;
        else {
          note(kind, result.worst, `${where}, ${result.at} ms from the step`);
          const over = result.over(tolerances.envelope);
          const recordedArea = envelopeArea(value);
          const areaNow = envelopeArea(now);
          const areaDelta = Math.abs(areaNow - recordedArea);
          note("area", areaDelta, where);
          const faults: string[] = [];
          if (over > 0) {
            faults.push(
              `${over} ms more than ${tolerances.envelope} dB from the recorded envelope; worst ${result.worst.toFixed(2)} dB at ${result.at} ms from the step`,
            );
          }
          if (areaDelta > tolerances.area) {
            faults.push(`area ${recordedArea.toFixed(1)} dB·ms recorded, ${areaNow.toFixed(1)} now`);
          }
          if (faults.length > 0) problem = faults.join("; ");
        }
      } else if (kind === "gains") {
        note(kind, difference(key, value, now), where);
        problem = compareGains(value, now, GAINS_TOLERANCE_DB);
      } else if (kind === "peak") problem = comparePeak(value, now);
      else if (kind === "exact") problem = value === now ? null : `"${value.slice(0, 80)}" recorded, "${now.slice(0, 80)}" now`;
      else {
        const before = pairs(value);
        const after = pairs(now);
        const over: string[] = [];
        for (const [name, recordedValue] of before) {
          const other = after.get(name);
          if (other === undefined) {
            over.push(`${name} no longer measured`);
            continue;
          }
          const delta = Math.abs(recordedValue - other);
          note(kind, delta, `${where} ${name}`);
          if (delta > tolerances[kind] + 1e-9) over.push(`${name}: ${recordedValue} recorded, ${other} now`);
        }
        for (const name of after.keys()) if (!before.has(name)) over.push(`${name} not recorded`);
        if (over.length > 0) problem = `${over.slice(0, 6).join(", ")}${over.length > 6 ? ` and ${over.length - 6} more` : ""}`;
      }
      if (problem) problems.push(`${run.label}: ${key}: ${problem}`);
    }
  }
  for (const label of runs.keys()) problems.push(`${label}: not recorded`);
  const margins = MARGINS.map(({ kind, unit, tolerance, what }) => {
    const largest = worst.get(kind);
    return {
      kind: what,
      unit,
      worst: largest?.value ?? 0,
      tolerance: tolerance(tolerances),
      where: largest && largest.value > 0 ? largest.where : "-",
    };
  });
  return { problems, margins };
}
