/**
 * What characterises a playback chain, measured from outside it.
 *
 * - `latency`: samples between input and output, the lag at which they
 *   correlate best on low-level white noise.
 * - `response-low`: gain in dB at 31 third-octave frequencies from 20 Hz to
 *   20 kHz, from a multitone at -60 dBFS RMS, under every compressor
 *   threshold. This is the equaliser and every fixed gain in the chain.
 * - `response-hot`: the same multitone at -20 dBFS RMS, where the
 *   compressor and the limiter work.
 * - `curve`: output level against input level for a 1 kHz tone, in dBFS of
 *   the tone's peak, steady state, one render per level.
 * - `attack` / `release`: the output envelope of a 1 kHz tone stepping from
 *   -40 to -6 dBFS and back, per millisecond from 10 ms before each step,
 *   aligned by the latency, written like the capture gains (value*count and
 *   from>to*count).
 * - `settle`: from those envelopes, ms to settle within 1 dB after each step.
 * - `overshoot`: how far the attack overshoots the level it settles at, in dB.
 */
import { encodeGains, type GainValue } from "../capture/record";
import { dbfs, fixed, mulberry32, rms } from "../dsp";
import { RATE, throughStrip, type StripSpec } from "./render";

const THIRD_OCTAVES = [
  20, 25, 31, 40, 50, 63, 80, 100, 125, 160, 200, 250, 315, 400, 500, 630, 800, 1000, 1250, 1600, 2000,
  2500, 3150, 4000, 5000, 6300, 8000, 10000, 12500, 16000, 20000,
];

/** Amplitude of the component at `hz` over `length` samples from `start`. */
function amplitudeAt(x: Float32Array, start: number, length: number, hz: number): number {
  const step = (2 * Math.PI * hz) / RATE;
  const cosStep = Math.cos(step);
  const sinStep = Math.sin(step);
  let c = 1;
  let s = 0;
  let re = 0;
  let im = 0;
  for (let n = 0; n < length; n += 1) {
    const value = x[start + n] ?? 0;
    re += value * c;
    im -= value * s;
    const next = c * cosStep - s * sinStep;
    s = s * cosStep + c * sinStep;
    c = next;
    if ((n & 1023) === 1023) {
      // Keep the rotating phasor on the unit circle.
      const norm = Math.hypot(c, s);
      c /= norm;
      s /= norm;
    }
  }
  return (2 * Math.hypot(re, im)) / length;
}

function multitone(rmsDb: number, samples: number, seed: number): Float32Array {
  const rng = mulberry32(seed);
  const amplitude = (10 ** (rmsDb / 20) * Math.SQRT2) / Math.sqrt(THIRD_OCTAVES.length);
  const phases = THIRD_OCTAVES.map(() => rng() * 2 * Math.PI);
  const out = new Float32Array(samples);
  for (let k = 0; k < THIRD_OCTAVES.length; k += 1) {
    const step = (2 * Math.PI * (THIRD_OCTAVES[k] ?? 0)) / RATE;
    const phase = phases[k] ?? 0;
    for (let i = 0; i < samples; i += 1) out[i] = (out[i] ?? 0) + amplitude * Math.sin(step * i + phase);
  }
  return out;
}

function tone(segments: Array<{ seconds: number; peakDb: number }>, hz = 1000): Float32Array {
  const total = segments.reduce((sum, segment) => sum + Math.round(segment.seconds * RATE), 0);
  const out = new Float32Array(total);
  let at = 0;
  for (const segment of segments) {
    const amplitude = 10 ** (segment.peakDb / 20);
    const end = at + Math.round(segment.seconds * RATE);
    for (let i = at; i < end; i += 1) out[i] = amplitude * Math.sin((2 * Math.PI * hz * i) / RATE);
    at = end;
  }
  return out;
}

export async function measureLatency(spec: StripSpec): Promise<number> {
  const rng = mulberry32(77);
  const input = new Float32Array(Math.round(0.4 * RATE));
  for (let i = 0; i < input.length; i += 1) input[i] = (rng() * 2 - 1) * 10 ** (-50 / 20) * Math.sqrt(3);
  const output = await throughStrip(spec, input);
  const start = 9600;
  const length = 4800;
  let best = -Infinity;
  let lag = 0;
  for (let candidate = 0; candidate <= 2400; candidate += 1) {
    let dot = 0;
    for (let i = start; i < start + length; i += 1) dot += (output[i] ?? 0) * (input[i - candidate] ?? 0);
    if (dot > best) {
      best = dot;
      lag = candidate;
    }
  }
  return lag;
}

