import { createCipheriv, hkdfSync } from "node:crypto";

import { describe, expect, it } from "vitest";

import {
  clearPrefixOf,
  deriveFrameKey,
  epochOfKeyId,
  headerLength,
  keyIdHex,
  keyIdOf,
  nonceOf,
  openFrame,
  parseFrame,
  readHeader,
  REPLAY_WINDOW,
  ReplayWindow,
  sealFrame,
  writeHeader,
  type KeyId,
} from "./sframe";

const EPOCH_KEY = Uint8Array.from({ length: 32 }, (_, index) => index);
const EPOCH = "4f2a8c7e-1b3d-4e5f-8a9b-0c1d2e3f4a5b";
const CONVERSATION = "0b5c1c1e-7a6f-4b0e-9f3a-1d2c3b4a5f60";
const SENDER = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
const OTHER = "6f5e4d3c-2b1a-4f9e-8d7c-6b5a4f3e2d1c";
/** Epoch sequence 1 and the random bytes a1..a6. */
const KID = keyIdOf(1, Uint8Array.of(0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6));
const PLAIN = Uint8Array.from({ length: 48 }, (_, index) => index);

const hex = (bytes: Uint8Array) => Buffer.from(bytes).toString("hex");
const fromHex = (text: string) => Uint8Array.from(Buffer.from(text, "hex"));
const kid = (value: bigint): KeyId => ({ hi: Number(value >> 32n), lo: Number(value & 0xffff_ffffn) });

const base = () => crypto.subtle.importKey("raw", EPOCH_KEY, "HKDF", false, ["deriveBits"]);
const context = (senderId = SENDER, epochId = EPOCH, conversationId = CONVERSATION) => ({
  epochId,
  conversationId,
  senderId,
});

/** The same frame made with Node's own HKDF and AES-GCM, independently of the code under test. */
function referenceFrame(counter: number, clear: number, plain = PLAIN): Uint8Array {
  const info = `stellarr/e2ee/call/v1|${CONVERSATION}|${SENDER}|${keyIdHex(KID)}`;
  const okm = Buffer.from(hkdfSync("sha256", EPOCH_KEY, Buffer.from(EPOCH), Buffer.from(info), 44));
  const header = new Uint8Array(headerLength(KID, counter));
  writeHeader(header, 0, KID, counter);
  const iv = Buffer.from(okm.subarray(32));
  const counterBytes = Buffer.alloc(8);
  counterBytes.writeBigUInt64BE(BigInt(counter));
  for (let index = 0; index < 8; index += 1) iv[4 + index] = (iv[4 + index] ?? 0) ^ (counterBytes[index] ?? 0);
  const aad = Buffer.concat([plain.subarray(0, clear), header, Buffer.of(clear)]);
  const cipher = createCipheriv("aes-256-gcm", okm.subarray(0, 32), iv);
  cipher.setAAD(aad);
  const body = Buffer.concat([cipher.update(plain.subarray(clear)), cipher.final(), cipher.getAuthTag()]);
  return Uint8Array.from(Buffer.concat([plain.subarray(0, clear), header, body, Buffer.of(clear)]));
}

