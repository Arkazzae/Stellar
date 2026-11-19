/**
 * The RNNoise worklet: the entry the build bundles on its own into
 * /audio/rnnoise-worklet.js (see web/worklets.ts). Nothing on the main
 * thread imports this file; noiseSuppression.ts takes the address from
 * `address.ts` and the name and messages from `messages.ts`.
 *
 * It cleans the first channel of its input into the first channel of its
 * output, one render quantum at a time, and is mono: every microphone the app
 * opens is. The binary arrives compiled in the options; the constructor
 * instantiates it, which copies the model's weights into a memory of its
 * own, about a millisecond for the full model, and tells the main thread it
 * is ready. Every quantum from the first is cleaned.
 */
import { RNNOISE_PROCESSOR_NAME, RNNOISE_READY, type RnnoiseOptions } from "./messages";
import { RnnoiseDenoiser, instantiateRnnoise } from "./denoiser";

// The AudioWorkletGlobalScope, which TypeScript's DOM library does not describe.
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}
declare function registerProcessor(
  name: string,
  processor: new (options: AudioWorkletNodeOptions) => AudioWorkletProcessor,
): void;

class RnnoiseProcessor extends AudioWorkletProcessor {
  private denoiser: RnnoiseDenoiser | null;

  constructor(options: AudioWorkletNodeOptions) {
    super();
    const { module } = options.processorOptions as RnnoiseOptions;
    // A module that will not instantiate throws here, and the node fires
    // "processorerror" instead of the message below.
    this.denoiser = new RnnoiseDenoiser(instantiateRnnoise(module));
    // Assigning onmessage starts the port, which addEventListener alone
    // would not: the message has to arrive for the state to be freed.
    this.port.onmessage = (event: MessageEvent<unknown>) => {
      if (event.data !== "destroy") return;
      this.denoiser?.destroy();
      this.denoiser = null;
    };
    this.port.postMessage(RNNOISE_READY);
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const denoiser = this.denoiser;
    // Destroyed: returning false lets the node go, and it outputs silence.
    if (denoiser === null) return false;
    const input = inputs[0]?.[0];
    const output = outputs[0]?.[0];
    // Nothing connected yet: the output stays silent and the state untouched.
    if (input === undefined || output === undefined) return true;
    denoiser.run(input, output);
    return true;
  }
}

registerProcessor(RNNOISE_PROCESSOR_NAME, RnnoiseProcessor);
