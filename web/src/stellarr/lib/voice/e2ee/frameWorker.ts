/**
 * The worker that encrypts and decrypts the frames of one encrypted call.
 *
 * Every sender and receiver of the call's connection hands its encoded frames
 * through here, by WebRTC Encoded Transform (`RTCRtpScriptTransform`) or by
 * Chromium's older `createEncodedStreams`. The format is in `./sframe`.
 *
 * It fails closed. A sender with no epoch to send under, or a picture in a
 * codec that cannot be encrypted, sends nothing. A receiver plays only what
 * opened: a frame that is not encrypted, does not verify, names an epoch this
 * side does not hold yet or repeats a counter is dropped and counted, and
 * once nothing has opened for a second the receiver says it is failing and
 * asks for keyframes, at most one a second, until a keyframe opens.
 *
 * The main thread gives it the conversation, the epochs (as non-extractable
 * HKDF bases) and which one to send under, and binds each receiver to the
 * user the media server's directory names as its publisher. That user goes
 * into the key derivation, so a frame reflected back to its own sender, or
 * relabelled as somebody else's, does not open.
 *
 * Erasable TypeScript only, for the media smoke harness, which serves this
 * file to a plain page.
 */

import {
  clearPrefixOf,
  deriveFrameKey,
  epochOfKeyId,
  keyIdHex,
  keyIdOf,
  openFrame,
  parseFrame,
  ReplayWindow,
  sealFrame,
  type FrameKey,
  type KeyId,
} from "./sframe";

export interface TransformOptions {
  role: "send" | "receive";
  kind: "audio" | "video";
  /** Names the transform in status messages and statistics. */
  label: string;
  /** For a receiver: the user whose frames it opens, when already known. */
  sender?: string;
}

export interface WorkerEpoch {
  sequence: number;
  epochId: string;
  /** The epoch key's raw bytes as an HKDF key; never extractable. */
  base: CryptoKey;
}

export type FrameWorkerRequest =
  | { type: "init"; conversationId: string; self: string }
  | { type: "epochs"; epochs: WorkerEpoch[] }
  /** The epoch senders use from their next frame; null stops every sender. */
  | { type: "send"; sequence: number | null }
  | { type: "bind"; label: string; sender: string }
  /** The codec a sender negotiated, for a browser whose frames do not say. */
  | { type: "codec"; label: string; mimeType: string }
  | { type: "stats"; id: number }
  | { type: "streams"; readable: ReadableStream; writable: WritableStream; options: TransformOptions };

export type DropReason = "plain" | "unbound" | "unknown-epoch" | "failed" | "replay" | "no-key" | "codec";

export interface TransformStats {
  label: string;
  role: "send" | "receive";
  kind: "audio" | "video";
  frames: number;
  /** Frames sealed or opened and passed on. */
  passed: number;
  dropped: Record<DropReason, number>;
  keyframeRequests: number;
  bytesIn: number;
  bytesOut: number;
  /** Sizes of the frames that came in, for telling a constant bitrate from a variable one. */
  sizeMin: number;
  sizeMax: number;
  sizeSquares: number;
  failing: boolean;
}

export type FrameWorkerEvent =
  | { type: "status"; label: string; failing: boolean }
  /** Frames name an epoch this side does not hold; asked at most once a second per epoch. */
  | { type: "need-epoch"; sequence: number }
  | { type: "stats"; id: number; transforms: TransformStats[] };

export interface EncodedFrame {
  data: ArrayBuffer;
  type?: "key" | "delta" | "empty";
  getMetadata(): { mimeType?: string };
}

export interface ScriptTransformer {
  readable: ReadableStream<EncodedFrame>;
  writable: WritableStream<EncodedFrame>;
  options: TransformOptions;
  sendKeyFrameRequest?: () => Promise<void>;
}

export type Transform = (
  frame: EncodedFrame,
  controller: { enqueue(frame: EncodedFrame): void },
) => Promise<void>;

/** Nothing opened for this long after a drop: the receiver is failing. */
const FAILING_AFTER_MS = 1_000;
const KEYFRAME_REQUEST_MS = 1_000;
const EPOCH_REQUEST_MS = 1_000;
const RECEIVE_KEYS = 32;
const REPLAY_WINDOWS = 256;
const DERIVATIONS_PER_SECOND = 32;

const noDrops = (): Record<DropReason, number> => ({
  plain: 0,
  unbound: 0,
  "unknown-epoch": 0,
  failed: 0,
  replay: 0,
  "no-key": 0,
  codec: 0,
});

/**
 * The worker's whole state and behaviour apart from the worker scope, so the
 * unit tests can drive it with frames of their own.
 */
