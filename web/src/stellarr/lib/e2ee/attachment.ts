import { bytesOf, fromBase64, randomBytes, toBase64 } from "./bytes";

/**
 * One file of an encrypted message, sealed on its own.
 *
 * Every file, video poster and image preview gets a fresh AES-256 key, carried inside
 * the message's plaintext, so the server stores bytes it cannot read and a key
 * for one file opens nothing else.
 */
export interface SealedFile {
  data: Blob;
  key: string;
  iv: string;
}

export async function encryptFile(file: Blob): Promise<SealedFile> {
  const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, true, ["encrypt"]);
  const iv = randomBytes(12);
  const sealed = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, await file.arrayBuffer());
  const raw = bytesOf(await crypto.subtle.exportKey("raw", key));
  return {
    data: new Blob([sealed], { type: "application/octet-stream" }),
    key: toBase64(raw),
    iv: toBase64(iv),
  };
}

/** Throws when the key or the IV do not fit, or the bytes were changed. */
export async function decryptFile(data: ArrayBuffer, key: string, iv: string): Promise<ArrayBuffer> {
  const raw = fromBase64(key);
  if (raw.length !== 32) throw new Error("A file key is 32 bytes.");
  const imported = await crypto.subtle.importKey("raw", raw, "AES-GCM", false, ["decrypt"]);
  return crypto.subtle.decrypt({ name: "AES-GCM", iv: fromBase64(iv) }, imported, data);
}

/**
 * The type a decrypted file is shown with.
 *
 * The type comes from the other person's plaintext, and a blob URL runs in
 * this page's origin, so only kinds the page draws itself keep theirs. SVG
 * and HTML can carry script and are never among them; anything else is
 * opaque bytes, to be saved rather than opened.
 */
const SHOWN_TYPES: ReadonlySet<string> = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "image/avif",
  "video/mp4",
  "video/webm",
  "video/quicktime",
  "audio/mpeg",
  "audio/mp4",
  "audio/aac",
  "audio/ogg",
  "audio/opus",
  "audio/wav",
  "audio/webm",
  "audio/flac",
]);

export const OPAQUE_TYPE = "application/octet-stream";

export function shownType(type: string): string {
  const bare = type.split(";")[0]?.trim().toLowerCase() ?? "";
  return SHOWN_TYPES.has(bare) ? bare : OPAQUE_TYPE;
}
