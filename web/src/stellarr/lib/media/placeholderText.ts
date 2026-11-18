/**
 * The text form of a placeholder: standard padded base64 of at most 32 bytes.
 * Kept apart from the codec so that code which only checks or passes the text
 * along does not load the code that draws pictures.
 */

export const PLACEHOLDER_MAX_BYTES = 32;

const BASE64_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** The text form the bytes travel in: standard padded base64. */
export function placeholderToBase64(bytes: Uint8Array): string {
  let text = "";
  for (let p = 0; p < bytes.length; p += 3) {
    const a = bytes[p]!;
    const b = bytes[p + 1];
    const c = bytes[p + 2];
    const word = (a << 16) | ((b ?? 0) << 8) | (c ?? 0);
    text += BASE64_ALPHABET[(word >> 18) & 63]!;
    text += BASE64_ALPHABET[(word >> 12) & 63]!;
    text += b === undefined ? "=" : BASE64_ALPHABET[(word >> 6) & 63]!;
    text += c === undefined ? "=" : BASE64_ALPHABET[word & 63]!;
  }
  return text;
}

/**
 * Null for anything that is not canonical padded base64 (wrong length, a
 * character outside the alphabet, padding anywhere but the end, nonzero
 * leftover bits) or that decodes to more than PLACEHOLDER_MAX_BYTES bytes.
 */
export function placeholderFromBase64(text: string): Uint8Array | null {
  if (typeof text !== "string" || text.length % 4 !== 0) return null;
  const padding = text.endsWith("==") ? 2 : text.endsWith("=") ? 1 : 0;
  const size = (text.length / 4) * 3 - padding;
  if (size > PLACEHOLDER_MAX_BYTES) return null;

  const out = new Uint8Array(size);
  let produced = 0;
  for (let p = 0; p < text.length; p += 4) {
    let word = 0;
    for (let k = 0; k < 4; k++) {
      const isPad = p + k >= text.length - padding;
      const sextet = isPad ? 0 : BASE64_ALPHABET.indexOf(text[p + k]!);
      if (sextet < 0) return null;
      word = (word << 6) | sextet;
    }
    const group = [(word >> 16) & 255, (word >> 8) & 255, word & 255];
    for (const byte of group) {
      if (produced < size) out[produced++] = byte;
      else if (byte !== 0) return null;
    }
  }
  return out;
}
