/**
 * What crosses between the RNNoise worklet and the main thread: the name it
 * registers under, the options it is built with, and the one message it
 * sends, once its constructor has made the denoiser. The one message it takes
 * is the "destroy" that noiseSuppression.ts sends every engine.
 */

/** The name the worklet registers its processor under. */
export const RNNOISE_PROCESSOR_NAME = "stellarr-rnnoise";

/**
 * `processorOptions` of the RNNoise node: rnnoise.wasm, already compiled.
 * The binary is several megabytes, and compiling it on the render thread
 * would hold up every node of the context while it ran; the main thread
 * compiles it off any thread that renders, and the processor only
 * instantiates it.
 */
export interface RnnoiseOptions {
  module: WebAssembly.Module;
}

/**
 * Posted by the processor when its denoiser exists. A constructor that throws
 * posts nothing, and the node fires "processorerror" instead.
 */
export const RNNOISE_READY = "ready";
