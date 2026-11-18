import { bytesOf, randomBytes, utf8, type Bytes } from "./bytes";
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

/** Throws when the envelope is not for this private key, or was changed. */
export async function unwrapEpochKey(
  envelope: SealedEnvelope,
  privateKey: CryptoKey,
  context: Omit<WrapContext, "recipientKeyId">,
): Promise<CryptoKey> {
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
  if (raw.length !== EPOCH_KEY_LENGTH) throw new Error("An epoch key is 32 bytes.");
  return importEpochKey(raw);
}
