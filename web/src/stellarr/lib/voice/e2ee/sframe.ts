/**
 * The frame format of an encrypted call, and nothing else.
 *
 * Every encoded frame a sender hands to its packetizer becomes
 *
 *     clear prefix | header | AES-256-GCM ciphertext | tag (16 B) | prefix length (1 B)
 *
 * - The clear prefix is what the receiving browser's depacketizer reads
 *   before the receiving transform runs: nothing for Opus and VP9, the 3-byte
 *   frame tag of a VP8 delta frame and 10 bytes of a VP8 keyframe (the tag,
 *   the start code and the size). H.264 and AV1 cannot be sent this way, so a
 *   frame of any other codec has no prefix rule and is not sent at all.
 * - The header is the one of RFC 9605 section 4.3: a config byte, then the
 *   key id and the counter in as few big-endian bytes as they take.
 * - The prefix, the header and the trailing length byte are the associated
 *   data, so none of them can be changed on the way without the frame failing.
 *
 * A key id is 64 bits: the low 16 bits of the epoch's sequence, then 48
 * random bits that a sender draws for each track, each connection and each
 * epoch. Its key and its nonce salt come from HKDF-SHA-256 of the epoch key
 * with the epoch id as the salt and, as the info, the label, the conversation,
 * the sending user and the key id. The nonce is the salt with the counter
 * XORed into its last eight bytes, so it never repeats under one key; a new
 * key id starts its counter again at zero.
 *
 * No imports and only erasable TypeScript: the frame worker, the unit tests
 * and the media smoke harness all run this very file.
 */

/** The HKDF info label of a frame key; a new format gets a new label. */
export const CALL_KEY_LABEL = "stellarr/e2ee/call/v1";
export const TAG_BYTES = 16;
export const NONCE_BYTES = 12;
export const KEY_BYTES = 32;
/** The longest clear prefix: a VP8 keyframe's tag, start code and size. */
export const MAX_CLEAR_BYTES = 10;
/** The largest counter this side writes or accepts: every value stays an exact number. */
export const MAX_COUNTER = Number.MAX_SAFE_INTEGER;

const TWO_32 = 0x1_0000_0000;

/** A 64-bit key id as its upper and lower 32 bits, both unsigned. */
export interface KeyId {
  hi: number;
  lo: number;
}

/** The key id of a sender of epoch `sequence`, from 48 random bits given as six bytes. */
export function keyIdOf(sequence: number, random: Uint8Array): KeyId {
  const r = (at: number) => random[at] ?? 0;
  return {
    hi: (((sequence & 0xffff) << 16) | (r(0) << 8) | r(1)) >>> 0,
    lo: ((r(2) << 24) | (r(3) << 16) | (r(4) << 8) | r(5)) >>> 0,
  };
}

/** The 16 bits of epoch sequence a key id carries. */
export function epochOfKeyId(kid: KeyId): number {
  return kid.hi >>> 16;
}

/** Sixteen lowercase hex digits: how a key id is written into the key derivation. */
export function keyIdHex(kid: KeyId): string {
  return kid.hi.toString(16).padStart(8, "0") + kid.lo.toString(16).padStart(8, "0");
}

/** Bytes a value takes big-endian, at least one; `hi` and `lo` are its 32-bit halves. */
function lengthOf(hi: number, lo: number): number {
  if (hi !== 0) return hi >= 0x100_0000 ? 8 : hi >= 0x1_0000 ? 7 : hi >= 0x100 ? 6 : 5;
  return lo >= 0x100_0000 ? 4 : lo >= 0x1_0000 ? 3 : lo >= 0x100 ? 2 : 1;
}

function writeValue(out: Uint8Array, at: number, hi: number, lo: number, length: number): void {
  for (let index = 0; index < length; index += 1) {
    const shift = index * 8;
    out[at + length - 1 - index] = shift < 32 ? (lo >>> shift) & 0xff : (hi >>> (shift - 32)) & 0xff;
  }
}

/** The length of the header of `kid` and `counter`. */
export function headerLength(kid: KeyId, counter: number): number {
  const kidBytes = kid.hi === 0 && kid.lo < 8 ? 0 : lengthOf(kid.hi, kid.lo);
  const counterBytes = counter < 8 ? 0 : lengthOf(Math.floor(counter / TWO_32), counter >>> 0);
  return 1 + kidBytes + counterBytes;
}

/**
 * Writes the RFC 9605 header at `at`: X|K|Y|C, then the key id when it does
 * not fit in K and the counter when it does not fit in C. Returns its length.
 */
