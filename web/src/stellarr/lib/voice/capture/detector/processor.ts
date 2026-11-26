/**
 * The detector's AudioWorklet module: the entry the build bundles on its own
 * into /audio/detector-worklet.js (see web/worklets.ts). Nothing on the main
 * thread imports this file; the app takes the address from `address.ts`, and
 * the constants and classes from the modules beside this one.
 *
 * One processor, two jobs. Without `gate` in its options it is an observer: it
 * analyses, posts speaking and level, and outputs nothing. With `gate: true`
 * it is also the audible path: the input comes out the other side delayed by
 * the lookahead and multiplied by a gain that follows the very same detector.
 * Sharing one instance means the ring around your avatar and the sound the
 * others hear can never disagree.
 */
import { GateSmoother } from "./gate";
import { Leveller } from "./leveller";
import { VAD_PROCESSOR_NAME, VadPublisher, type DetectorCommand, type DetectorOptions } from "./messages";
import { AdaptiveVad, type VadFrameResult } from "./vad";

// The AudioWorkletGlobalScope, which TypeScript's DOM library does not describe.
declare const sampleRate: number;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}
declare function registerProcessor(
  name: string,
  processor: new (options?: AudioWorkletNodeOptions) => AudioWorkletProcessor,
): void;

class DetectorProcessor extends AudioWorkletProcessor {
  private readonly vad: AdaptiveVad;
  private readonly publisher: VadPublisher;
  private readonly gate: GateSmoother | null;
  private readonly leveller: Leveller | null;
  private readonly results: VadFrameResult[] = [];
  private alive = true;
  /** Push to talk's hold on the gate; null while the detector opens it. */
  private held: boolean | null = null;

  constructor(options?: AudioWorkletNodeOptions) {
    super();
    const processorOptions: DetectorOptions =
      options && options.processorOptions ? (options.processorOptions as DetectorOptions) : {};
    this.vad = new AdaptiveVad({ sampleRate, ...processorOptions });
    this.publisher = new VadPublisher({
      publishSilentLevels: processorOptions.publishSilentLevels !== false,
    });
    this.gate =
      processorOptions.gate === true
        ? new GateSmoother({
            sampleRate,
            ...(typeof processorOptions.lookaheadMs === "number" ? { lookaheadMs: processorOptions.lookaheadMs } : {}),
            ...(typeof processorOptions.attackMs === "number" ? { attackMs: processorOptions.attackMs } : {}),
            ...(typeof processorOptions.releaseMs === "number" ? { releaseMs: processorOptions.releaseMs } : {}),
            ...(typeof processorOptions.closedGain === "number" ? { closedGain: processorOptions.closedGain } : {}),
          })
        : null;
    if (this.gate && processorOptions.gateEnabled === false) {
      this.gate.setEnabled(false);
      this.gate.reset();
    }
    // Automatic volume rides on the same detector: it is told which frames
    // were vowels, and it is applied after the gate so the two never fight.
    this.leveller = processorOptions.leveller === true ? new Leveller({ sampleRate }) : null;
    if (this.leveller && processorOptions.levellerEnabled === false) {
      this.leveller.setEnabled(false);
    }
    this.port.onmessage = (event: MessageEvent<DetectorCommand | null>) => {
      const data = event.data || ({} as Partial<DetectorCommand>);
      if (data.type === "reset") {
        this.vad.reset();
        this.publisher.reset();
        this.gate?.reset();
        this.leveller?.reset();
      } else if (data.type === "stop") {
        this.alive = false;
      } else if (data.type === "config") {
        // Applied in place: rebuilding the node would drop the learned noise
        // floor and force another warm-up mid-sentence.
        this.vad.configure(data.config || {});
      } else if (data.type === "gate") {
        this.gate?.setEnabled(data.enabled !== false);
      } else if (data.type === "leveller") {
        this.leveller?.setEnabled(data.enabled !== false);
      } else if (data.type === "hold") {
        this.held = typeof data.open === "boolean" ? data.open : null;
      }
    };
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    if (!this.alive) return false;
    const input = inputs[0];
    const channel = input && input[0];
    // A disconnected or not-yet-flowing source yields an empty array. Keep the
    // node alive and wait rather than treating the gap as silence.
    if (!channel || channel.length === 0) return true;

    const results = this.vad.push(channel, this.results);
    for (let i = 0; i < results.length; i += 1) {
      const result = results[i] as VadFrameResult;
      this.leveller?.observe(result);
      const payload = this.publisher.consider(result);
      if (payload) this.port.postMessage(payload);
    }

    if (this.gate || this.leveller) {
      const output = outputs[0] && outputs[0][0];
      if (output) {
        if (this.gate) {
          // `speaking` includes the hangover, so pauses between words stay
          // open; nothing voiceless ever sets it, so a crunch stays shut.
          // Push to talk, when it holds the gate, decides instead.
          this.gate.setOpen(this.held ?? this.vad.speaking);
          this.gate.process(channel, output, channel.length);
        } else {
          output.set(channel);
        }
        if (this.leveller) this.leveller.process(output, output, channel.length);
      }
    }
    return true;
  }
}

registerProcessor(VAD_PROCESSOR_NAME, DetectorProcessor);