describe("frame header", () => {
  // Rows of RFC 9605 appendix C.1, within the counters a sender of ours reaches.
  const vectors: [bigint, number, string][] = [
    [0n, 0, "00"],
    [0n, 1, "01"],
    [0n, 0xff, "08ff"],
    [0n, 0x100, "090100"],
    [0n, 0xffff, "09ffff"],
    [0n, 0x1_0000, "0a010000"],
    [0n, 0xffff_ffff, "0bffffffff"],
    [0n, 0x1_0000_0000, "0c0100000000"],
    [0n, 2 ** 48, "0e01000000000000"],
    [1n, 0, "10"],
    [1n, 1, "11"],
    [0xffn, 0, "80ff"],
    [0xffn, 0xff, "88ffff"],
    [0x100n, 0x100, "9901000100"],
    [0xffffn, 0x1_0000, "9affff010000"],
    [0x1_0000n, 0, "a0010000"],
    [0xffff_ffffn, 0x100, "b9ffffffff0100"],
    [0x1_0000_0000n, 0, "c00100000000"],
    [0xffff_ffff_ffffn, 0xffff_ffff, "dbffffffffffffffffffff"],
    [0x1_0000_0000_0000n, 0x100, "e9010000000000000100"],
    [0xffff_ffff_ffff_ffffn, 0, "f0ffffffffffffffff"],
  ];

  it.each(vectors)("kid %s, counter %s is written as %s and read back", (value, counter, expected) => {
    const id = kid(value);
    const out = new Uint8Array(headerLength(id, counter));
    expect(writeHeader(out, 0, id, counter)).toBe(out.length);
    expect(hex(out)).toBe(expected);
    expect(readHeader(fromHex(expected), 0)).toEqual({ kid: id, counter, end: out.length });
  });

  it("refuses a header cut short and a counter past the exact range", () => {
    expect(readHeader(fromHex("9901000100"), 0, 4)).toBeNull();
    expect(readHeader(fromHex("0f0100000000000000"), 0)).toBeNull();
  });

  it("puts the epoch's sequence in the key id's top 16 bits", () => {
    expect(keyIdHex(KID)).toBe("0001a1a2a3a4a5a6");
    expect(epochOfKeyId(keyIdOf(0x1_2345, new Uint8Array(6)))).toBe(0x2345);
  });

  it("XORs the counter into the last eight bytes of the salt", () => {
    const salt = fromHex("000102030405060708090a0b");
    expect(hex(nonceOf(salt, 0))).toBe("000102030405060708090a0b");
    expect(hex(nonceOf(salt, 0x1_0000_0001))).toBe("000102030405060608090a0a");
    expect(hex(nonceOf(salt, 2 ** 48 + 0x100))).toBe("000102030404060708090b0b");
  });
});

describe("clear prefix", () => {
  it("keeps nothing of Opus or VP9, the VP8 tag, and refuses every other picture", () => {
    expect(clearPrefixOf("audio", undefined, false, 120)).toBe(0);
    expect(clearPrefixOf("video", "video/VP9", true, 9000)).toBe(0);
    expect(clearPrefixOf("video", "video/VP8", true, 9000)).toBe(10);
    expect(clearPrefixOf("video", "video/vp8", false, 900)).toBe(3);
    expect(clearPrefixOf("video", "video/VP8", true, 6)).toBe(6);
    expect(clearPrefixOf("video", "video/H264", true, 9000)).toBeNull();
    expect(clearPrefixOf("video", "video/AV1", false, 900)).toBeNull();
    expect(clearPrefixOf("video", undefined, false, 900)).toBeNull();
  });
});

