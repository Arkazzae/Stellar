/**
 * The playback record's page. run.mjs builds it with Vite, opens it in
 * Chrome and calls `window.__voice`.
 */
import type { PlaybackTuning } from "@/lib/voice/settings";
import {
  blockGains,
  compareGains,
  decodeGains,
  encodeGains,
  formatPeak,
  formatRecord,
  parseRecord,
} from "../capture/record";
import { buildFixture, fixtureByName } from "../fixtures";
import { compare, formatTolerances, spread, tolerancesFrom, tolerancesInForce, type Comparison } from "./compare";
import { characterise, levelStats, measureLatency } from "./measure";
import { decode, throughDetector, throughStrip, type StripSpec } from "./render";

const CUSTOM: Record<string, PlaybackTuning> = {
  flat: { warmthDb: 0, mudDb: 0, presenceDb: 0, airDb: 0, compression: 0 },
  bright: { warmthDb: -6, mudDb: -3, presenceDb: 8, airDb: 10, compression: 40 },
  extreme: { warmthDb: 12, mudDb: -12, presenceDb: 12, airDb: 12, compression: 100 },
};

/** Every profile at 100%, three custom tunings, and both sides of 100% on two profiles. */
const STRIPS: Array<{ label: string; spec: StripSpec }> = [
  { label: "off v100", spec: { profile: "off", volume: 100 } },
  { label: "natural v100", spec: { profile: "natural", volume: 100 } },
  { label: "studio v100", spec: { profile: "studio", volume: 100 } },
  { label: "broadcast v100", spec: { profile: "broadcast", volume: 100 } },
  ...Object.entries(CUSTOM).map(([name, tuning]) => ({
    label: `custom ${name} v100`,
    spec: { profile: "custom" as const, tuning, volume: 100 },
  })),
  { label: "studio v50", spec: { profile: "studio", volume: 50 } },
  { label: "studio v150", spec: { profile: "studio", volume: 150 } },
  { label: "studio v200", spec: { profile: "studio", volume: 200 } },
  { label: "off v50", spec: { profile: "off", volume: 50 } },
  { label: "off v200", spec: { profile: "off", volume: 200 } },
];

const SAMPLE_URL = "/sounds/voice-playback-test.mp3";
const PROFILES = ["off", "natural", "studio", "broadcast"] as const;

type Section = { label: string; lines: Array<[string, string]> };

async function sampleSections(name: string, signal: Float32Array): Promise<Section[]> {
  const sections: Section[] = [];
  for (const profile of PROFILES) {
    const output = await throughStrip({ profile, volume: 100 }, signal);
    sections.push({ label: `${name} through ${profile} v100`, lines: [["level", levelStats(output)]] });
  }
  return sections;
}

function speakingTurns(posts: unknown[]): number {
  let turns = 0;
  let speaking = false;
  for (const post of posts) {
    const next = (post as { speaking?: boolean }).speaking === true;
    if (next && !speaking) turns += 1;
    speaking = next;
  }
  return turns;
}

async function record(): Promise<string> {
  const sections: Section[] = [];
  for (const { label, spec } of STRIPS) sections.push({ label: `strip ${label}`, lines: await characterise(spec) });

  // The receiver's gate on a strip, over the speech fixture.
  const speech = buildFixture(fixtureByName("speech"));
  const gatedSpec: StripSpec = { profile: "studio", volume: 100, gated: true };
  const chainLatency = await measureLatency({ profile: "studio", volume: 100 });
  const gated = await throughStrip(gatedSpec, speech);
  sections.push({
    label: "strip studio v100 with the receiver's gate, fixture speech",
    lines: [
      ["gain", encodeGains(blockGains(speech, gated, chainLatency + 3840))],
      ["peak", formatPeak(gated)],
    ],
  });

  // The app's own sample, decoded by Chrome.
  const sample = await decode(SAMPLE_URL);
  sections.push(...(await sampleSections("voice-playback-test.mp3", sample)));
  const captured = await throughDetector(60, { gate: true, leveller: true }, sample);
  sections.push({
    label: "voice-playback-test.mp3 through the capture node, s60 gate+level, in Chrome",
    lines: [
      ["gain", encodeGains(blockGains(sample, captured.output, 3840))],
      ["peak", formatPeak(captured.output)],
      ["turns", `speaking-starts:${speakingTurns(captured.posts)} posts:${captured.posts.length}`],
    ],
  });
  return formatRecord([], sections, []);
}

