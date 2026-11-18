/**
 * The main-thread side of an encrypted call.
 *
 * One `CallCrypto` lives as long as one encrypted call: it is made when the
 * call is decided, before the seat is taken, and it outlives every media
 * session the call goes through, so a replaced connection keeps its keys and
 * its encryption. It owns the frame worker (`frameWorker.ts`), puts a
 * transform on every sender and receiver of the call's connection, hands the
 * worker the epochs it needs, and follows the conversation's keys during the
 * call: a new epoch goes to the worker at once, and the senders move to it
 * after a short grace period, by which the other side has it too.
 *
 * Switching encryption off during the call changes nothing here: the call
 * keeps the epoch it has until it ends.
 */

import type { CallEpoch, CallKeys } from "@/lib/e2ee/client";

import type { FrameWorkerEvent, FrameWorkerRequest, TransformOptions } from "./frameWorker";

export type { CallEpoch, CallKeys } from "@/lib/e2ee/client";

export type CallCryptoPath = "script" | "streams";

/**
 * Which encoded-transform API this browser offers for encrypted calls, or
 * null when it has none and cannot take part in one.
 */
export function callEncryptionPath(): CallCryptoPath | null {
  if (typeof RTCRtpScriptTransform === "function") return "script";
  if (typeof RTCRtpSender !== "undefined" && "createEncodedStreams" in RTCRtpSender.prototype) return "streams";
  return null;
}

export type CallKeyEvent = "keys" | "on" | "off";

/** Where a call's keys come from: the encryption client of the conversation. */
export interface CallKeySource {
  /** The keys of a call; null when the server says the conversation is not encrypted. Throws when no key can be had. */
  keys(conversationId: string): Promise<CallKeys | null>;
  /** The keys for joining an encrypted call under way, whatever the switch says now. */
  follow(conversationId: string): Promise<CallKeys>;
  /** An epoch a frame names by the low 16 bits of its sequence. */
  epoch(conversationId: string, sequence: number): Promise<CallEpoch | null>;
  /** Keys or epochs of the conversation changed, or its switch moved. */
  subscribe(conversationId: string, listener: (event: CallKeyEvent) => void): () => void;
}

/** How long senders wait before they move to a new epoch the receivers were just given. */
const SWITCH_GRACE_MS = 1_500;
/** An epoch a frame names is looked for at most this often. */
const EPOCH_LOOKUP_MS = 2_000;

export interface CallCryptoOptions {
  conversationId: string;
  keys: CallKeys;
  source: CallKeySource;
  path: CallCryptoPath;
  /** A receiver started or stopped failing. */
  onStatus(): void;
  /** The frame worker; a module worker of `frameWorker.ts` unless tests give another. */
  createWorker?: () => Worker;
  switchGraceMs?: number;
}

interface Receiver {
  label: string;
  kind: "audio" | "video";
  mid: string | null;
  bound: boolean;
  failing: boolean;
}

function frameWorker(): Worker {
  return new Worker(new URL("./frameWorker.ts", import.meta.url), { type: "module", name: "call-frames" });
}

export class CallCrypto {
  readonly path: CallCryptoPath;
  private readonly worker: Worker;
  private readonly conversationId: string;
  private readonly source: CallKeySource;
  private readonly onStatus: () => void;
  private readonly graceMs: number;
  private sending: number;
  private readonly given = new Set<number>();
  private readonly lookedUp = new Map<number, number>();
  private readonly senders = new Map<RTCRtpSender, string>();
  private readonly receivers = new Map<RTCRtpReceiver, Receiver>();
  private readonly unsubscribe: () => void;
  private switchTimer: ReturnType<typeof setTimeout> | null = null;
  private next = 0;
  private disposed = false;

  constructor(options: CallCryptoOptions) {
    this.path = options.path;
    this.conversationId = options.conversationId;
    this.source = options.source;
    this.onStatus = options.onStatus;
    this.graceMs = options.switchGraceMs ?? SWITCH_GRACE_MS;
    this.worker = (options.createWorker ?? frameWorker)();
    this.worker.onmessage = (event: MessageEvent<FrameWorkerEvent>) => this.message(event.data);
    this.post({ type: "init", conversationId: options.conversationId, self: options.keys.self });
    this.give(options.keys.send);
    this.sending = options.keys.send.sequence;
    this.post({ type: "send", sequence: this.sending });
    this.unsubscribe = options.source.subscribe(options.conversationId, (event) => {
      if (event === "keys") void this.refresh();
    });
  }

  /** The epoch senders use now. */
  get sendingSequence(): number {
    return this.sending;
  }

  /** Chromium's older API needs the connection made for it. */
  get needsEncodedStreams(): boolean {
    return this.path === "streams";
  }

  /** Every sender encrypts from its first frame; call it as the sender is made. */
  attachSender(sender: RTCRtpSender, kind: "audio" | "video"): void {
    if (this.disposed) return;
    const label = `tx:${++this.next}`;
    this.senders.set(sender, label);
    this.attach(sender, { role: "send", kind, label });
  }

