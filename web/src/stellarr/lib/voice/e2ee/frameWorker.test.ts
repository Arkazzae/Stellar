import { describe, expect, it, vi } from "vitest";

import {
  createFrameCrypto,
  type EncodedFrame,
  type FrameWorkerEvent,
  type TransformOptions,
  type WorkerEpoch,
} from "./frameWorker";
import { epochOfKeyId, parseFrame } from "./sframe";

const CONVERSATION = "0b5c1c1e-7a6f-4b0e-9f3a-1d2c3b4a5f60";
const MAJA = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
const OLA = "6f5e4d3c-2b1a-4f9e-8d7c-6b5a4f3e2d1c";

async function epoch(sequence: number): Promise<WorkerEpoch> {
  const raw = crypto.getRandomValues(new Uint8Array(32));
  return {
    sequence,
    epochId: crypto.randomUUID(),
    base: await crypto.subtle.importKey("raw", raw, "HKDF", false, ["deriveBits"]),
  };
}

function frame(bytes: number[] | Uint8Array, options: { type?: "key" | "delta"; mimeType?: string } = {}): EncodedFrame {
  const data = Uint8Array.from(bytes);
  return {
    data: data.buffer,
    ...(options.type === undefined ? {} : { type: options.type }),
    getMetadata: () => (options.mimeType === undefined ? {} : { mimeType: options.mimeType }),
  };
}

/** One side of a call: its frame crypto, what it posted, and a clock the test moves. */
function side(self: string) {
  const events: FrameWorkerEvent[] = [];
  let clock = 1_000_000;
  const frames = createFrameCrypto((event) => events.push(event), () => clock);
  frames.handle({ type: "init", conversationId: CONVERSATION, self });
  return {
    frames,
    events,
    tick(ms: number) {
      clock += ms;
    },
    async pass(options: TransformOptions, input: EncodedFrame, transformer = null) {
      const out: EncodedFrame[] = [];
      await frames.transformFor(options, transformer)(input, { enqueue: (item) => out.push(item) });
      return out;
    },
  };
}

const SEND_AUDIO: TransformOptions = { role: "send", kind: "audio", label: "tx:mic" };
const SEND_VIDEO: TransformOptions = { role: "send", kind: "video", label: "tx:camera" };
const RECEIVE_AUDIO: TransformOptions = { role: "receive", kind: "audio", label: "rx:1", sender: MAJA };