/**
 * The speech fixture through Chrome's AudioWorklet, against the Node record
 * of the same configuration: the same output gains and the same messages in
 * the same order. This is what shows the Node driver runs the worklet as the
 * browser does.
 */
async function crossCheck(nodeRecord: string): Promise<{ problems: string[]; agreement: string }> {
  const parsed = parseRecord(nodeRecord);
  const run = parsed.runs.find((item) => item.label === "s60 gate+level");
  if (!run) return { problems: ["the capture record of the speech fixture has no s60 gate+level run"], agreement: "" };
  const speech = buildFixture(fixtureByName("speech"));
  const { output, posts } = await throughDetector(60, { gate: true, leveller: true }, speech);
  const problems: string[] = [];
  const measured = blockGains(speech, output, 3840);
  const recorded = decodeGains(run.lines.get("gain") ?? "");
  let worst = 0;
  measured.forEach((value, i) => {
    const other = recorded[i];
    if (typeof value === "number" && typeof other === "number") worst = Math.max(worst, Math.abs(value - other));
  });
  const gainProblem = compareGains(run.lines.get("gain") ?? "", encodeGains(measured));
  if (gainProblem) problems.push(`gain: ${gainProblem}`);
  const nodePosts = (run.lines.get("posts") ?? "")
    .split(" ")
    .map((token) => token.replace(/^[\d.]+/, ""))
    .join(" ");
  const chromePosts = posts
    .map((post) => {
      const { speaking, level } = post as { speaking: boolean; level: number };
      return `${speaking ? "+" : "-"}${level * 8}`;
    })
    .join(" ");
  if (nodePosts !== chromePosts) problems.push(`posts: Node "${nodePosts.slice(0, 120)}", Chrome "${chromePosts.slice(0, 120)}"`);
  return {
    problems,
    agreement: `${measured.length} blocks within ${worst.toFixed(3)} dB of the Node record (its rounding included), ${posts.length} posts in the same order`,
  };
}

async function recording(url: string, name: string): Promise<string> {
  const signal = await decode(url);
  return formatRecord([], await sampleSections(name, signal), []);
}

function withHeader(body: string, header: Array<[string, string]>, preamble: string[]): string {
  const parsed = parseRecord(body);
  return formatRecord(
    header,
    parsed.runs.map((run) => ({ label: run.label, lines: [...run.lines] })),
    preamble,
  );
}

declare global {
  interface Window {
    __voice?: unknown;
  }
}

window.__voice = {
  ready: true,
  record,
  recording,
  crossCheck,
  /** The record file: the first run's measurements, the spread over all runs, and the tolerances it sets. */
  finish(texts: string[], chrome: string, preamble: string[]): string {
    const observed = spread(texts);
    return withHeader(
      texts[0] ?? "",
      [
        ["chrome", chrome],
        ["runs", String(texts.length)],
        ["spread", formatTolerances(observed)],
        ["tolerance", formatTolerances(tolerancesFrom(observed))],
      ],
      preamble,
    );
  },
  spread(texts: string[]): string {
    return formatTolerances(spread(texts));
  },
  /** What disagrees, and the largest difference of each kind against its tolerance. */
  compare(recordedText: string, freshText: string): Comparison {
    const recorded = parseRecord(recordedText);
    return compare(recorded, parseRecord(freshText), tolerancesInForce(recorded));
  },
};
