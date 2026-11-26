import { bytesOf, randomBytes, toBase64, utf8, type Bytes } from "./bytes";
import { agree, exportPublicKey, generateKeyPair, importPublicKey } from "./keys";

/**
 * Wrapping one epoch key for one recipient key (a device or an account).
 *
 * An ephemeral X25519 pair agrees a secret with the recipient's public key;
 * HKDF turns it into a key bound to the conversation, the epoch and the
 * recipient, and AES-GCM seals the epoch key with the same three as its
 * associated data. An envelope therefore opens only for the key it names, in
 * the conversation and epoch it names.
 */

export interface WrapContext {
  conversationId: string;
  epochId: string;
  recipientKeyId: string;
}

export interface SealedEnvelope {
  recipientKeyId: string;
  ephemeralPublicKey: Bytes;
  iv: Bytes;
  ciphertext: Bytes;
}

const EPOCH_KEY_LENGTH = 32;

export function newEpochKey(): Bytes {
  return randomBytes(EPOCH_KEY_LENGTH);
}

export function importEpochKey(raw: Bytes): Promise<CryptoKey> {
  return crypto.subtle.importKey("raw", raw, "AES-GCM", false, ["encrypt", "decrypt"]);
}

/** An epoch key this browser holds, in the two forms it is used in. */
export interface OpenedEpoch {
  /** AES-GCM, for messages and files. */
  key: CryptoKey;
  /**
   * The same bytes as an HKDF key, from which a call derives a key per sender
   * track (lib/voice/e2ee/sframe.ts). Neither form can be exported.
   */
  callBase: CryptoKey;
}

export async function openEpochKey(raw: Bytes): Promise<OpenedEpoch> {
  return {
    key: await importEpochKey(raw),
    callBase: await crypto.subtle.importKey("raw", raw, "HKDF", false, ["deriveBits"]),
  };
}

async function keyEncryptionKey(
  shared: ArrayBuffer,
  context: WrapContext,
  usage: "encrypt" | "decrypt",
): Promise<CryptoKey> {
  const base = await crypto.subtle.importKey("raw", shared, "HKDF", false, ["deriveKey"]);
  return crypto.subtle.deriveKey(
    {
      name: "HKDF",
      hash: "SHA-256",
      salt: utf8(context.epochId),
      info: utf8(`stellarr/e2ee/wrap/v1|${context.conversationId}|${context.recipientKeyId}`),
    },
    base,
    { name: "AES-GCM", length: 256 },
    false,
    [usage],
  );
}

const wrapData = (context: WrapContext) =>
  utf8(`${context.conversationId}|${context.epochId}|${context.recipientKeyId}`);

export async function wrapEpochKey(
  epochKey: Bytes,
  recipientPublicKey: Bytes,
  context: WrapContext,
): Promise<SealedEnvelope> {
  const ephemeral = await generateKeyPair(false);
  const shared = await agree(ephemeral.privateKey, await importPublicKey(recipientPublicKey));
  const key = await keyEncryptionKey(shared, context, "encrypt");
  const iv = randomBytes(12);
  const ciphertext = bytesOf(
    await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: wrapData(context) }, key, epochKey),
  );
  return {
    recipientKeyId: context.recipientKeyId,
    ephemeralPublicKey: await exportPublicKey(ephemeral.publicKey),
    iv,
    ciphertext,
  };
}

/** Opens the envelope. Its caller validates the payload for the epoch version. */
export async function unwrapEpochMaterial(
  envelope: SealedEnvelope,
  privateKey: CryptoKey,
  context: Omit<WrapContext, "recipientKeyId">,
): Promise<Bytes> {
  const full: WrapContext = { ...context, recipientKeyId: envelope.recipientKeyId };
  const shared = await agree(privateKey, await importPublicKey(envelope.ephemeralPublicKey));
  const key = await keyEncryptionKey(shared, full, "decrypt");
  const raw = bytesOf(
    await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: envelope.iv, additionalData: wrapData(full) },
      key,
      envelope.ciphertext,
    ),
  );
  return raw;
}

/** Legacy epochs carry their traffic key directly. Only used to read history. */
export async function unwrapEpochKey(
  envelope: SealedEnvelope,
  privateKey: CryptoKey,
  context: Omit<WrapContext, "recipientKeyId">,
): Promise<OpenedEpoch> {
  const raw = await unwrapEpochMaterial(envelope, privateKey, context);
  if (raw.length !== EPOCH_KEY_LENGTH) throw new Error("An epoch key is 32 bytes.");
  return openEpochKey(raw);
}

/** V2 envelopes carry a salt and the other participant's public key. Even a
 * server supplying its own salt cannot know the traffic key of a verified pair. */
export async function deriveAccountEpoch(
  seed: Bytes,
  self: { userId: string; privateKey: CryptoKey; publicKey: Bytes },
  peer: { userId: string; publicKey: Bytes },
  conversationId: string,
  epochId: string,
): Promise<OpenedEpoch> {
  if (seed.length !== EPOCH_KEY_LENGTH) throw new Error("Invalid epoch salt");
  const parties = [self, peer]
    .sort((a, b) => a.userId < b.userId ? -1 : 1)
    .map(({ userId, publicKey }) => [userId, toBase64(publicKey)]);
  const shared = await agree(self.privateKey, await importPublicKey(peer.publicKey));
  const base = await crypto.subtle.importKey("raw", shared, "HKDF", false, ["deriveBits"]);
  const info = utf8(JSON.stringify(["stellarr/e2ee/account-epoch/v2", conversationId, epochId, parties]));
  const raw = await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: seed, info }, base, 256);
  return openEpochKey(bytesOf(raw));
}