export function writeHeader(out: Uint8Array, at: number, kid: KeyId, counter: number): number {
  let config = 0;
  let offset = at + 1;
  if (kid.hi === 0 && kid.lo < 8) {
    config |= kid.lo << 4;
  } else {
    const length = lengthOf(kid.hi, kid.lo);
    config |= 0x80 | ((length - 1) << 4);
    writeValue(out, offset, kid.hi, kid.lo, length);
    offset += length;
  }
  if (counter < 8) {
    config |= counter;
  } else {
    const hi = Math.floor(counter / TWO_32);
    const lo = counter >>> 0;
    const length = lengthOf(hi, lo);
    config |= 0x08 | (length - 1);
    writeValue(out, offset, hi, lo, length);
    offset += length;
  }
  out[at] = config;
  return offset - at;
}

export interface ParsedHeader {
  kid: KeyId;
  counter: number;
  /** The index just past the header. */
  end: number;
}

function readValue(data: Uint8Array, at: number, length: number): KeyId {
  let hi = 0;
  let lo = 0;
  for (let index = 0; index < length; index += 1) {
    hi = ((hi << 8) | (lo >>> 24)) >>> 0;
    lo = ((lo << 8) | (data[at + index] ?? 0)) >>> 0;
  }
  return { hi, lo };
}

/** Reads a header that starts at `at` and ends by `limit`; null when it does not fit or is out of range. */
export function readHeader(data: Uint8Array, at: number, limit = data.length): ParsedHeader | null {
  if (at >= limit) return null;
  const config = data[at] ?? 0;
  let offset = at + 1;
  let kid: KeyId;
  if (config & 0x80) {
    const length = ((config >> 4) & 0x07) + 1;
    if (offset + length > limit) return null;
    kid = readValue(data, offset, length);
    offset += length;
  } else {
    kid = { hi: 0, lo: (config >> 4) & 0x07 };
  }
  let counter: number;
  if (config & 0x08) {
    const length = (config & 0x07) + 1;
    if (offset + length > limit) return null;
    const value = readValue(data, offset, length);
    // Past 2^53 a counter is not an exact number; no sender of ours gets there.
    if (value.hi >= 0x20_0000) return null;
    counter = value.hi * TWO_32 + value.lo;
    offset += length;
  } else {
    counter = config & 0x07;
  }
  return { kid, counter, end: offset };
}

/** The salt with `counter` XORed into its last eight bytes, big-endian. */
export function nonceOf(salt: Uint8Array, counter: number): Uint8Array<ArrayBuffer> {
  const iv = new Uint8Array(NONCE_BYTES);
  iv.set(salt.subarray(0, NONCE_BYTES));
  const hi = Math.floor(counter / TWO_32);
  const lo = counter >>> 0;
  for (let index = 0; index < 4; index += 1) {
    const shift = index * 8;
    iv[11 - index] = (iv[11 - index] ?? 0) ^ ((lo >>> shift) & 0xff);
    iv[7 - index] = (iv[7 - index] ?? 0) ^ ((hi >>> shift) & 0xff);
  }
  return iv;
}

/**
 * How many leading bytes of a frame stay in the clear, or null for a frame
 * that cannot be encrypted and must not be sent: a picture of any codec but
 * VP8 and VP9. `mimeType` is the codec's, as `video/VP8`.
 */
export function clearPrefixOf(
  kind: "audio" | "video",
  mimeType: string | undefined,
  keyframe: boolean,
  length: number,
): number | null {
  if (kind === "audio") return 0;
  const codec = mimeType?.toLowerCase();
  if (codec === "video/vp9") return 0;
  if (codec === "video/vp8") return Math.min(length, keyframe ? 10 : 3);
  return null;
}

/** The HKDF info of a frame key. Every id is lowercase, as the server writes it. */
export function frameKeyInfo(conversationId: string, senderId: string, kid: KeyId): string {
  return `${CALL_KEY_LABEL}|${conversationId}|${senderId}|${keyIdHex(kid)}`;
}

export interface FrameKey {
  key: CryptoKey;
  salt: Uint8Array<ArrayBuffer>;
}

/**
 * The AES-GCM key and nonce salt of `kid` from the epoch's HKDF base, which is
 * the epoch key's raw bytes imported as a non-extractable HKDF key.
 */
export async function deriveFrameKey(
  base: CryptoKey,
  context: { epochId: string; conversationId: string; senderId: string },
  kid: KeyId,
  usage: "encrypt" | "decrypt",
): Promise<FrameKey> {
  const encoder = new TextEncoder();
  const bits = new Uint8Array(
    await crypto.subtle.deriveBits(
      {
        name: "HKDF",
        hash: "SHA-256",
        salt: encoder.encode(context.epochId),
        info: encoder.encode(frameKeyInfo(context.conversationId, context.senderId, kid)),
      },
      base,
      (KEY_BYTES + NONCE_BYTES) * 8,
    ),
  );
  const key = await crypto.subtle.importKey("raw", bits.subarray(0, KEY_BYTES), "AES-GCM", false, [usage]);
  return { key, salt: bits.slice(KEY_BYTES) };
}

