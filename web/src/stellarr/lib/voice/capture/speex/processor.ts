/**
 * The Speex worklet: the entry the build bundles on its own into
 * /audio/speex-worklet.js (see web/worklets.ts). Nothing on the main thread
 * imports this file; noiseSuppression.ts takes the address from `address.ts`
 * and the name from `messages.ts`.
 *
 * It cleans the first channel of its input into the first channel of its
 * output, one render quantum at a time, and is mono: every microphone the app
 * opens is. The binary comes in the options and is compiled here, at
 * construction, so the first quantum is already cleaned.
 */
import { SPEEX_PROCESSOR_NAME, type SpeexOptions } from "./messages";
import { SpeexPreprocessor, instantiateSpeex } from "./preprocessor";

// The AudioWorkletGlobalScope, which TypeScript's DOM library does not describe.
declare const sampleRate: number;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}
declare function registerProcessor(
  name: string,
  processor: new (options: AudioWorkletNodeOptions) => AudioWorkletProcessor,
): void;

/** One render quantum, the frame the preprocessor has always been given here. */
const FRAME = 128;

class SpeexProcessor extends AudioWorkletProcessor {
  private preprocessor: SpeexPreprocessor | null;

  constructor(options: AudioWorkletNodeOptions) {
    super();
    const { wasmBinary } = options.processorOptions as SpeexOptions;
    const speex = instantiateSpeex(new WebAssembly.Module(wasmBinary));
    this.preprocessor = new SpeexPreprocessor(speex, FRAME, sampleRate);
    // Assigning onmessage starts the port, which addEventListener alone
    // would not: the message has to arrive for the state to be freed.
    this.port.onmessage = (event: MessageEvent<unknown>) => {
      if (event.data !== "destroy") return;
      this.preprocessor?.destroy();
      this.preprocessor = null;
    };
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const preprocessor = this.preprocessor;
    // Destroyed: returning false lets the node go, and it outputs silence.
    if (preprocessor === null) return false;
    const input = inputs[0]?.[0];
    const output = outputs[0]?.[0];
    // Nothing connected yet: the output stays silent and the state untouched.
    if (input === undefined || output === undefined) return true;
    preprocessor.run(input, output);
    return true;
  }
}

registerProcessor(SPEEX_PROCESSOR_NAME, SpeexProcessor);