async function response(spec: StripSpec, rmsDb: number, latency: number): Promise<string> {
  const input = multitone(rmsDb, Math.round(1.6 * RATE), 31);
  const output = await throughStrip(spec, input);
  const start = 24_000;
  const tokens = THIRD_OCTAVES.map((hz) => {
    const gain = amplitudeAt(output, start + latency, RATE, hz) / amplitudeAt(input, start, RATE, hz);
    return `${hz}:${fixed(dbfs(gain), 2)}`;
  });
  return tokens.join(" ");
}

const CURVE_LEVELS = [-60, -50, -40, -35, -30, -27, -24, -21, -18, -15, -12, -9, -6, -3, 0];

async function curve(spec: StripSpec): Promise<string> {
  const tokens: string[] = [];
  for (const level of CURVE_LEVELS) {
    const input = tone([{ seconds: 0.8, peakDb: level }]);
    const output = await throughStrip(spec, input);
    const tail = rms(output, output.length - 9600, output.length);
    tokens.push(`${level}:${fixed(dbfs(tail * Math.SQRT2), 2)}`);
  }
  return tokens.join(" ");
}

/** dB of the tone's peak, per 1 ms (one cycle of 1 kHz). */
function envelope(output: Float32Array, fromMs: number, toMs: number, latency: number): number[] {
  const values: number[] = [];
  for (let ms = fromMs; ms < toMs; ms += 1) {
    const start = ms * 48 + latency;
    values.push(dbfs(rms(output, start, start + 48) * Math.SQRT2));
  }
  return values;
}

function settleMs(values: number[], from: number, target: number): number {
  // One past the last millisecond still more than 1 dB from the settled level.
  let last = from - 1;
  for (let i = from; i < values.length; i += 1) if (Math.abs((values[i] ?? 0) - target) > 1) last = i;
  return last - from + 1;
}

function mean(values: number[]): number {
  return values.reduce((sum, value) => sum + value, 0) / Math.max(1, values.length);
}

async function steps(spec: StripSpec, latency: number): Promise<Array<[string, string]>> {
  const input = tone([
    { seconds: 0.6, peakDb: -40 },
    { seconds: 0.6, peakDb: -6 },
    { seconds: 1.2, peakDb: -40 },
  ]);
  const output = await throughStrip(spec, input);
  const up = 600;
  const down = 1200;
  const attack = envelope(output, up - 10, up + 200, latency);
  const release = envelope(output, down - 10, down + 800, latency);
  const high = mean(envelope(output, up + 400, up + 600, latency));
  const low = mean(envelope(output, down + 1000, down + 1150, latency));
  const overshoot = Math.max(...attack.slice(10)) - high;
  const encode = (values: number[]) => encodeGains(values.map((value): GainValue => (value <= -60 ? "x" : value)));
  return [
    ["attack", encode(attack)],
    ["release", encode(release)],
    ["settle", `attack-ms:${settleMs(attack, 10, high)} release-ms:${settleMs(release, 10, low)}`],
    ["overshoot", `attack-db:${fixed(overshoot, 2)}`],
  ];
}

/** Every measurement of one strip configuration, as record lines. */
export async function characterise(spec: StripSpec): Promise<Array<[string, string]>> {
  const latency = await measureLatency(spec);
  return [
    ["latency", String(latency)],
    ["response-low", await response(spec, -60, latency)],
    ["response-hot", await response(spec, -20, latency)],
    ["curve", await curve(spec)],
    ...(await steps(spec, latency)),
  ];
}

/** Level statistics of a real signal after the strip, over 50 ms blocks. */
export function levelStats(output: Float32Array): string {
  const blocks: number[] = [];
  for (let start = 0; start + 2400 <= output.length; start += 2400) {
    const level = dbfs(rms(output, start, start + 2400));
    if (level > -70) blocks.push(level);
  }
  blocks.sort((a, b) => a - b);
  const pick = (fraction: number) => blocks[Math.min(blocks.length - 1, Math.floor(blocks.length * fraction))] ?? -200;
  let peak = 0;
  for (const value of output) peak = Math.max(peak, Math.abs(value));
  return [
    `rms:${fixed(dbfs(rms(output)), 2)}`,
    `peak:${fixed(dbfs(peak), 2)}`,
    `p50:${fixed(pick(0.5), 2)}`,
    `p90:${fixed(pick(0.9), 2)}`,
    `p99:${fixed(pick(0.99), 2)}`,
  ].join(" ");
}
