/**
 * What crosses between the detector worklet and the main thread: the name it
 * registers under, the options it is built with, the messages the main thread
 * sends it, and the readings it posts back, which `VadPublisher` decides on.
 */
import { clamp } from "./frame";
import type { VadConfig } from "./vad";

/** The name the worklet registers its processor under. */
export const VAD_PROCESSOR_NAME = "stellarr-vad";

/**
 * `processorOptions` of the detector node. Any `VadConfig` field sets the
 * detector; the rest choose the stages and how much it posts.
 */
export interface DetectorOptions extends Partial<VadConfig> {
  /** Post levels while nobody speaks (the local meter does; remote rings need not). */
  publishSilentLevels?: boolean;
  /** Build the gate: the input comes out delayed and gated. */
  gate?: boolean;
  /** false starts the gate passing everything. */
  gateEnabled?: boolean;
  /** Build the leveller, applied after the gate. */
  leveller?: boolean;
  /** false starts the leveller at unity. */
  levellerEnabled?: boolean;
  lookaheadMs?: number;
  attackMs?: number;
  releaseMs?: number;
  closedGain?: number;
}

/** Messages the main thread posts to the detector. */
export type DetectorCommand =
  | { type: "reset" }
  | { type: "stop" }
  | { type: "config"; config?: Partial<Record<keyof VadConfig, unknown>> }
  | { type: "gate"; enabled?: boolean }
  | { type: "leveller"; enabled?: boolean };

/** What the detector posts to the main thread. */
export interface SpeechPost {
  speaking: boolean;
  /** 0..1, in whole steps of 1 / LEVEL_STEPS. */
  level: number;
}

/** Level steps the UI meter can actually draw: 3 bars, so 8 steps is ample. */
export const LEVEL_STEPS = 8;

/**
 * Decides when a frame result is worth a `postMessage`.
 *
 * The detector produces 100 results a second; the UI can act on far fewer.
 * A speaking flip crosses the thread immediately because it changes what is on
 * screen, while level changes are quantised and rate-limited. Without this the
 * audio thread would wake the main thread 100 times a second to redraw three
 * bars that only have 8 distinct states.
 */
export class VadPublisher {
  private readonly minFramesBetweenLevelPosts: number;
  private readonly publishSilentLevels: boolean;
  private lastSpeaking = false;
  private lastLevelStep = -1;
  private framesSincePost = 0;

  constructor(options: { minFramesBetweenLevelPosts?: number; publishSilentLevels?: boolean } = {}) {
    // Ten 10 ms frames cap meter traffic at 10 Hz. CSS interpolates the bars,
    // while the main thread avoids 25 updates per second for every participant.
    this.minFramesBetweenLevelPosts = options.minFramesBetweenLevelPosts ?? 10;
    this.publishSilentLevels = options.publishSilentLevels ?? true;
  }

  /** The payload to post for this frame, or null. */
  consider(result: SpeechPost): SpeechPost | null {
    this.framesSincePost += 1;
    const step = Math.round(clamp(result.level, 0, 1) * LEVEL_STEPS);
    const speakingChanged = result.speaking !== this.lastSpeaking;
    const levelChanged =
      (this.publishSilentLevels || result.speaking) &&
      step !== this.lastLevelStep &&
      this.framesSincePost >= this.minFramesBetweenLevelPosts;

    if (!speakingChanged && !levelChanged) return null;

    this.lastSpeaking = result.speaking;
    // A remote strip does not need to redraw room noise while nobody speaks.
    // Release still publishes one explicit zero so a meter never stays raised.
    this.lastLevelStep = result.speaking || this.publishSilentLevels ? step : 0;
    this.framesSincePost = 0;
    return {
      speaking: result.speaking,
      level: this.lastLevelStep / LEVEL_STEPS,
    };
  }

  reset(): void {
    this.lastSpeaking = false;
    this.lastLevelStep = -1;
    this.framesSincePost = 0;
  }
}
