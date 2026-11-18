import { bytesOf, fromUtf8, randomBytes, utf8, type Bytes } from "./bytes";

/**
 * The plaintext of an encrypted message, version 1.
 *
 * Every field the server would otherwise have seen (the text, and each file's
 * name, type, size and dimensions) lives in here; the server keeps only the
 * ciphertext and the opaque upload ids.
 */
export interface PayloadPoster {
  id: string;
  /** base64 AES-256 key of this upload alone. */
  key: string;
  iv: string;
  type: string;
  size: number;
}

export interface PayloadAttachment {
  id: string;
  key: string;
  iv: string;
  name: string;
  type: string;
  size: number;
  width?: number;
  height?: number;
  duration?: number;
  voice?: boolean;
  poster?: PayloadPoster;
}

export interface MessagePayload {
  v: 1;
  text: string;
  attachments: PayloadAttachment[];
}

export interface MessageContext {
  conversationId: string;
  epochId: string;
  /** The author's user id: a message cannot be replayed as someone else's. */
  authorId: string;
}

const messageData = (context: MessageContext) =>
  utf8(`${context.conversationId}|${context.epochId}|${context.authorId}`);

export async function encryptPayload(
  epochKey: CryptoKey,
  context: MessageContext,
  payload: MessagePayload,
): Promise<{ iv: Bytes; ciphertext: Bytes }> {
  const iv = randomBytes(12);
  const ciphertext = bytesOf(
    await crypto.subtle.encrypt(
      { name: "AES-GCM", iv, additionalData: messageData(context) },
      epochKey,
      utf8(JSON.stringify(payload)),
    ),
  );
  return { iv, ciphertext };
}

/** Throws when the key, the context or the bytes are not the ones it was sealed with. */
export async function decryptPayload(
  epochKey: CryptoKey,
  context: MessageContext,
  iv: Bytes,
  ciphertext: Bytes,
): Promise<MessagePayload> {
  const plain = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv, additionalData: messageData(context) },
    epochKey,
    ciphertext,
  );
  const payload = parsePayload(JSON.parse(fromUtf8(plain)) as unknown);
  if (payload === null) throw new Error("The message does not have the shape of version 1.");
  return payload;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isString = (value: unknown): value is string => typeof value === "string";
const isCount = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value) && value >= 0;

function parsePoster(value: unknown): PayloadPoster | null {
  if (!isRecord(value)) return null;
  const { id, key, iv, type, size } = value;
  if (!isString(id) || !isString(key) || !isString(iv) || !isString(type) || !isCount(size)) return null;
  return { id, key, iv, type, size };
}

function parseAttachment(value: unknown): PayloadAttachment | null {
  if (!isRecord(value)) return null;
  const { id, key, iv, name, type, size, width, height, duration, voice, poster } = value;
  if (!isString(id) || !isString(key) || !isString(iv) || !isString(name) || !isString(type) || !isCount(size)) {
    return null;
  }
  const parsedPoster = poster === undefined ? undefined : parsePoster(poster);
  if (parsedPoster === null) return null;
  return {
    id,
    key,
    iv,
    // A name is shown and offered as a file name; nothing in it may be a path.
    name: name.replace(/[\\/]/g, "_").slice(0, 255),
    type,
    size,
    ...(isCount(width) ? { width } : {}),
    ...(isCount(height) ? { height } : {}),
    ...(isCount(duration) ? { duration } : {}),
    ...(voice === true ? { voice: true } : {}),
    ...(parsedPoster === undefined ? {} : { poster: parsedPoster }),
  };
}

/** The payload, or null for anything that is not version 1. */
export function parsePayload(value: unknown): MessagePayload | null {
  if (!isRecord(value) || value["v"] !== 1 || !isString(value["text"])) return null;
  const list = value["attachments"] ?? [];
  if (!Array.isArray(list)) return null;
  const attachments: PayloadAttachment[] = [];
  for (const item of list) {
    const attachment = parseAttachment(item);
    if (attachment === null) return null;
    attachments.push(attachment);
  }
  return { v: 1, text: value["text"], attachments };
}