export function createFrameCrypto(post: (message: FrameWorkerEvent) => void, now: () => number = Date.now) {
  let conversationId = "";
  let self = "";
  const epochs = new Map<number, WorkerEpoch>();
  /** The 16-bit sequence senders seal under, or null. */
  let sending: number | null = null;
  /** Only keys that opened a frame are cached. Replay state survives LRU eviction. */
  const receiveKeys = new Map<string, FrameKey>();
  const replayWindows = new Map<string, ReplayWindow>();
  let derivationsLeft = DERIVATIONS_PER_SECOND;
  let lastRefill = now();
  const binds = new Map<string, string>();
  const codecs = new Map<string, string>();
  let lastEpochRequest = Number.NEGATIVE_INFINITY;
  const allStats: TransformStats[] = [];

  function newStats(options: TransformOptions): TransformStats {
    const stats: TransformStats = {
      label: options.label,
      role: options.role,
      kind: options.kind,
      frames: 0,
      passed: 0,
      dropped: noDrops(),
      keyframeRequests: 0,
      bytesIn: 0,
      bytesOut: 0,
      sizeMin: Number.POSITIVE_INFINITY,
      sizeMax: 0,
      sizeSquares: 0,
      failing: false,
    };
    allStats.push(stats);
    return stats;
  }

  function count(stats: TransformStats, size: number): void {
    stats.frames += 1;
    stats.bytesIn += size;
    if (size < stats.sizeMin) stats.sizeMin = size;
    if (size > stats.sizeMax) stats.sizeMax = size;
    stats.sizeSquares += size * size;
  }

  function askForEpoch(sequence: number): void {
    const at = now();
    if (at - lastEpochRequest < EPOCH_REQUEST_MS) return;
    lastEpochRequest = at;
    post({ type: "need-epoch", sequence });
  }

  function sender(options: TransformOptions, stats: TransformStats): Transform {
    let sequence: number | null = null;
    let kid: KeyId = { hi: 0, lo: 0 };
    let counter = 0;
    let key: FrameKey | null = null;

    async function rekey(wanted: number): Promise<void> {
      key = null;
      sequence = wanted;
      const epoch = epochs.get(wanted);
      if (epoch === undefined) return;
      // A fresh key id for every epoch and every transform: a new connection
      // makes new transforms, so a counter never starts again under a key.
      const next = keyIdOf(epoch.sequence, crypto.getRandomValues(new Uint8Array(6)));
      const derived = await deriveFrameKey(
        epoch.base,
        { epochId: epoch.epochId, conversationId, senderId: self },
        next,
        "encrypt",
      );
      kid = next;
      counter = 0;
      key = derived;
    }

    return async (frame, controller) => {
      const data = new Uint8Array(frame.data);
      count(stats, data.length);
      const wanted = sending;
      if (wanted === null) {
        stats.dropped["no-key"] += 1;
        return;
      }
      // Frames of one transform come one at a time, so nothing else moves
      // the key while this waits.
      if (wanted !== sequence || key === null) await rekey(wanted);
      const frameKey = key;
      if (frameKey === null) {
        stats.dropped["no-key"] += 1;
        return;
      }
      const mimeType =
        options.kind === "video" ? (frame.getMetadata().mimeType ?? codecs.get(options.label)) : undefined;
      const clear = clearPrefixOf(options.kind, mimeType, frame.type === "key", data.length);
      if (clear === null) {
        stats.dropped.codec += 1;
        return;
      }
      // Taken before the first await, so no two frames share one.
      const frameCounter = counter;
      counter += 1;
      const sealed = await sealFrame(frameKey, kid, frameCounter, data, clear);
      frame.data = sealed.buffer;
      stats.passed += 1;
      stats.bytesOut += sealed.length;
      controller.enqueue(frame);
    };
  }

  function receiver(options: TransformOptions, stats: TransformStats, transformer: ScriptTransformer | null): Transform {
    let lastOpened = Number.NEGATIVE_INFINITY;
    let lastRequest = Number.NEGATIVE_INFINITY;
    let needKeyframe = false;

    function setFailing(failing: boolean): void {
      if (stats.failing === failing) return;
      stats.failing = failing;
      post({ type: "status", label: options.label, failing });
    }

    function requestKeyframe(): void {
      if (options.kind !== "video" || transformer?.sendKeyFrameRequest === undefined) return;
      const at = now();
      if (at - lastRequest < KEYFRAME_REQUEST_MS) return;
      lastRequest = at;
      stats.keyframeRequests += 1;
      transformer.sendKeyFrameRequest().catch(() => undefined);
    }

    function drop(reason: DropReason): void {
      stats.dropped[reason] += 1;
      // A repeated frame is not a sign that the picture or sound is broken.
      if (reason === "replay") return;
      needKeyframe = true;
      if (now() - lastOpened >= FAILING_AFTER_MS) setFailing(true);
      requestKeyframe();
    }

    return async (frame, controller) => {
      const data = new Uint8Array(frame.data);
      count(stats, data.length);
      const senderId = options.sender ?? binds.get(options.label);
      if (senderId === undefined) {
        drop("unbound");
        return;
      }
      const parsed = parseFrame(data);
      if (parsed === null) {
        drop("plain");
        return;
      }
      const sequence = epochOfKeyId(parsed.kid);
      const epoch = epochs.get(sequence);
      if (epoch === undefined) {
        askForEpoch(sequence);
        drop("unknown-epoch");
        return;
      }
      const id = `${epoch.epochId}|${senderId}|${keyIdHex(parsed.kid)}`;
      let replay = replayWindows.get(id);
      if (replay && !replay.accepts(parsed.counter)) {
        drop("replay");
        return;
      }
      let key = receiveKeys.get(id);
      let opened: ArrayBuffer;
      try {
        if (key === undefined) {
          const at = now();
          if (at - lastRefill >= 1_000) {
            derivationsLeft = DERIVATIONS_PER_SECOND;
            lastRefill = at;
          }
          if (derivationsLeft === 0 || (!replay && replayWindows.size >= REPLAY_WINDOWS)) {
            drop("failed");
            return;
          }
          derivationsLeft -= 1;
          key = await deriveFrameKey(
            epoch.base, { epochId: epoch.epochId, conversationId, senderId }, parsed.kid, "decrypt",
          );
        }
        opened = await openFrame(key, parsed, data);
      } catch {
        drop("failed");
        return;
      }

      // Another transform may have opened this key or counter in the meantime.
      replay = replayWindows.get(id);
      if (replay === undefined) {
        if (replayWindows.size >= REPLAY_WINDOWS) {
          drop("failed");
          return;
        }
        replay = new ReplayWindow();
        replayWindows.set(id, replay);
      }
      if (!replay.accepts(parsed.counter)) {
        drop("replay");
        return;
      }
      replay.mark(parsed.counter);
      receiveKeys.delete(id);
      receiveKeys.set(id, key);
      if (receiveKeys.size > RECEIVE_KEYS) {
        const oldest = receiveKeys.keys().next().value;
        if (oldest !== undefined) receiveKeys.delete(oldest);
      }
      frame.data = opened;
      lastOpened = now();
      stats.passed += 1;
      stats.bytesOut += opened.byteLength;
      setFailing(false);
      if (needKeyframe) {
        if (frame.type === "key") needKeyframe = false;
        else requestKeyframe();
      }
      controller.enqueue(frame);
    };
  }

  function make(options: TransformOptions, transformer: ScriptTransformer | null) {
    const stats = newStats(options);
    const transform = options.role === "send" ? sender(options, stats) : receiver(options, stats, transformer);
    return { stats, transform };
  }

  function attach(
    readable: ReadableStream<EncodedFrame>,
    writable: WritableStream<EncodedFrame>,
    options: TransformOptions,
    transformer: ScriptTransformer | null,
  ): void {
    const { stats, transform } = make(options, transformer);
    readable
      .pipeThrough(new TransformStream<EncodedFrame, EncodedFrame>({ transform }))
      .pipeTo(writable)
      .catch(() => undefined)
      .finally(() => {
        // A transform of a closed connection: its statistics go with it.
        const at = allStats.indexOf(stats);
        if (at !== -1) allStats.splice(at, 1);
      });
  }

  /** The function one sender's or receiver's frames go through, for the tests. */
  function transformFor(options: TransformOptions, transformer: ScriptTransformer | null = null): Transform {
    return make(options, transformer).transform;
  }

  function handle(message: FrameWorkerRequest): void {
    switch (message.type) {
      case "init":
        conversationId = message.conversationId;
        self = message.self;
        return;
      case "epochs":
        for (const epoch of message.epochs) {
          epochs.set(epoch.sequence & 0xffff, epoch);

        }
        return;
      case "send":
        sending = message.sequence === null ? null : message.sequence & 0xffff;
        return;
      case "bind":
        binds.set(message.label, message.sender);
        return;
      case "codec":
        codecs.set(message.label, message.mimeType);
        return;
      case "stats":
        post({
          type: "stats",
          id: message.id,
          transforms: allStats.map((stats) => ({ ...stats, dropped: { ...stats.dropped } })),
        });
        return;
      case "streams":
        attach(message.readable, message.writable, message.options, null);
        return;
    }
  }

  return { handle, attach, transformFor };
}

interface WorkerScope {
  postMessage(message: FrameWorkerEvent): void;
  onmessage: ((event: MessageEvent<FrameWorkerRequest>) => void) | null;
  onrtctransform: ((event: Event & { transformer: ScriptTransformer }) => void) | null;
}

// Only inside a worker: the unit tests import this module for its factory.
const scope = globalThis as unknown as Partial<WorkerScope> & { WorkerGlobalScope?: unknown };
if (typeof scope.WorkerGlobalScope !== "undefined" && typeof scope.postMessage === "function") {
  const worker = scope as WorkerScope;
  const frames = createFrameCrypto((message) => worker.postMessage(message));
  worker.onmessage = (event) => frames.handle(event.data);
  worker.onrtctransform = (event) => {
    const { transformer } = event;
    frames.attach(transformer.readable, transformer.writable, transformer.options, transformer);
  };
}
