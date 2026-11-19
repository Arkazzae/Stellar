/**
 * SpeexDSP's preprocessor, driven through the exports of speex.wasm, which
 * artifacts/recipes/speex/ builds. The worklet's processor (`processor.ts`)
 * is a thin shell around this, and the tests drive the real binary through it
 * in Node.
 *
 * The binary imports nothing and needs no generated JavaScript: it is
 * instantiated with an empty import object, and everything below is a plain
 * call into its exports.
 *
 * What makes Speex sound as it does is kept from the engine this replaced,
 * @sapphi-red/speex-preprocess-wasm 0.4.0 under web-noise-suppressor 0.4.1,
 * including the parts that look odd: the frame is one render quantum (128
 * samples, 2.7 ms at 48 kHz) where SpeexDSP's time constants assume 20 ms,
 * denoise is on and every other setting is the library's default, and the
 * samples cross to int16 and back as that engine moved them. See `run`.
 */

/** What speex.wasm exports: its memory, an allocator and the preprocessor's API. */
export interface SpeexExports {
  readonly memory: WebAssembly.Memory;
  malloc(size: number): number;
  free(pointer: number): void;
  speex_preprocess_state_init(frameSize: number, samplingRate: number): number;
  speex_preprocess_state_destroy(state: number): void;
  speex_preprocess_run(state: number, frame: number): number;
  speex_preprocess_ctl(state: number, request: number, pointer: number): number;
}

/** One instance of the binary. Each holds its own memory, and any number of states. */
export function instantiateSpeex(module: WebAssembly.Module): SpeexExports {
  return new WebAssembly.Instance(module, {}).exports as unknown as SpeexExports;
}

/** From speex/speex_preprocess.h. The value it reads is an int behind a pointer. */
const SPEEX_PREPROCESS_SET_DENOISE = 0;

/** int16 full scale as the replaced engine took it, both ways: 2^15 - 1, not 2^15. */
const INT16_SCALE = 32767;

const INT16_BYTES = 2;
const INT32_BYTES = 4;

export class SpeexPreprocessor {
  private state: number;
  /** Where the frame lives in the module's memory, as int16 samples. */
  private readonly frame: number;
  /** A view of the frame; made again when the memory grows, which detaches the old one. */
  private samples: Int16Array;

  constructor(
    private readonly speex: SpeexExports,
    readonly frameSize: number,
    sampleRate: number,
  ) {
    this.state = speex.speex_preprocess_state_init(frameSize, sampleRate);
    this.frame = speex.malloc(frameSize * INT16_BYTES);
    const flag = speex.malloc(INT32_BYTES);
    if (this.state === 0 || this.frame === 0 || flag === 0) {
      speex.free(flag);
      this.destroy();
      throw new Error("speex.wasm is out of memory");
    }
    // A new state already denoises; the replaced engine set it all the same,
    // and so does this.
    new Int32Array(speex.memory.buffer, flag, 1)[0] = 1;
    speex.speex_preprocess_ctl(this.state, SPEEX_PREPROCESS_SET_DENOISE, flag);
    speex.free(flag);
    this.samples = new Int16Array(speex.memory.buffer, this.frame, frameSize);
  }

  /**
   * Cleans one frame of `frameSize` samples from `input` into `output`.
   * Nothing is allocated unless the module's memory has grown since the last
   * frame, which a running state never makes it do.
   *
   * The conversion is the replaced engine's, step for step: each sample times
   * 32767, stored into an Int16Array, which truncates towards zero and wraps
   * past the int16 range instead of clamping (1.1 comes back as about -0.9);
   * then each int16 divided by 32767. A sample outside [-1, 1] only reaches
   * this with input gain above 0 dB, and wrapping is what Speex has always
   * done with it here.
   */
  run(input: Float32Array, output: Float32Array): void {
    if (this.state === 0) throw new Error("the Speex state was destroyed");
    let samples = this.samples;
    if (samples.buffer !== this.speex.memory.buffer) {
      samples = new Int16Array(this.speex.memory.buffer, this.frame, this.frameSize);
      this.samples = samples;
    }
    const size = this.frameSize;
    for (let i = 0; i < size; i += 1) samples[i] = (input[i] as number) * INT16_SCALE;
    this.speex.speex_preprocess_run(this.state, this.frame);
    for (let i = 0; i < size; i += 1) output[i] = (samples[i] as number) / INT16_SCALE;
  }

  /** Frees the state and the frame. Idempotent; `run` refuses afterwards. */
  destroy(): void {
    if (this.state !== 0) this.speex.speex_preprocess_state_destroy(this.state);
    this.speex.free(this.frame);
    this.state = 0;
  }
}
