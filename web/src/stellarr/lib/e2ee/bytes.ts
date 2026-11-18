/**
 * Byte helpers for the encryption code.
 *
 * Everything that crosses the wire is standard base64 with padding; the one
 * base64url value is a JWK's `x`. Arrays are typed over a plain `ArrayBuffer`
 * because that is what WebCrypto accepts as a `BufferSource`.
 */

export type Bytes = Uint8Array<ArrayBuffer>;

const CHUNK = 0x8000;

export function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let start = 0; start < bytes.length; start += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(start, start + CHUNK));
  }
  return btoa(binary);
}

/** Throws on anything that is not base64. */
export function fromBase64(text: string): Bytes {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

export function fromBase64Url(text: string): Bytes {
  const standard = text.replace(/-/g, "+").replace(/_/g, "/");
  return fromBase64(standard + "=".repeat((4 - (standard.length % 4)) % 4));
}

export function utf8(text: string): Bytes {
  return new TextEncoder().encode(text);
}

export function fromUtf8(bytes: BufferSource): string {
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}

export function concatBytes(...parts: readonly Uint8Array[]): Bytes {
  const out = new Uint8Array(parts.reduce((total, part) => total + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

export function sameBytes(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index += 1) difference |= (left[index] ?? 0) ^ (right[index] ?? 0);
  return difference === 0;
}

export function randomBytes(length: number): Bytes {
  return crypto.getRandomValues(new Uint8Array(length));
}

export function bytesOf(buffer: ArrayBuffer): Bytes {
  return new Uint8Array(buffer);
}
