import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CallCrypto, type CallEpoch, type CallKeyEvent, type CallKeys, type CallKeySource } from "./callCrypto";
import type { FrameWorkerEvent, FrameWorkerRequest } from "./frameWorker";

const CONVERSATION = "0b5c1c1e-7a6f-4b0e-9f3a-1d2c3b4a5f60";
const SELF = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

/** A worker that records what it is told and lets the test answer. */
class FakeWorker {
  readonly posted: FrameWorkerRequest[] = [];
  onmessage: ((event: MessageEvent<FrameWorkerEvent>) => void) | null = null;
  terminated = false;
  postMessage(message: FrameWorkerRequest): void {
    this.posted.push(message);
  }
  terminate(): void {
    this.terminated = true;
  }
  say(event: FrameWorkerEvent): void {
    this.onmessage?.({ data: event } as MessageEvent<FrameWorkerEvent>);
  }
  sends(): (number | null)[] {
    return this.posted.flatMap((message) => (message.type === "send" ? [message.sequence] : []));
  }
  epochs(): number[] {
    return this.posted.flatMap((message) => (message.type === "epochs" ? message.epochs.map((e) => e.sequence) : []));
  }
}

const epoch = (sequence: number): CallEpoch => ({
  sequence,
  epochId: `epoch-${sequence}`,
  base: { type: "secret" } as CryptoKey,
});
const keysAt = (sequence: number): CallKeys => ({ self: SELF, send: epoch(sequence) });

function source(overrides: Partial<CallKeySource> = {}) {
  const listeners = new Set<(event: CallKeyEvent) => void>();
  const value = {
    keys: vi.fn<CallKeySource["keys"]>(() => Promise.resolve<CallKeys | null>(keysAt(1))),
    follow: vi.fn<CallKeySource["follow"]>(() => Promise.resolve(keysAt(1))),
    epoch: vi.fn<CallKeySource["epoch"]>((_conversation, sequence) => Promise.resolve(epoch(sequence))),
    subscribe: vi.fn<CallKeySource["subscribe"]>((_conversation, listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    }),
  };
  Object.assign(value, overrides);
  return { value, emit: (event: CallKeyEvent) => listeners.forEach((listener) => listener(event)), listeners };
}

function make(keys: CallKeySource, onStatus = vi.fn()) {
  const worker = new FakeWorker();
  const crypto = new CallCrypto({
    conversationId: CONVERSATION,
    keys: keysAt(1),
    source: keys,
    path: "script",
    onStatus,
    createWorker: () => worker as unknown as Worker,
    switchGraceMs: 1_500,
  });
  return { crypto, worker, onStatus };
}

const settle = () => vi.advanceTimersByTimeAsync(0);

describe("CallCrypto", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("hands the worker the conversation, its epoch and the epoch to send under before any frame", () => {
    const { worker } = make(source().value);
    expect(worker.posted.map((message) => message.type)).toEqual(["init", "epochs", "send"]);
    expect(worker.posted[0]).toEqual({ type: "init", conversationId: CONVERSATION, self: SELF });
    expect(worker.sends()).toEqual([1]);
  });

  it("gives a new epoch to the receivers at once and moves the senders only after the grace period", async () => {
    const keys = source();
    keys.value.keys.mockResolvedValue(keysAt(2));
    const { crypto, worker } = make(keys.value);
    keys.emit("keys");
    await settle();
    expect(worker.epochs()).toEqual([1, 2]);
    expect(worker.sends()).toEqual([1]);
    await vi.advanceTimersByTimeAsync(1_499);
    expect(worker.sends()).toEqual([1]);
    await vi.advanceTimersByTimeAsync(1);
    expect(worker.sends()).toEqual([1, 2]);
    expect(crypto.sendingSequence).toBe(2);
  });

  it("keeps its epoch when encryption was switched off or the keys cannot be read", async () => {
    const keys = source();
    const { worker } = make(keys.value);
    keys.value.keys.mockResolvedValueOnce(null);
    keys.emit("keys");
    await vi.advanceTimersByTimeAsync(5_000);
    keys.value.keys.mockRejectedValueOnce(new Error("offline"));
    keys.emit("keys");
    await vi.advanceTimersByTimeAsync(5_000);
    expect(worker.sends()).toEqual([1]);
    expect(worker.epochs()).toEqual([1]);
  });

  it("looks up an epoch frames name at most once every two seconds and gives it to the worker", async () => {
    const keys = source();
    const { worker } = make(keys.value);
    worker.say({ type: "need-epoch", sequence: 7 });
    worker.say({ type: "need-epoch", sequence: 7 });
    await settle();
    expect(keys.value.epoch).toHaveBeenCalledTimes(1);
    expect(worker.epochs()).toEqual([1, 7]);
    await vi.advanceTimersByTimeAsync(2_000);
    worker.say({ type: "need-epoch", sequence: 7 });
    await settle();
    expect(keys.value.epoch).toHaveBeenCalledTimes(2);
    // Given once is enough.
    expect(worker.epochs()).toEqual([1, 7]);
  });

  it("knows which receiver fails, and stops knowing once the connection is gone", () => {
    const { crypto, worker, onStatus } = make(source().value);
    const transforms: unknown[] = [];
    vi.stubGlobal(
      "RTCRtpScriptTransform",
      class {
        constructor(_worker: Worker, options: unknown) {
          transforms.push(options);
        }
      },
    );
    const receiver = {} as RTCRtpReceiver;
    crypto.attachReceiver(receiver, "video", "3", "peer");
    expect(transforms).toEqual([{ role: "receive", kind: "video", label: "rx:1", sender: "peer" }]);
    worker.say({ type: "status", label: "rx:1", failing: true });
    expect(crypto.failing(receiver)).toBe(true);
    expect(onStatus).toHaveBeenCalledTimes(1);
    crypto.forgetTransforms();
    expect(crypto.failing(receiver)).toBe(false);
    expect(onStatus).toHaveBeenCalledTimes(2);
    vi.unstubAllGlobals();
  });

  it("binds a receiver the directory did not name yet once it does", () => {
    const { crypto, worker } = make(source().value);
    vi.stubGlobal("RTCRtpScriptTransform", class {});
    crypto.attachReceiver({} as RTCRtpReceiver, "audio", "5", null);
    crypto.bind((mid) => (mid === "5" ? "peer" : null));
    crypto.bind(() => "someone-else");
    expect(worker.posted.filter((message) => message.type === "bind")).toEqual([
      { type: "bind", label: "rx:1", sender: "peer" },
    ]);
    vi.unstubAllGlobals();
  });

  it("stops the worker and listening when the call ends", () => {
    const keys = source();
    const { crypto, worker } = make(keys.value);
    crypto.dispose();
    expect(worker.terminated).toBe(true);
    expect(keys.listeners.size).toBe(0);
  });
});
