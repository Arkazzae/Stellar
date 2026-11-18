import { createHash, hkdfSync, pbkdf2Sync } from "node:crypto";

import { describe, expect, it } from "vitest";

import { decryptFile, encryptFile, OPAQUE_TYPE, shownType } from "./attachment";
import { bytesOf, concatBytes, fromBase64, toBase64, utf8 } from "./bytes";
import { importEpochKey, newEpochKey, unwrapEpochKey, wrapEpochKey } from "./epoch";
import { exportPrivateKey, exportPublicKey, generateKeyPair, privateKeyMatches } from "./keys";
import { decryptPayload, encryptPayload, parsePayload, type MessagePayload } from "./message";
import { derivePinKeys, openBackup, sealBackup, verifierOf } from "./pin";
import { safetyCode } from "./safetyCode";

const CONVERSATION = "0b5c1c1e-7a6f-4b0e-9f3a-1d2c3b4a5f60";
const EPOCH = "4f2a8c7e-1b3d-4e5f-8a9b-0c1d2e3f4a5b";
const RECIPIENT = "9e8d7c6b-5a49-4382-a1b0-c9d8e7f6a5b4";
const AUTHOR = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
const OTHER = "6f5e4d3c-2b1a-4f9e-8d7c-6b5a4f3e2d1c";
const FEW_ITERATIONS = 1_000;

const flip = (bytes: Uint8Array, at = 0) => {
  const copy = new Uint8Array(bytes);
  copy[at] = (copy[at] ?? 0) ^ 0x01;
  return copy;
};

describe("epoch key wrapping", () => {
  const context = { conversationId: CONVERSATION, epochId: EPOCH, recipientKeyId: RECIPIENT };

  it("opens for the recipient's private key and yields a key that decrypts what the raw key sealed", async () => {
    const recipient = await generateKeyPair(false);
    const raw = newEpochKey();
    const envelope = await wrapEpochKey(raw, await exportPublicKey(recipient.publicKey), context);
    const opened = await unwrapEpochKey(envelope, recipient.privateKey, context);

    const payload: MessagePayload = { v: 1, text: "ok", attachments: [] };
    const message = { conversationId: CONVERSATION, epochId: EPOCH, authorId: AUTHOR };
    const sealed = await encryptPayload(await importEpochKey(raw), message, payload);
    expect(await decryptPayload(opened, message, sealed.iv, sealed.ciphertext)).toEqual(payload);
  });

  it("does not open for another key, in another conversation or epoch, or once changed", async () => {
    const recipient = await generateKeyPair(false);
    const stranger = await generateKeyPair(false);
    const envelope = await wrapEpochKey(newEpochKey(), await exportPublicKey(recipient.publicKey), context);

    await expect(unwrapEpochKey(envelope, stranger.privateKey, context)).rejects.toThrow();
    await expect(
      unwrapEpochKey(envelope, recipient.privateKey, { ...context, conversationId: OTHER }),
    ).rejects.toThrow();
    await expect(unwrapEpochKey(envelope, recipient.privateKey, { ...context, epochId: OTHER })).rejects.toThrow();
    await expect(
      unwrapEpochKey({ ...envelope, recipientKeyId: OTHER }, recipient.privateKey, context),
    ).rejects.toThrow();
    await expect(
      unwrapEpochKey({ ...envelope, ciphertext: flip(envelope.ciphertext) }, recipient.privateKey, context),
    ).rejects.toThrow();
  });
});

describe("message encryption", () => {
  const context = { conversationId: CONVERSATION, epochId: EPOCH, authorId: AUTHOR };
  const payload: MessagePayload = {
    v: 1,
    text: "login: klub.kajakowy",
    attachments: [{ id: "u-1", key: "k", iv: "i", name: "a/b.png", type: "image/png", size: 3, width: 2 }],
  };

  it("round-trips the plaintext, cleaning a file name of path separators", async () => {
    const key = await importEpochKey(newEpochKey());
    const sealed = await encryptPayload(key, context, payload);
    const opened = await decryptPayload(key, context, sealed.iv, sealed.ciphertext);
    expect(opened.text).toBe("login: klub.kajakowy");
    expect(opened.attachments[0]).toMatchObject({ id: "u-1", name: "a_b.png", width: 2 });
  });

  it("refuses another epoch key, another author or conversation, and changed bytes", async () => {
    const key = await importEpochKey(newEpochKey());
    const sealed = await encryptPayload(key, context, payload);
    await expect(
      decryptPayload(await importEpochKey(newEpochKey()), context, sealed.iv, sealed.ciphertext),
    ).rejects.toThrow();
    await expect(decryptPayload(key, { ...context, authorId: OTHER }, sealed.iv, sealed.ciphertext)).rejects.toThrow();
    await expect(
      decryptPayload(key, { ...context, conversationId: OTHER }, sealed.iv, sealed.ciphertext),
    ).rejects.toThrow();
    await expect(decryptPayload(key, context, sealed.iv, flip(sealed.ciphertext, 3))).rejects.toThrow();
    await expect(decryptPayload(key, context, flip(sealed.iv), sealed.ciphertext)).rejects.toThrow();
  });

  it("takes only version 1 with a well-formed list of files", () => {
    expect(parsePayload({ v: 2, text: "x", attachments: [] })).toBeNull();
    expect(parsePayload({ v: 1, text: 3, attachments: [] })).toBeNull();
    expect(parsePayload({ v: 1, text: "x", attachments: [{ id: "u" }] })).toBeNull();
    expect(parsePayload({ v: 1, text: "x" })).toEqual({ v: 1, text: "x", attachments: [] });
  });
});