  /**
   * Every receiver decrypts, bound to the publisher the media server's
   * directory names for its section. One that is not named yet drops what it
   * gets until `bind` names it.
   */
  attachReceiver(receiver: RTCRtpReceiver, kind: "audio" | "video", mid: string | null, publisher: string | null): void {
    if (this.disposed || this.receivers.has(receiver)) return;
    const label = `rx:${++this.next}`;
    this.receivers.set(receiver, { label, kind, mid, bound: publisher !== null, failing: false });
    this.attach(receiver, { role: "receive", kind, label, ...(publisher === null ? {} : { sender: publisher }) });
  }

  /** Names the publisher of every receiver that was waiting for the directory. */
  bind(publisherOf: (mid: string) => string | null): void {
    for (const receiver of this.receivers.values()) {
      if (receiver.bound || receiver.mid === null) continue;
      const publisher = publisherOf(receiver.mid);
      if (publisher === null) continue;
      receiver.bound = true;
      this.post({ type: "bind", label: receiver.label, sender: publisher });
    }
  }

  /** After a negotiation: what each sender ended up with, for a browser whose frames do not say. */
  noteCodecs(): void {
    for (const [sender, label] of this.senders) {
      try {
        const mimeType = sender.getParameters().codecs?.[0]?.mimeType;
        if (mimeType) this.post({ type: "codec", label, mimeType });
      } catch {
        // A sender of a closed connection.
      }
    }
  }

  /** Whether nothing of this receiver's frames has opened for a while. */
  failing(receiver: RTCRtpReceiver): boolean {
    return this.receivers.get(receiver)?.failing ?? false;
  }

  /** The connection was closed: forget its senders and receivers. Their transforms end with it. */
  forgetTransforms(): void {
    this.senders.clear();
    const wasFailing = [...this.receivers.values()].some((receiver) => receiver.failing);
    this.receivers.clear();
    if (wasFailing) this.onStatus();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.unsubscribe();
    if (this.switchTimer !== null) clearTimeout(this.switchTimer);
    this.worker.terminate();
    this.senders.clear();
    this.receivers.clear();
  }

  private attach(target: RTCRtpSender | RTCRtpReceiver, options: TransformOptions): void {
    if (this.path === "script") {
      target.transform = new RTCRtpScriptTransform(this.worker, options);
      return;
    }
    const streams = (
      target as unknown as { createEncodedStreams(): { readable: ReadableStream; writable: WritableStream } }
    ).createEncodedStreams();
    this.post({ type: "streams", readable: streams.readable, writable: streams.writable, options }, [
      streams.readable,
      streams.writable,
    ]);
  }

  private post(message: FrameWorkerRequest, transfer: Transferable[] = []): void {
    if (!this.disposed) this.worker.postMessage(message, transfer);
  }

  private give(epoch: CallEpoch): void {
    const sequence = epoch.sequence & 0xffff;
    if (this.given.has(sequence)) return;
    this.given.add(sequence);
    this.post({ type: "epochs", epochs: [epoch] });
  }

  /**
   * The conversation's keys changed: the epoch the next message would go out
   * under is handed out now and sent under after the grace period. While the
   * conversation is not encrypted, or the keys cannot be read, the call keeps
   * the epoch it has.
   */
  private async refresh(): Promise<void> {
    let keys: CallKeys | null;
    try {
      keys = await this.source.keys(this.conversationId);
    } catch {
      return;
    }
    if (keys === null || this.disposed) return;
    const next = keys.send;
    this.give(next);
    if (next.sequence === this.sending) return;
    if (this.switchTimer !== null) clearTimeout(this.switchTimer);
    this.switchTimer = setTimeout(() => {
      this.switchTimer = null;
      // A newer epoch may have arrived meanwhile; only ever move forward.
      if (this.disposed || next.sequence < this.sending) return;
      this.sending = next.sequence;
      this.post({ type: "send", sequence: next.sequence });
    }, this.graceMs);
  }

  private message(event: FrameWorkerEvent): void {
    if (this.disposed) return;
    if (event.type === "status") {
      for (const receiver of this.receivers.values()) {
        if (receiver.label !== event.label || receiver.failing === event.failing) continue;
        receiver.failing = event.failing;
        this.onStatus();
      }
    } else if (event.type === "need-epoch") {
      void this.lookUp(event.sequence);
    }
  }

  private async lookUp(sequence: number): Promise<void> {
    const now = Date.now();
    if (now - (this.lookedUp.get(sequence) ?? Number.NEGATIVE_INFINITY) < EPOCH_LOOKUP_MS) return;
    this.lookedUp.set(sequence, now);
    const epoch = await this.source.epoch(this.conversationId, sequence).catch(() => null);
    if (epoch !== null && !this.disposed) this.give(epoch);
  }
}
