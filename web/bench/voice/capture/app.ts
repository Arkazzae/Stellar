/**
 * The detector nodes the app builds, built by the app's own code.
 *
 * The record should not restate what the app passes to the worklet; it should
 * take it from the app. So each configuration here calls the function the
 * app calls (`buildCaptureGraph` for the microphone, `SpeechDetector` for the
 * meter and for remote participants, `buildGateNode` at the receiver's
 * sensitivity for a gated remote strip) against a stand-in AudioContext, and
 * keeps the `AudioWorkletNode` it constructs: its processor options, and every
 * message the app posts to its port afterwards (`config` when the dial moves,
 * `stop` on teardown), which the driver hands to the processor.
 */
import { buildCaptureGraph, type CaptureGraph } from "@/lib/voice/capture/captureGraph";
import { buildGateNode } from "@/lib/voice/capture/gateNode";
import { SpeechDetector } from "@/lib/voice/capture/vad";
import { REMOTE_GATE_SENSITIVITY } from "@/lib/voice/playback/remoteMixer";
import { DEFAULT_SETTINGS, type VoiceSettings } from "@/lib/voice/settings";
import { SAMPLE_RATE } from "../dsp";

/** Stands in for `AudioWorkletNode` while the app builds a graph. */
class CapturedNode {
  static built: CapturedNode[] = [];
  /** Messages the app posted to the processor, oldest first. */
  readonly inbox: unknown[] = [];
  readonly port = {
    onmessage: null as ((event: MessageEvent) => void) | null,
    postMessage: (message: unknown) => {
      this.inbox.push(message);
    },
  };

  constructor(
    readonly context: unknown,
    readonly name: string,
    readonly options: { processorOptions?: Record<string, unknown> },
  ) {
    CapturedNode.built.push(this);
  }

  connect<T>(target: T): T {
    return target;
  }

  disconnect(): void {}
}

function stubNode() {
  return { connect: <T>(target: T) => target, disconnect: () => undefined };
}

function stubParam() {
  return {
    value: 1,
    cancelScheduledValues: () => undefined,
    setTargetAtTime: () => undefined,
    setValueAtTime: () => undefined,
    linearRampToValueAtTime: () => undefined,
  };
}

/** Enough of an AudioContext for the graph builders; no audio flows through it. */
function stubContext(): AudioContext {
  const context = {
    state: "running",
    sampleRate: SAMPLE_RATE,
    currentTime: 0,
    destination: stubNode(),
    audioWorklet: { addModule: async () => undefined },
    resume: async () => undefined,
    close: async () => undefined,
    createMediaStreamSource: () => stubNode(),
    createGain: () => ({ ...stubNode(), gain: stubParam() }),
    createMediaStreamDestination: () => {
      const track = { stop: () => undefined };
      return {
        ...stubNode(),
        stream: { getTracks: () => [track], getAudioTracks: () => [track] },
      };
    },
  };
  return context as unknown as AudioContext;
}

const stream = { getAudioTracks: () => [{}], getTracks: () => [{}] } as unknown as MediaStream;

/** Puts the stand-ins in the global scope for the length of `build`. */
async function inStubScope<T>(build: () => Promise<T>): Promise<{ result: T; node: CapturedNode }> {
  const scope = globalThis as Record<string, unknown>;
  const hadWindow = typeof scope.window !== "undefined";
  if (!hadWindow) scope.window = scope;
  const win = scope.window as Record<string, unknown>;
  const saved = { node: scope.AudioWorkletNode, context: win.AudioContext };
  scope.AudioWorkletNode = CapturedNode;
  win.AudioContext = function AudioContext() {
    return stubContext();
  };
  const before = CapturedNode.built.length;
  try {
    const result = await build();
    const node = CapturedNode.built[before];
    if (!node || CapturedNode.built.length !== before + 1) {
      throw new Error(`the app built ${CapturedNode.built.length - before} worklet nodes, expected one`);
    }
    return { result, node };
  } finally {
    if (saved.node === undefined) delete scope.AudioWorkletNode;
    else scope.AudioWorkletNode = saved.node;
    if (saved.context === undefined) delete win.AudioContext;
    else win.AudioContext = saved.context;
    if (!hadWindow) delete scope.window;
  }
}

/** A worklet node as the app built it, and what the app can still do to it. */
export interface AppDetector {
  name: string;
  processorOptions: Record<string, unknown>;
  /** Messages the app has posted to the processor so far; the driver drains it. */
  inbox: unknown[];
  /** Move the dial as the settings screen does, when the app can. */
  setSensitivity?(sensitivity: number): void;
  /** Tear down as the app does when the call ends. */
  dispose(): void;
}

function detector(node: CapturedNode, controls: Pick<AppDetector, "setSensitivity" | "dispose">): AppDetector {
  return {
    name: node.name,
    processorOptions: { ...(node.options.processorOptions ?? {}) },
    inbox: node.inbox,
    ...controls,
  };
}

/**
 * The node between the microphone and the peers: the gate, the leveller or
 * both, as the settings choose them. Neither means no node at all.
 */
export async function microphoneNode(
  sensitivity: number,
  stages: { gate: boolean; leveller: boolean },
): Promise<AppDetector> {
  const settings: VoiceSettings = {
    ...DEFAULT_SETTINGS,
    // The suppressor and the fixed gain are other nodes; they stay out.
    noiseSuppression: "off",
    inputGainDb: 0,
    vadSensitivity: sensitivity,
    inputGate: stages.gate,
    autoGainControl: stages.leveller,
  };
  const { result: graph, node } = await inStubScope<CaptureGraph | null>(() =>
    buildCaptureGraph(stream, settings, { createContext: stubContext }),
  );
  if (!graph) throw new Error("the capture graph was not built");
  return detector(node, {
    setSensitivity: (value) => graph.setSensitivity(value),
    dispose: () => graph.dispose(),
  });
}

/** The local meter and speaking ring: the detector on the monitor stream. */
export async function meterNode(sensitivity: number): Promise<AppDetector> {
  const speech = new SpeechDetector(() => undefined, { sensitivity });
  const { result: added, node } = await inStubScope(() => speech.add("local", stream, true));
  if (!added) throw new Error("the speech detector did not attach");
  return detector(node, {
    setSensitivity: (value) => speech.setSensitivity(value),
    dispose: () => speech.remove("local"),
  });
}

/** The detector behind a remote participant's speaking ring. */
export async function remoteMeterNode(): Promise<AppDetector> {
  const speech = new SpeechDetector(() => undefined);
  const { result: added, node } = await inStubScope(() => speech.add("remote", stream, false));
  if (!added) throw new Error("the speech detector did not attach");
  return detector(node, { dispose: () => speech.remove("remote") });
}

/** The gate a listener puts on someone else's strip (`RemoteVoiceMixer.setGate`). */
export async function remoteGateNode(): Promise<AppDetector> {
  const { result: gate, node } = await inStubScope(() =>
    buildGateNode(stubContext(), REMOTE_GATE_SENSITIVITY, { gate: true, leveller: false }),
  );
  if (!gate) throw new Error("the remote gate was not built");
  return detector(node, { dispose: () => node.port.postMessage({ type: "stop" }) });
}

export { REMOTE_GATE_SENSITIVITY };
