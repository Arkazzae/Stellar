/**
 * Runs `vad-processor.js` the way an AudioWorklet runs it.
 *
 * The module registers its processor only when it finds
 * `AudioWorkletProcessor` and `registerProcessor` in its global scope, and the
 * processor reads a free `sampleRate`. Here those three are put on the global
 * object, a separate copy of the module is evaluated (the query string makes
 * it a module of its own, so the copy the app's code imports, which skipped
 * the block, is not reused), and the class it registers is kept. The
 * processor is then driven in render quanta of 128 frames, with messages
 * from the main thread delivered between quanta and its own `postMessage`
 * calls collected with the quantum they were made in.
 *
 * This is the only file of the harness that knows how the worklet is built.
 * Everything recorded comes out of `process()` and the port: the output
 * samples and the messages. No field of the processor is read.
 */
import { QUANTUM, SAMPLE_RATE } from "../dsp";

export interface ScopeMessageEvent {
  data: unknown;
}

/** The processor's end of its MessagePort. */
class ScopePort {
  onmessage: ((event: ScopeMessageEvent) => void) | null = null;
  readonly sent: unknown[] = [];

  postMessage(message: unknown): void {
    this.sent.push(message);
  }
}

interface ProcessorInstance {
  readonly port: ScopePort;
  process(
    inputs: Float32Array[][],
    outputs: Float32Array[][],
    parameters: Record<string, Float32Array>,
  ): boolean;
}

export type ProcessorClass = new (options: {
  numberOfInputs: number;
  numberOfOutputs: number;
  outputChannelCount: number[];
  processorOptions: Record<string, unknown>;
}) => ProcessorInstance;

export interface RegisteredProcessor {
  name: string;
  Processor: ProcessorClass;
}

let registration: Promise<RegisteredProcessor> | null = null;

/** The processor class `vad-processor.js` registers, evaluated in a worklet-like scope. */
export function registeredProcessor(): Promise<RegisteredProcessor> {
  registration ??= (async () => {
    const scope = globalThis as Record<string, unknown>;
    const saved = {
      AudioWorkletProcessor: scope.AudioWorkletProcessor,
      registerProcessor: scope.registerProcessor,
    };
    const registered: RegisteredProcessor[] = [];
    scope.AudioWorkletProcessor = class AudioWorkletProcessor {
      readonly port = new ScopePort();
    };
    scope.registerProcessor = (name: string, Processor: ProcessorClass) => {
      registered.push({ name, Processor });
    };
    try {
      // @ts-expect-error The query names a separate copy of the module, which TypeScript cannot resolve.
      await import("../../../public/audio/vad-processor.js?worklet-scope");
    } finally {
      for (const [key, value] of Object.entries(saved)) {
        if (value === undefined) delete scope[key];
        else scope[key] = value;
      }
    }
    const [first] = registered;
    if (registered.length !== 1 || !first) {
      throw new Error(`vad-processor.js registered ${registered.length} processors, expected one`);
    }
    return first;
  })();
  return registration;
}

/** Something the main thread posts to the node's port, before quantum `quantum`. */
export interface Delivery {
  quantum: number;
  message: unknown;
}

export interface Post {
  /** The render quantum during which the processor posted it. */
  quantum: number;
  data: unknown;
}

export interface ProcessorRun {
  posts: Post[];
  /** Silence where the processor wrote nothing, as a browser would play it. */
  output: Float32Array;
  /** False when the processor never wrote its output buffer. */
  wroteOutput: boolean;
  /** The first quantum whose `process()` returned false, or null. */
  stoppedAt: number | null;
}

/**
 * One processor over `input`, mono, in render quanta. `input.length` must be
 * a multiple of 128.
 */
export function runProcessor(
  Processor: ProcessorClass,
  processorOptions: Record<string, unknown>,
  input: Float32Array,
  deliveries: readonly Delivery[] = [],
): ProcessorRun {
  if (input.length % QUANTUM !== 0) throw new Error("input is not a whole number of quanta");
  const scope = globalThis as Record<string, unknown>;
  const savedRate = scope.sampleRate;
  scope.sampleRate = SAMPLE_RATE;
  let processor: ProcessorInstance;
  try {
    processor = new Processor({
      numberOfInputs: 1,
      numberOfOutputs: 1,
      outputChannelCount: [1],
      processorOptions,
    });
  } finally {
    if (savedRate === undefined) delete scope.sampleRate;
    else scope.sampleRate = savedRate;
  }

  const port = processor.port;
  const posts: Post[] = [];
  const output = new Float32Array(input.length);
  const pending = [...deliveries].sort((a, b) => a.quantum - b.quantum);
  const parameters: Record<string, Float32Array> = {};
  let wroteOutput = false;
  let stoppedAt: number | null = null;
  const quanta = input.length / QUANTUM;
  for (let quantum = 0; quantum < quanta; quantum += 1) {
    while (pending.length > 0 && (pending[0]?.quantum ?? Infinity) <= quantum) {
      const delivery = pending.shift();
      port.onmessage?.({ data: delivery?.message });
    }
    const from = quantum * QUANTUM;
    const channel = input.subarray(from, from + QUANTUM);
    const out = output.subarray(from, from + QUANTUM);
    // A sentinel shows whether the processor wrote the buffer; a browser
    // plays silence where it did not.
    out.fill(Number.NaN);
    const before = port.sent.length;
    const alive = processor.process([[channel]], [[out]], parameters);
    if (Number.isNaN(out[0])) out.fill(0);
    else wroteOutput = true;
    for (let i = before; i < port.sent.length; i += 1) posts.push({ quantum, data: port.sent[i] });
    if (!alive && stoppedAt === null) stoppedAt = quantum;
  }
  return { posts, output, wroteOutput, stoppedAt };
}