describe("file encryption", () => {
  it("round-trips the bytes under a key of the file's own", async () => {
    const sealed = await encryptFile(new Blob(["tajny raport"]));
    const data = await sealed.data.arrayBuffer();
    expect(new TextDecoder().decode(await decryptFile(data, sealed.key, sealed.iv))).toBe("tajny raport");
    // What the server stores is the ciphertext and the 16-byte tag, nothing more.
    expect(data.byteLength).toBe("tajny raport".length + 16);
    expect(sealed.data.type).toBe(OPAQUE_TYPE);
  });

  it("refuses another file's key and changed bytes", async () => {
    const first = await encryptFile(new Blob(["one"]));
    const second = await encryptFile(new Blob(["two"]));
    const data = new Uint8Array(await first.data.arrayBuffer());
    await expect(decryptFile(data.buffer, second.key, first.iv)).rejects.toThrow();
    await expect(decryptFile(flip(data).buffer, first.key, first.iv)).rejects.toThrow();
  });

  it("shows only kinds the page draws itself, never one that can carry script", () => {
    expect(shownType("image/png")).toBe("image/png");
    expect(shownType("video/mp4; codecs=avc1")).toBe("video/mp4");
    expect(shownType("image/svg+xml")).toBe(OPAQUE_TYPE);
    expect(shownType("text/html")).toBe(OPAQUE_TYPE);
    expect(shownType("")).toBe(OPAQUE_TYPE);
  });
});

describe("the PIN backup", () => {
  it("derives the keys as PBKDF2-SHA-256 and HKDF-SHA-256 with an empty salt and fixed labels", async () => {
    const salt = utf8("0123456789abcdef");
    const { authKey } = await derivePinKeys("246802", salt, FEW_ITERATIONS);
    const master = pbkdf2Sync("246802", salt, FEW_ITERATIONS, 32, "sha256");
    const expected = Buffer.from(hkdfSync("sha256", master, Buffer.alloc(0), "stellarr/e2ee/pin/auth/v1", 32));
    expect(toBase64(authKey)).toBe(expected.toString("base64"));
  });

  it("opens with the right PIN and not with a wrong one", async () => {
    const pair = await generateKeyPair(true);
    const pkcs8 = await exportPrivateKey(pair.privateKey);
    const backup = await sealBackup(pkcs8, "135790", FEW_ITERATIONS);

    const right = await derivePinKeys("135790", backup.salt, backup.iterations);
    expect(await verifierOf(right.authKey)).toEqual(backup.verifier);
    expect(await openBackup(right.encKey, backup.iv, backup.ciphertext)).toEqual(pkcs8);

    const wrong = await derivePinKeys("135791", backup.salt, backup.iterations);
    expect(await verifierOf(wrong.authKey)).not.toEqual(backup.verifier);
    await expect(openBackup(wrong.encKey, backup.iv, backup.ciphertext)).rejects.toThrow();
  });

  it("tells whether a decrypted private key belongs to the account's public key", async () => {
    const account = await generateKeyPair(true);
    const other = await generateKeyPair(true);
    const pkcs8 = await exportPrivateKey(account.privateKey);
    expect(await privateKeyMatches(pkcs8, await exportPublicKey(account.publicKey))).toBe(true);
    expect(await privateKeyMatches(pkcs8, await exportPublicKey(other.publicKey))).toBe(false);
    expect(await privateKeyMatches(utf8("not a key"), await exportPublicKey(account.publicKey))).toBe(false);
  });
});

describe("the safety code", () => {
  const party = (userId: string, fill: number) => ({ userId, publicKey: new Uint8Array(32).fill(fill) });

  it("is 12 groups of 5 digits, the same whoever computes it", async () => {
    const left = party(AUTHOR, 7);
    const right = party(OTHER, 9);
    const one = await safetyCode(left, right);
    const two = await safetyCode(right, left);
    expect(one).toEqual(two);
    expect(one).toHaveLength(12);
    for (const group of one) expect(group).toMatch(/^\d{5}$/);
  });

  it("is the last 60 decimal digits of SHA-256 over the label and both people in user id order", async () => {
    const left = party(OTHER, 1);
    const right = party(AUTHOR, 2);
    const [first, second] = [right, left];
    const digest = createHash("sha256")
      .update(concatBytes(utf8("stellarr/e2ee/safety/v1"), utf8(first.userId), first.publicKey, utf8(second.userId), second.publicKey))
      .digest("hex");
    const digits = (BigInt(`0x${digest}`) % 10n ** 60n).toString().padStart(60, "0");
    expect((await safetyCode(left, right)).join("")).toBe(digits);
  });

  it("changes when either account key changes, and only then", async () => {
    const base = await safetyCode(party(AUTHOR, 7), party(OTHER, 9));
    expect(await safetyCode(party(AUTHOR, 7), party(OTHER, 9))).toEqual(base);
    expect(await safetyCode(party(AUTHOR, 7), party(OTHER, 8))).not.toEqual(base);
  });
});

describe("bytes", () => {
  it("round-trips base64 across chunk boundaries", () => {
    const bytes = bytesOf(new Uint8Array(70_000).map((_, index) => index % 251).buffer);
    expect(fromBase64(toBase64(bytes))).toEqual(bytes);
  });
});
