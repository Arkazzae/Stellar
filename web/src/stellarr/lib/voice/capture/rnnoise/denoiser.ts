/**
 * Xiph's RNNoise, driven through the exports of rnnoise.wasm, which
 * artifacts/recipes/rnnoise/ builds with the model compiled in. The
 * worklet's processor (`processor.ts`) is a thin shell around this, and the
 * tests drive the real binary through it in Node.
 *
 * The binary imports nothing and needs no generated JavaScript: it is
 * instantiated with an empty import object, and everything below is a plain
 * call into its exports.
 *
 * The library takes frames of 480 samples (10 ms at 48 kHz) as floats in the
 * int16 range; the render thread hands over 128 at a time. Samples are
 * gathered into a frame inside the module's memory, the frame is cleaned
 * there in place as soon as it is whole, and what comes out waits in a ring
 * until the render quanta ask for it. See `BUFFERING` and `LATENCY`.
 */

/** What rnnoise.wasm exports: its memory, an allocator and the library's API. */
export interface RnnoiseExports {
  readonly memory: WebAssembly.Memory;
  malloc(size: number): number;
  free(pointer: number): void;
  /** A state for the model compiled in, given a null model; 0 when out of memory. */
  rnnoise_create(model: number): number;
  rnnoise_destroy(state: number): void;
  /** Cleans one frame from `input` into `output`, which may be the same; returns a voice probability. */
  rnnoise_process_frame(state: number, output: number, input: number): number;
}

/** One instance of the binary. Each holds its own memory, the model's weights among it, and any number of states. */
export function instantiateRnnoise(module: WebAssembly.Module): RnnoiseExports {
  return new WebAssembly.Instance(module, {}).exports as unknown as RnnoiseExports;
}

/** The library's frame: 10 ms at 48 kHz, the only rate it is built for. */
export const FRAME = 480;

/** One render quantum, what the processor is given each time. */
export const QUANTUM = 128;

/**
 * Samples of delay the framing adds, the smallest that never leaves a
 * quantum short. A frame is cleaned in the quantum that completes it, so at
 * the end of quantum k (128 * (k + 1) samples in) the cleaned output reaches
 * as far as the last whole frame. 480 is not a multiple of 128, and the
 * furthest the input gets ahead of the last whole frame is 448 samples, at
 * 1408 = 2 * 480 + 448; every other quantum ends at most that far ahead.
 */
export const BUFFERING = 448;

/**
 * Samples from input to output: the framing, then the library's own two
 * frames. A frame is windowed together with the one before it, and the gains
 * the network computes from a frame are applied to the frame before, so
 * the library gives back the frame it was handed two calls ago.
 */
export const LATENCY = BUFFERING + 2 * FRAME;

/** int16 full scale, both ways: the library's features are tuned to that range. */
const INT16_SCALE = 32768;

const FLOAT_BYTES = 4;

/** The ring of cleaned samples: room for the delay and one more frame, rounded up to a power of two. */
const RING = 1024;

export class RnnoiseDenoiser {
  private state: number;
  /** Where the frame lives in the module's memory, as floats. */
  private frame: number;
  /** A view of the frame; made again when the memory grows, which detaches the old one. */
  private samples: Float32Array;
  /** How many samples of the next frame are in place. */
  private filled = 0;
  private readonly ring = new Float32Array(RING);
  private written = BUFFERING;
  private read = 0;

  constructor(private readonly rnnoise: RnnoiseExports) {
    this.state = rnnoise.rnnoise_create(0);
    this.frame = rnnoise.malloc(FRAME * FLOAT_BYTES);
    if (this.state === 0 || this.frame === 0) {
      this.destroy();
      throw new Error("rnnoise.wasm is out of memory");
    }
    this.samples = new Float32Array(rnnoise.memory.buffer, this.frame, FRAME);
  }

  /**
   * Cleans one render quantum, `QUANTUM` samples of `input`, into `output`,
   * `LATENCY` samples late;
   * the first `LATENCY` samples out are silence. Nothing is allocated unless
   * the module's memory has grown since the last call, which a running state
   * never makes it do.
   *
   * A sample past full scale, which input gain above 0 dB can make, is
   * clamped to the int16 range on its way in, as Speex's is: the network's
   * features were learnt on int16 audio, and nothing it was trained on lies
   * beyond it.
   */
  run(input: Float32Array, output: Float32Array): void {
    if (this.state === 0) throw new Error("the RNNoise state was destroyed");
    let samples = this.samples;
    if (samples.buffer !== this.rnnoise.memory.buffer) {
      samples = new Float32Array(this.rnnoise.memory.buffer, this.frame, FRAME);
      this.samples = samples;
    }
    const ring = this.ring;
    const size = QUANTUM;
    let filled = this.filled;
    for (let i = 0; i < size; i += 1) {
      const scaled = (input[i] as number) * INT16_SCALE;
      samples[filled] = scaled > 32767 ? 32767 : scaled < -32768 ? -32768 : scaled;
      filled += 1;
      if (filled === FRAME) {
        this.rnnoise.rnnoise_process_frame(this.state, this.frame, this.frame);
        let at = this.written;
        for (let j = 0; j < FRAME; j += 1) {
          ring[at] = (samples[j] as number) / INT16_SCALE;
          at = (at + 1) & (RING - 1);
        }
        this.written = at;
        filled = 0;
      }
    }
    this.filled = filled;
    let at = this.read;
    for (let i = 0; i < size; i += 1) {
      output[i] = ring[at] as number;
      at = (at + 1) & (RING - 1);
    }
    this.read = at;
  }

  /** Frees the state and the frame. Idempotent; `run` refuses afterwards. */
  destroy(): void {
    if (this.state !== 0) this.rnnoise.rnnoise_destroy(this.state);
    if (this.frame !== 0) this.rnnoise.free(this.frame);
    this.state = 0;
    this.frame = 0;
  }
}