describe("frame worker", () => {
  it("sends nothing until it has an epoch to send under, then frames the receiver opens", async () => {
    const shared = await epoch(3);
    const maja = side(MAJA);
    const ola = side(OLA);
    const send = maja.frames.transformFor(SEND_AUDIO);
    const out: EncodedFrame[] = [];
    await send(frame([1, 2, 3, 4]), { enqueue: (item) => out.push(item) });
    expect(out).toHaveLength(0);

    maja.frames.handle({ type: "epochs", epochs: [shared] });
    maja.frames.handle({ type: "send", sequence: 3 });
    await send(frame([1, 2, 3, 4]), { enqueue: (item) => out.push(item) });
    expect(out).toHaveLength(1);
    const sealed = new Uint8Array(out[0]!.data);
    expect(epochOfKeyId(parseFrame(sealed)!.kid)).toBe(3);

    ola.frames.handle({ type: "epochs", epochs: [shared] });
    const opened = await ola.pass(RECEIVE_AUDIO, frame(sealed));
    expect([...new Uint8Array(opened[0]!.data)]).toEqual([1, 2, 3, 4]);
  });

  it("draws a new key id for every sender transform and every epoch", async () => {
    const [first, second] = [await epoch(1), await epoch(2)];
    const maja = side(MAJA);
    maja.frames.handle({ type: "epochs", epochs: [first, second] });
    maja.frames.handle({ type: "send", sequence: 1 });
    const kidOf = async (send: ReturnType<typeof maja.frames.transformFor>) => {
      const out: EncodedFrame[] = [];
      await send(frame([9, 9]), { enqueue: (item) => out.push(item) });
      return parseFrame(new Uint8Array(out[0]!.data))!.kid;
    };
    const send = maja.frames.transformFor(SEND_AUDIO);
    const one = await kidOf(send);
    const other = await kidOf(maja.frames.transformFor(SEND_AUDIO));
    expect(other).not.toEqual(one);
    maja.frames.handle({ type: "send", sequence: 2 });
    const rotated = await kidOf(send);
    expect(epochOfKeyId(rotated)).toBe(2);
  });

  it("refuses a picture it cannot encrypt, and takes the codec from the main thread when frames do not say", async () => {
    const maja = side(MAJA);
    maja.frames.handle({ type: "epochs", epochs: [await epoch(1)] });
    maja.frames.handle({ type: "send", sequence: 1 });
    expect(await maja.pass(SEND_VIDEO, frame(new Uint8Array(40), { type: "key", mimeType: "video/H264" }))).toHaveLength(0);
    expect(await maja.pass(SEND_VIDEO, frame(new Uint8Array(40), { type: "key" }))).toHaveLength(0);

    maja.frames.handle({ type: "codec", label: SEND_VIDEO.label, mimeType: "video/VP8" });
    const [sent] = await maja.pass(SEND_VIDEO, frame(new Uint8Array(40).fill(7), { type: "key" }));
    expect(parseFrame(new Uint8Array(sent!.data))!.clear).toBe(10);
  });

  it("drops what an unbound receiver gets, and opens once the directory names the publisher", async () => {
    const shared = await epoch(1);
    const maja = side(MAJA);
    maja.frames.handle({ type: "epochs", epochs: [shared] });
    maja.frames.handle({ type: "send", sequence: 1 });
    const [sent] = await maja.pass(SEND_AUDIO, frame([5, 6, 7]));
    const sealed = new Uint8Array(sent!.data);

    const ola = side(OLA);
    ola.frames.handle({ type: "epochs", epochs: [shared] });
    const unbound: TransformOptions = { role: "receive", kind: "audio", label: "rx:7" };
    const receive = ola.frames.transformFor(unbound);
    const out: EncodedFrame[] = [];
    await receive(frame(sealed), { enqueue: (item) => out.push(item) });
    expect(out).toHaveLength(0);
    ola.frames.handle({ type: "bind", label: "rx:7", sender: MAJA });
    await receive(frame(sealed), { enqueue: (item) => out.push(item) });
    expect(out).toHaveLength(1);
  });

  it("opens the frames of whoever its section is handed to, and none of the one before", async () => {
    const shared = await epoch(1);
    const sealedBy = async (self: string, bytes: number[]) => {
      const sender = side(self);
      sender.frames.handle({ type: "epochs", epochs: [shared] });
      sender.frames.handle({ type: "send", sequence: 1 });
      const [sent] = await sender.pass(SEND_AUDIO, frame(bytes));
      return new Uint8Array(sent!.data);
    };
    const fromMaja = await sealedBy(MAJA, [1, 1, 1]);
    const fromOla = await sealedBy(OLA, [5, 6, 7]);

    // This side's receiver was made for Maja's section, which the media
    // server later gives to Ola.
    const self = side("c3d4e5f6-0718-4293-a4b5-c6d7e8f90a1b");
    self.frames.handle({ type: "epochs", epochs: [shared] });
    const receive = self.frames.transformFor({ ...RECEIVE_AUDIO, label: "rx:7", sender: MAJA });
    self.frames.handle({ type: "bind", label: "rx:7", sender: OLA });
    const out: EncodedFrame[] = [];
    await receive(frame(fromOla), { enqueue: (item) => out.push(item) });
    await receive(frame(fromMaja), { enqueue: (item) => out.push(item) });
    expect(out).toHaveLength(1);
    expect([...new Uint8Array(out[0]!.data)]).toEqual([5, 6, 7]);
  });

  it("never plays a frame of somebody else, a plain frame, or the sender's own reflected back", async () => {
    const shared = await epoch(1);
    const maja = side(MAJA);
    maja.frames.handle({ type: "epochs", epochs: [shared] });
    maja.frames.handle({ type: "send", sequence: 1 });
    const [sent] = await maja.pass(SEND_AUDIO, frame([5, 6, 7]));
    const sealed = new Uint8Array(sent!.data);

    const ola = side(OLA);
    ola.frames.handle({ type: "epochs", epochs: [shared] });
    expect(await ola.pass({ ...RECEIVE_AUDIO, sender: OLA }, frame(sealed))).toHaveLength(0);
    expect(await ola.pass(RECEIVE_AUDIO, frame(new Uint8Array(60).fill(1)))).toHaveLength(0);
    expect(await maja.pass({ ...RECEIVE_AUDIO, sender: MAJA }, frame(sealed))).toHaveLength(1);
    // Maja's own frame bound to Ola, as a server reflecting it would label it.
    expect(await maja.pass({ ...RECEIVE_AUDIO, sender: OLA }, frame(sealed))).toHaveLength(0);
  });

  it("asks for an unknown epoch at most once a second, drops its frames meanwhile, then opens them", async () => {
    const later = await epoch(4);
    const maja = side(MAJA);
    maja.frames.handle({ type: "epochs", epochs: [later] });
    maja.frames.handle({ type: "send", sequence: 4 });
    const send = maja.frames.transformFor(SEND_AUDIO);
    const sealed: Uint8Array[] = [];
    for (let index = 0; index < 4; index += 1) {
      await send(frame([index, 1, 2]), { enqueue: (item) => sealed.push(new Uint8Array(item.data)) });
    }

    const ola = side(OLA);
    const receive = ola.frames.transformFor(RECEIVE_AUDIO);
    const out: EncodedFrame[] = [];
    const enqueue = (item: EncodedFrame) => out.push(item);
    await receive(frame(sealed[0]!), { enqueue });
    await receive(frame(sealed[1]!), { enqueue });
    ola.tick(1_000);
    await receive(frame(sealed[2]!), { enqueue });
    expect(out).toHaveLength(0);
    expect(ola.events.filter((event) => event.type === "need-epoch")).toEqual([
      { type: "need-epoch", sequence: 4 },
      { type: "need-epoch", sequence: 4 },
    ]);

    ola.frames.handle({ type: "epochs", epochs: [later] });
    await receive(frame(sealed[3]!), { enqueue });
    expect(out).toHaveLength(1);
  });

  it("says a receiver is failing once nothing opened for a second, and clears it when a frame opens", async () => {
    const shared = await epoch(1);
    const maja = side(MAJA);
    maja.frames.handle({ type: "epochs", epochs: [shared] });
    maja.frames.handle({ type: "send", sequence: 1 });
    const send = maja.frames.transformFor(SEND_AUDIO);
    const sealed: Uint8Array[] = [];
    for (let index = 0; index < 3; index += 1) {
      await send(frame([index]), { enqueue: (item) => sealed.push(new Uint8Array(item.data)) });
    }

    const ola = side(OLA);
    ola.frames.handle({ type: "epochs", epochs: [shared] });
    const receive = ola.frames.transformFor(RECEIVE_AUDIO);
    const ignore = { enqueue: () => undefined };
    await receive(frame(sealed[0]!), ignore);
    const forged = sealed[1]!.slice();
    forged[5] = (forged[5] ?? 0) ^ 1;
    await receive(frame(forged), ignore);
    expect(ola.events).toEqual([]);
    ola.tick(1_000);
    await receive(frame(forged), ignore);
    expect(ola.events).toEqual([{ type: "status", label: "rx:1", failing: true }]);
    await receive(frame(sealed[2]!), ignore);
    expect(ola.events.at(-1)).toEqual({ type: "status", label: "rx:1", failing: false });
  });

  it("drops a repeated frame without calling the receiver failing", async () => {
    const shared = await epoch(1);
    const maja = side(MAJA);
    maja.frames.handle({ type: "epochs", epochs: [shared] });
    maja.frames.handle({ type: "send", sequence: 1 });
    const [sent] = await maja.pass(SEND_AUDIO, frame([1]));
    const sealed = new Uint8Array(sent!.data);

    const ola = side(OLA);
    ola.frames.handle({ type: "epochs", epochs: [shared] });
    const receive = ola.frames.transformFor(RECEIVE_AUDIO);
    const out: EncodedFrame[] = [];
    await receive(frame(sealed), { enqueue: (item) => out.push(item) });
    ola.tick(5_000);
    await receive(frame(sealed), { enqueue: (item) => out.push(item) });
    expect(out).toHaveLength(1);
    expect(ola.events).toEqual([]);
  });

  it("asks for keyframes at most once a second while a picture fails, and stops once a keyframe opens", async () => {
    const shared = await epoch(1);
    const maja = side(MAJA);
    maja.frames.handle({ type: "epochs", epochs: [shared] });
    maja.frames.handle({ type: "send", sequence: 1 });
    const send = maja.frames.transformFor(SEND_VIDEO);
    const sealed: Uint8Array[] = [];
    for (const type of ["delta", "delta", "key"] as const) {
      await send(frame(new Uint8Array(30).fill(3), { type, mimeType: "video/VP8" }), {
        enqueue: (item) => sealed.push(new Uint8Array(item.data)),
      });
    }

    const ola = side(OLA);
    ola.frames.handle({ type: "epochs", epochs: [shared] });
    const sendKeyFrameRequest = vi.fn(() => Promise.resolve());
    const receive = ola.frames.transformFor(
      { role: "receive", kind: "video", label: "rx:2", sender: MAJA },
      { readable: new ReadableStream(), writable: new WritableStream(), options: SEND_VIDEO, sendKeyFrameRequest },
    );
    const ignore = { enqueue: () => undefined };
    await receive(frame(new Uint8Array(40)), ignore);
    await receive(frame(new Uint8Array(40)), ignore);
    expect(sendKeyFrameRequest).toHaveBeenCalledTimes(1);
    ola.tick(1_000);
    await receive(frame(sealed[0]!, { type: "delta" }), ignore);
    expect(sendKeyFrameRequest).toHaveBeenCalledTimes(2);
    ola.tick(1_000);
    await receive(frame(sealed[2]!, { type: "key" }), ignore);
    ola.tick(1_000);
    await receive(frame(sealed[1]!, { type: "delta" }), ignore);
    expect(sendKeyFrameRequest).toHaveBeenCalledTimes(2);
  });
});
