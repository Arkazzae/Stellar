/**
 * Neural and DSP noise suppressors for the microphone path.
 *
 * Three engines, each a WASM binary driven by an AudioWorklet processor on the
 * render thread, so none of them competes with React for the main thread. The
 * processors and binaries are prebuilt files from web-noise-suppressor and the
 * projects it builds on, served from /audio/ (see artifacts/manifest.json);
 * this module is the main-thread side that loads and starts them. Everything
 * is loaded lazily: the binaries weigh 150-200 KB and most sessions ask for
 * exactly one.
 *
 * - `rnnoise`: Xiph's RNNoise. Cheap and good on steady background noise, weak
 *   on impulsive sounds such as keys or chewing.
 * - `gtcrn`: a larger network with a far better record on non-stationary noise,
 *   at a higher CPU cost. It runs at 48 kHz natively, so it never band-limits
 *   the voice.
 * - `speex`: SpeexDSP's `preprocess`, classic spectral subtraction. The lightest
 *   option and the least effective.
 *
 * Everything here degrades instead of failing. When the worklet, the WASM or the
 * node constructor gives out, `attachSuppressor` resolves to null and the
 * caller keeps the browser's own suppressor: a person who picked "GTCRN" and got
 * browser suppression is mildly disappointed, one whose call would not start
 * because a download failed is not on the call at all.
 */

import type { WorkletSuppressor } from "./audioProfiles";

export const RNNOISE_WORKLET_URL = "/audio/rnnoise-worklet.js";
export const RNNOISE_WASM_URL = "/audio/rnnoise.wasm";
export const RNNOISE_SIMD_WASM_URL = "/audio/rnnoise_simd.wasm";
export const GTCRN_WORKLET_URL = "/audio/gtcrn-worklet.js";
export const GTCRN_WASM_URL = "/audio/gtcrn.wasm";
export const SPEEX_WORKLET_URL = "/audio/speex-worklet.js";
export const SPEEX_WASM_URL = "/audio/speex.wasm";

export const WORKLET_URL: Record<WorkletSuppressor, string> = {
  rnnoise: RNNOISE_WORKLET_URL,
  gtcrn: GTCRN_WORKLET_URL,
  speex: SPEEX_WORKLET_URL,
};

/**
 * The name each worklet module registers its processor under. The worklets are
 * served byte for byte as they were published, so these are theirs to choose.
 */
export const PROCESSOR_NAME: Record<WorkletSuppressor, string> = {
  rnnoise: "@sapphi-red/web-noise-suppressor/rnnoise",
  gtcrn: "@sapphi-red/web-noise-suppressor/gtcrn",
  speex: "@sapphi-red/web-noise-suppressor/speex",
};

export type NoiseSuppressionStatus = "inactive" | "loading" | "active" | "unavailable";

export interface NoiseSuppressorHandle {
  /** Which engine is actually in the graph. */
  kind: WorkletSuppressor;
  /** The node carrying cleaned audio; the caller wires whatever comes next. */
  output: AudioNode;
  /** Idempotent; disconnects the worklet node and tells it to free its WASM heap. */
  dispose(): void;
}

/**
 * What every one of the three processors reads from `processorOptions`, and
 * all it reads. The binary is copied into the render thread with the options
 * and instantiated there; each of the `maxChannels` channels gets its own
 * suppressor state, and a mono microphone needs one.
 */
interface ProcessorOptions {
  maxChannels: number;
  wasmBinary: ArrayBuffer;
}

/**
 * The message a processor takes as its cue to stop processing and free its
 * WASM state. Disconnecting alone frees nothing: a processor without input
 * still reports itself alive and keeps its heap.
 *
 * The worklets as published (0.4.1) listen for it with addEventListener and
 * never start their port, which leaves its messages queued: in Chromium the
 * processors were seen not to hear it. Going by their code, their state then
 * lasts as long as the AudioContext. The capture graph closes its context on
 * every recapture; the remote mixer keeps one for the whole call. It is sent
 * regardless, so a build that starts its port needs nothing here.
 */
const DESTROY = "destroy";

/**
 * The smallest module that only an engine with WebAssembly SIMD accepts: one
 * function that splats an i32 into a vector and asks whether any lane is set.
 * Both instructions have the numbers the finished SIMD proposal gave them,
 * which is the SIMD rnnoise_simd.wasm was built for.
 */
