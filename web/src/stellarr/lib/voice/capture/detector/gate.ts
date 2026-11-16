import { clamp } from "./frame";

export interface GateConfig {
  sampleRate: number;
  lookaheadMs: number;
  attackMs: number;
  releaseMs: number;
  closedGain: number;
}

export const GATE_DEFAULTS: Readonly<Omit<GateConfig, "sampleRate">> = Object.freeze({
  /**
   * The detector needs `attackFrames` (30 ms) to confirm an onset, and it
   * confirms on the first *vowel*: a word that starts with "s", "sz" or "f"
   * has been making sound for 50–80 ms by then. Output is delayed by that
   * much, so opening the gate on confirmation reaches back to the consonant
   * instead of clipping it, which is the whole reason a plain VAD-as-gate
   * sounds chopped. 80 ms on top of the call's latency is audible only as a
   * fraction of the jitter buffer everyone already lives with.
   */
  lookaheadMs: 80,
  /** Fast enough to feel instant, slow enough to avoid a click. */
  attackMs: 6,
  /** Long enough that word endings fade rather than snap off. */
  releaseMs: 90,
  /** Fully closed: what the other side hears between your sentences is nothing. */
  closedGain: 0,
});

function coefficientFor(ms: number, sampleRate: number): number {
  const samples = (Math.max(0, ms) * sampleRate) / 1000;
  if (samples < 1) return 1;
  return 1 - Math.exp(-1 / samples);
}

/**
 * Sample-accurate gain follower with a lookahead delay: the input gate.
 *
 * It allocates once, in the constructor; `process` never allocates.
 */
export class GateSmoother {
  readonly delaySamples: number;
  readonly closedGain: number;
  gain: number;
  enabled = true;
  open = false;
  private readonly buffer: Float32Array;
  private position = 0;
  private readonly attackCoefficient: number;
  private readonly releaseCoefficient: number;

  constructor(options: Partial<GateConfig> & { sampleRate: number }) {
    const config = { ...GATE_DEFAULTS, ...options };
    const sampleRate = Number.isFinite(config.sampleRate) && config.sampleRate > 0 ? config.sampleRate : 48000;
    this.closedGain = clamp(Number.isFinite(config.closedGain) ? config.closedGain : 0, 0, 1);
    this.delaySamples = Math.max(0, Math.round((Math.max(0, config.lookaheadMs) * sampleRate) / 1000));
    this.buffer = new Float32Array(Math.max(1, this.delaySamples));
    this.attackCoefficient = coefficientFor(config.attackMs, sampleRate);
    this.releaseCoefficient = coefficientFor(config.releaseMs, sampleRate);
    this.gain = this.closedGain;
  }

  /** The gain the follower is heading towards right now. */
  get target(): number {
    return this.enabled && !this.open ? this.closedGain : 1;
  }

  setOpen(open: boolean): void {
    this.open = Boolean(open);
  }

  /**
   * A disabled gate passes everything but keeps its delay, so toggling it
   * mid-call never shifts the audio in time.
   */
  setEnabled(enabled: boolean): void {
    this.enabled = Boolean(enabled);
  }

  reset(): void {
    this.buffer.fill(0);
    this.position = 0;
    this.gain = this.target;
  }

  /** `output` may be the same array as `input`. */
  process(input: Float32Array, output: Float32Array, length = input.length): void {
    const target = this.target;
    const coefficient = target > this.gain ? this.attackCoefficient : this.releaseCoefficient;
    let gain = this.gain;
    if (this.delaySamples === 0) {
      for (let i = 0; i < length; i += 1) {
        gain += (target - gain) * coefficient;
        output[i] = (input[i] as number) * gain;
      }
    } else {
      const buffer = this.buffer;
      const size = this.delaySamples;
      let position = this.position;
      for (let i = 0; i < length; i += 1) {
        const delayed = buffer[position] as number;
        buffer[position] = input[i] as number;
        position += 1;
        if (position === size) position = 0;
        gain += (target - gain) * coefficient;
        output[i] = delayed * gain;
      }
      this.position = position;
    }
    // Snap once the ramp is inaudibly close, so a closed gate is truly silent
    // rather than a denormal-speed tail of -100 dB residue.
    this.gain = Math.abs(target - gain) < 1e-4 ? target : gain;
  }
}