/**
 * Seals one frame. `counter` must be taken (and the sender's moved on)
 * before this is called, so two frames never share one.
 */
export async function sealFrame(
  frameKey: FrameKey,
  kid: KeyId,
  counter: number,
  data: Uint8Array<ArrayBuffer>,
  clear: number,
): Promise<Uint8Array<ArrayBuffer>> {
  const head = clear + headerLength(kid, counter);
  const aad = new Uint8Array(head + 1);
  aad.set(data.subarray(0, clear), 0);
  writeHeader(aad, clear, kid, counter);
  aad[head] = clear;
  const sealed = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: nonceOf(frameKey.salt, counter), additionalData: aad, tagLength: TAG_BYTES * 8 },
    frameKey.key,
    data.subarray(clear),
  );
  const out = new Uint8Array(head + sealed.byteLength + 1);
  out.set(aad.subarray(0, head), 0);
  out.set(new Uint8Array(sealed), head);
  out[out.length - 1] = clear;
  return out;
}

export interface ParsedFrame {
  clear: number;
  kid: KeyId;
  counter: number;
  /** The index just past the header, where the ciphertext starts. */
  headerEnd: number;
}

/** The parts of an encrypted frame, or null for anything that is not one. */
export function parseFrame(data: Uint8Array): ParsedFrame | null {
  const length = data.length;
  // The length byte, a config byte and a tag at the very least.
  if (length < 2 + TAG_BYTES) return null;
  const clear = data[length - 1] ?? 0;
  if (clear > MAX_CLEAR_BYTES) return null;
  const header = readHeader(data, clear, length - 1 - TAG_BYTES);
  if (header === null) return null;
  return { clear, kid: header.kid, counter: header.counter, headerEnd: header.end };
}

/**
 * Opens a parsed frame and returns what the decoder gets: the clear prefix
 * followed by the plaintext. Rejects when the tag does not verify.
 */
export async function openFrame(
  frameKey: FrameKey,
  frame: ParsedFrame,
  data: Uint8Array<ArrayBuffer>,
): Promise<ArrayBuffer> {
  const { clear, headerEnd, counter } = frame;
  const aad = new Uint8Array(headerEnd + 1);
  aad.set(data.subarray(0, headerEnd), 0);
  aad[headerEnd] = clear;
  const opened = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv: nonceOf(frameKey.salt, counter), additionalData: aad, tagLength: TAG_BYTES * 8 },
    frameKey.key,
    data.subarray(headerEnd, data.length - 1),
  );
  // Opus and VP9 keep nothing in the clear: the plaintext is the frame.
  if (clear === 0) return opened;
  const out = new Uint8Array(clear + opened.byteLength);
  out.set(data.subarray(0, clear), 0);
  out.set(new Uint8Array(opened), clear);
  return out.buffer;
}

/** How far behind the newest counter a frame of one key id may still arrive. */
export const REPLAY_WINDOW = 128;

/**
 * The counters of one key id seen so far: a frame is accepted once, and not
 * at all once it falls more than the window behind the newest. Only a frame
 * whose tag verified is marked, so a forged counter cannot move the window.
 */
export class ReplayWindow {
  private newest = -1;
  private readonly seen = new Uint32Array(REPLAY_WINDOW / 32);

  accepts(counter: number): boolean {
    if (counter > this.newest) return true;
    if (this.newest - counter >= REPLAY_WINDOW) return false;
    const slot = counter % REPLAY_WINDOW;
    return ((this.seen[slot >>> 5] ?? 0) & (1 << (slot & 31))) === 0;
  }

  mark(counter: number): void {
    if (counter > this.newest) {
      const step = counter - this.newest;
      if (step >= REPLAY_WINDOW || this.newest < 0) {
        this.seen.fill(0);
      } else {
        for (let value = this.newest + 1; value < counter; value += 1) this.clear(value % REPLAY_WINDOW);
      }
      this.newest = counter;
      this.clear(counter % REPLAY_WINDOW);
    }
    const slot = counter % REPLAY_WINDOW;
    this.seen[slot >>> 5] = (this.seen[slot >>> 5] ?? 0) | (1 << (slot & 31));
  }

  private clear(slot: number): void {
    this.seen[slot >>> 5] = (this.seen[slot >>> 5] ?? 0) & ~(1 << (slot & 31));
  }
}