const SIMD_PROBE = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, // "\0asm"
  0x01, 0x00, 0x00, 0x00, // version 1
  0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, // type section: () -> i32
  0x03, 0x02, 0x01, 0x00, // function section: one function of that type
  0x0a, 0x0a, 0x01, 0x08, 0x00, // code section: one body of 8 bytes, no locals
  0x41, 0x00, // i32.const 0
  0xfd, 0x0f, // i8x16.splat
  0xfd, 0x53, // v128.any_true
  0x0b, // end
]);

function supportsSimd(): boolean {
  try {
    return WebAssembly.validate(SIMD_PROBE);
  } catch {
    return false;
  }
}

const WASM_URL: Record<WorkletSuppressor, () => string> = {
  // The SIMD build is the same network, a good deal cheaper per frame.
  rnnoise: () => (supportsSimd() ? RNNOISE_SIMD_WASM_URL : RNNOISE_WASM_URL),
  gtcrn: () => GTCRN_WASM_URL,
  speex: () => SPEEX_WASM_URL,
};

/**
 * The binary, or a rejection. A response that is not a success is refused here:
 * its body would reach the worklet, fail to compile there off the main thread,
 * and leave a processor that keeps running and writes nothing, a microphone
 * gone silent instead of one falling back to the browser's suppressor.
 */
async function fetchBinary(url: string): Promise<ArrayBuffer> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url} answered ${response.status}`);
  return response.arrayBuffer();
}

function createNode(
  kind: WorkletSuppressor,
  context: AudioContext,
  wasmBinary: ArrayBuffer,
): AudioWorkletNode {
  const processorOptions: ProcessorOptions = { maxChannels: 1, wasmBinary };
  return new AudioWorkletNode(context, PROCESSOR_NAME[kind], { processorOptions });
}

/**
 * Loads are cached as promises, so two captures starting together share one
 * download. A rejected load is evicted: a transient chunk or network failure
 * should cost one capture its suppressor, not pin the page to browser
 * suppression until reload.
 */
function remember<K, V>(cache: Map<K, Promise<V>>, key: K, load: () => Promise<V>): Promise<V> {
  const cached = cache.get(key);
  if (cached) return cached;
  const created = load();
  cache.set(key, created);
  created.catch(() => {
    if (cache.get(key) === created) cache.delete(key);
  });
  return created;
}

const binaryCache = new Map<WorkletSuppressor, Promise<ArrayBuffer>>();
const registeredWorklets = new WeakMap<AudioContext, Map<WorkletSuppressor, Promise<void>>>();

function registerWorklet(kind: WorkletSuppressor, context: AudioContext): Promise<void> {
  let registered = registeredWorklets.get(context);
  if (!registered) {
    registered = new Map();
    registeredWorklets.set(context, registered);
  }
  return remember(registered, kind, () => context.audioWorklet.addModule(WORKLET_URL[kind]));
}

/** Undo whatever of the node's wiring exists; every step tolerates having nothing to do. */
function release(source: AudioNode, node: AudioWorkletNode): void {
  const steps = [
    () => source.disconnect(node),
    () => node.disconnect(),
    // Asks the processor to free its WASM heap; see DESTROY for when that
    // actually happens.
    () => node.port.postMessage(DESTROY),
  ];
  for (const step of steps) {
    try {
      step();
    } catch {
      // Already detached, or only partly constructed.
    }
  }
}

/**
 * Insert a suppressor after `source`, or resolve to null when it is unavailable.
 *
 * The caller owns `context`; this adds one node to it and hands back a disposer
 * that removes exactly what it added. The caller connects `handle.output`
 * onward, to a gate, a destination, whatever comes next.
 */
export async function attachSuppressor(
  kind: WorkletSuppressor,
  context: AudioContext,
  source: AudioNode,
): Promise<NoiseSuppressorHandle | null> {
  let node: AudioWorkletNode | null = null;
  try {
    await registerWorklet(kind, context);
    const wasmBinary = await remember(binaryCache, kind, () => fetchBinary(WASM_URL[kind]()));

    const created = createNode(kind, context, wasmBinary);
    node = created;
    source.connect(created);

    let disposed = false;
    return {
      kind,
      output: created,
      dispose() {
        if (disposed) return;
        disposed = true;
        release(source, created);
      },
    };
  } catch {
    // Construction can fail once the node exists and is partly wired; without
    // this every failed recapture would leak a WASM instance.
    if (node) release(source, node);
    return null;
  }
}

/** Test seam: forget the cached binaries between cases. */
export function resetSuppressorCacheForTests(): void {
  binaryCache.clear();
}
