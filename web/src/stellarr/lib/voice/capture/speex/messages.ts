/**
 * What crosses between the Speex worklet and the main thread: the name it
 * registers under and the options it is built with. The one message it takes
 * is the "destroy" that noiseSuppression.ts sends every engine.
 */

/** The name the worklet registers its processor under. */
export const SPEEX_PROCESSOR_NAME = "stellarr-speex";

/** `processorOptions` of the Speex node: speex.wasm, compiled on the render thread. */
export interface SpeexOptions {
  wasmBinary: ArrayBuffer;
}