describe("frame sealing", () => {
  it("derives the key and salt the format names", async () => {
    const derived = await deriveFrameKey(await base(), context(), KID, "encrypt");
    expect(hex(derived.salt)).toBe("9a67a4989c61ceb198701233");
  });

  it("matches a pinned VP8 keyframe", async () => {
    const frameKey = await deriveFrameKey(await base(), context(), KID, "encrypt");
    const sealed = await sealFrame(frameKey, KID, 300, PLAIN, 10);
    expect(hex(sealed)).toBe(
      "00010203040506070809e901a1a2a3a4a5a6012c3539a5f979f773d9348a6dee3b24147cec0e8ee8e9ca4603504f" +
        "3eb76c0f8a10820b502bd9af01e3ac30261cfe9f9ba5a9fcda5b97080a",
    );
  });

  it.each([
    ["Opus", 0, 0],
    ["VP9", 7, 0],
    ["a VP8 delta frame", 8, 3],
    ["a VP8 keyframe", 70_000, 10],
  ])("seals %s as Node's own AES-GCM does", async (_name, counter, clear) => {
    const frameKey = await deriveFrameKey(await base(), context(), KID, "encrypt");
    expect(hex(await sealFrame(frameKey, KID, counter, PLAIN, clear))).toBe(hex(referenceFrame(counter, clear)));
  });

  it("opens what it sealed back to the frame the encoder made", async () => {
    const sealing = await deriveFrameKey(await base(), context(), KID, "encrypt");
    const opening = await deriveFrameKey(await base(), context(), KID, "decrypt");
    for (const clear of [0, 3, 10]) {
      const sealed = await sealFrame(sealing, KID, 42, PLAIN, clear);
      const parsed = parseFrame(sealed);
      expect(parsed).toMatchObject({ clear, kid: KID, counter: 42 });
      expect(hex(new Uint8Array(await openFrame(opening, parsed!, sealed)))).toBe(hex(PLAIN));
    }
  });

  it("does not open once any part changed: prefix, header, ciphertext, tag or length byte", async () => {
    const sealing = await deriveFrameKey(await base(), context(), KID, "encrypt");
    const opening = await deriveFrameKey(await base(), context(), KID, "decrypt");
    const sealed = await sealFrame(sealing, KID, 9, PLAIN, 3);
    const headerEnd = parseFrame(sealed)!.headerEnd;
    for (const at of [0, 3, headerEnd - 1, headerEnd + 2, sealed.length - 3]) {
      const changed = sealed.slice();
      changed[at] = (changed[at] ?? 0) ^ 0x01;
      const parsed = parseFrame(changed);
      if (parsed === null) continue;
      await expect(openFrame(opening, parsed, changed)).rejects.toThrow();
    }
    // A shorter prefix claimed by the length byte moves the header and fails too.
    const relabelled = sealed.slice();
    relabelled[relabelled.length - 1] = 0;
    const parsed = parseFrame(relabelled);
    if (parsed !== null) await expect(openFrame(opening, parsed, relabelled)).rejects.toThrow();
  });

  it("does not open for another sender, conversation or epoch: a reflected frame fails", async () => {
    const sealing = await deriveFrameKey(await base(), context(), KID, "encrypt");
    const sealed = await sealFrame(sealing, KID, 1, PLAIN, 0);
    const parsed = parseFrame(sealed)!;
    for (const other of [context(OTHER), context(SENDER, OTHER), context(SENDER, EPOCH, OTHER)]) {
      const wrong = await deriveFrameKey(await base(), other, KID, "decrypt");
      await expect(openFrame(wrong, parsed, sealed)).rejects.toThrow();
    }
  });

  it("does not parse what is too short, claims a prefix over 10 bytes or has no room for its header", () => {
    expect(parseFrame(new Uint8Array(17))).toBeNull();
    const long = new Uint8Array(40);
    long[39] = 11;
    expect(parseFrame(long)).toBeNull();
    const cut = new Uint8Array(18);
    cut[0] = 0xff;
    expect(parseFrame(cut)).toBeNull();
  });
});

describe("replay window", () => {
  it("accepts a counter once, late ones within the window, and none behind it", () => {
    const window = new ReplayWindow();
    expect(window.accepts(5)).toBe(true);
    window.mark(5);
    expect(window.accepts(5)).toBe(false);
    expect(window.accepts(3)).toBe(true);
    window.mark(3);
    expect(window.accepts(3)).toBe(false);
    window.mark(5 + REPLAY_WINDOW);
    expect(window.accepts(5)).toBe(false);
    expect(window.accepts(6)).toBe(true);
    expect(window.accepts(4 + REPLAY_WINDOW)).toBe(true);
    expect(window.accepts(5 + REPLAY_WINDOW)).toBe(false);
  });

  it("moves only for frames that were marked", () => {
    const window = new ReplayWindow();
    window.mark(1);
    expect(window.accepts(1_000_000)).toBe(true);
    expect(window.accepts(2)).toBe(true);
  });

  it("forgets the slots a large step passes over", () => {
    const window = new ReplayWindow();
    window.mark(0);
    window.mark(10);
    window.mark(10 + 3 * REPLAY_WINDOW);
    expect(window.accepts(10 + 3 * REPLAY_WINDOW - 1)).toBe(true);
    expect(window.accepts(10 + 2 * REPLAY_WINDOW + 1)).toBe(true);
  });
});
