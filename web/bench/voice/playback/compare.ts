/**
 * Comparing playback records, and measuring how far repeated runs spread.
 *
 * Chrome's compressor and filters are its own code, so the playback record is
 * held to tolerances rather than to the digit. Each kind of measurement has
 * its own: the larger of a floor written here and four times the largest
 * spread seen across the runs the record was made from. The tolerances in
 * force are written into the record, so a check uses exactly what the
 * recording chose.
 */
import { compareGains, comparePeak, decodeGains, parseRecord, type RecordFile } from "../capture/record";

export interface Tolerances {
  latency: number;
  response: number;
  curve: number;
  envelope: number;
  ms: number;
  level: number;
}

/** Below these no tolerance goes, whatever the spread. */
export const FLOORS: Tolerances = {
  latency: 0,
  response: 0.05,
  curve: 0.1,
  envelope: 0.25,
  ms: 2,
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
  const result: Tolerances = { latency: 0, response: 0, curve: 0, envelope: 0, ms: 0, level: 0 };
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

function tokensOverTolerance(a: string, b: string, tolerance: number): string | null {
  const before = pairs(a);
  const after = pairs(b);
  const problems: string[] = [];
  for (const [name, value] of before) {
    const other = after.get(name);
    if (other === undefined) problems.push(`${name} no longer measured`);
    else if (Math.abs(value - other) > tolerance + 1e-9) problems.push(`${name}: ${value} recorded, ${other} now`);
  }
  for (const name of after.keys()) if (!before.has(name)) problems.push(`${name} not recorded`);
  return problems.length === 0 ? null : `${problems.slice(0, 6).join(", ")}${problems.length > 6 ? ` and ${problems.length - 6} more` : ""}`;
}

/** Every disagreement between a recorded playback record and a fresh one. */
export function compare(recorded: RecordFile, fresh: RecordFile, tolerances: Tolerances): string[] {
  const problems: string[] = [];
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
      let problem: string | null = null;
      if (kind === "latency") {
        if (Math.abs(Number(value) - Number(now)) > tolerances.latency) problem = `${value} recorded, ${now} now`;
      } else if (kind === "envelope") problem = compareGains(value, now, tolerances.envelope);
      else if (kind === "gains") problem = compareGains(value, now, Math.max(0.1, tolerances.envelope));
      else if (kind === "peak") problem = comparePeak(value, now);
      else if (kind === "exact") problem = value === now ? null : `"${value.slice(0, 80)}" recorded, "${now.slice(0, 80)}" now`;
      else problem = tokensOverTolerance(value, now, tolerances[kind]);
      if (problem) problems.push(`${run.label}: ${key}: ${problem}`);
    }
  }
  for (const label of runs.keys()) problems.push(`${label}: not recorded`);
  return problems;
}
