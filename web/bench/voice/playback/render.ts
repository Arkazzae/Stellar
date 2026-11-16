/**
 * Renders audio through the app's own playback and capture nodes in an
 * OfflineAudioContext. Runs in Chrome only.
 *
 * A strip is built by `RemoteVoiceMixer` itself, exactly as it builds one for
 * a participant: the mixer's `attachElement` mounts a `RemoteStrip`, applies
 * the profile and the volume immediately, and `setGate` inserts the
 * receiver's gate. Two things an offline context lacks are added to the
 * context object for the length of the build, and nothing else is changed:
 *
 * - `createMediaStreamDestination`, which the strip ends in, becomes a gain
 *   node that feeds the rendered output and carries a stand-in `stream`;
 * - `createMediaElementSource` hands back a buffer source playing the test
 *   signal in place of the `<audio>` element the mixer would play.
 *
 * `window.AudioContext` is pointed at that context while the mixer is
 * created, so its own `ensureContext` picks it up.
 */
import { buildGateNode } from "@/lib/voice/capture/gateNode";
import { RemoteVoiceMixer } from "@/lib/voice/playback/remoteMixer";
import type { PlaybackProfileId, PlaybackTuning } from "@/lib/voice/settings";

export const RATE = 48_000;

function stubStream(): MediaStream {
  const track = { stop: () => undefined, kind: "audio" };
  return { getTracks: () => [track], getAudioTracks: () => [track] } as unknown as MediaStream;
}

function offline(signal: Float32Array): { context: OfflineAudioContext; source: AudioBufferSourceNode } {
  const context = new OfflineAudioContext({ numberOfChannels: 1, length: signal.length, sampleRate: RATE });
  const buffer = context.createBuffer(1, signal.length, RATE);
  buffer.copyToChannel(signal as Float32Array<ArrayBuffer>, 0);
  const source = context.createBufferSource();
  source.buffer = buffer;
  return { context, source };
}

async function rendered(context: OfflineAudioContext, source: AudioBufferSourceNode): Promise<Float32Array> {
  source.start(0);
  const result = await context.startRendering();
  return result.getChannelData(0).slice();
}

export interface StripSpec {
  profile: PlaybackProfileId;
  tuning?: PlaybackTuning;
  volume: number;
  gated?: boolean;
}

/** `signal` through one participant's strip, built by the mixer. */
export async function throughStrip(spec: StripSpec, signal: Float32Array): Promise<Float32Array> {
  const { context, source } = offline(signal);
  const sink = context.createGain();
  sink.connect(context.destination);
  Object.assign(context, {
    createMediaStreamDestination() {
      const node = context.createGain();
      node.connect(sink);
      return Object.assign(node, { stream: stubStream() });
    },
    createMediaElementSource() {
      return source;
    },
  });
  const scope = window as unknown as { AudioContext: unknown };
  const previous = scope.AudioContext;
  scope.AudioContext = function AudioContext() {
    return context;
  };
  try {
    const mixer = new RemoteVoiceMixer(spec.profile, spec.tuning);
    const output = { srcObject: null } as unknown as HTMLAudioElement;
    if (!mixer.attachElement("participant", {} as HTMLMediaElement, output, spec.volume)) {
      throw new Error("the mixer refused the strip");
    }
    if (spec.gated && !(await mixer.setGate("participant", true))) {
      throw new Error("the mixer could not gate the strip");
    }
  } finally {
    scope.AudioContext = previous;
  }
  return rendered(context, source);
}

/** `signal` through the capture node `buildGateNode` makes, in Chrome's AudioWorklet. */
export async function throughDetector(
  sensitivity: number,
  stages: { gate: boolean; leveller: boolean },
  signal: Float32Array,
): Promise<{ output: Float32Array; posts: unknown[] }> {
  const { context, source } = offline(signal);
  const node = await buildGateNode(context as unknown as AudioContext, sensitivity, stages);
  if (!node) throw new Error("the detector node was not built");
  const posts: unknown[] = [];
  node.port.onmessage = (event: MessageEvent) => posts.push(event.data);
  source.connect(node);
  node.connect(context.destination);
  const output = await rendered(context, source);
  // Messages posted during the last quanta arrive after the render resolves.
  await new Promise((resolve) => setTimeout(resolve, 200));
  return { output, posts };
}

/** Decodes a file with Chrome's own decoder, at 48 kHz, as one channel. */
export async function decode(url: string): Promise<Float32Array> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: ${response.status}`);
  const context = new OfflineAudioContext({ numberOfChannels: 1, length: 1, sampleRate: RATE });
  const audio = await context.decodeAudioData(await response.arrayBuffer());
  const mono = new Float32Array(audio.length);
  for (let channel = 0; channel < audio.numberOfChannels; channel += 1) {
    const data = audio.getChannelData(channel);
    for (let i = 0; i < mono.length; i += 1) mono[i] = (mono[i] ?? 0) + (data[i] ?? 0) / audio.numberOfChannels;
  }
  // A whole number of render quanta, so the capture driver and this agree.
  const padded = new Float32Array(Math.ceil(mono.length / 128) * 128);
  padded.set(mono);
  return padded;
}
