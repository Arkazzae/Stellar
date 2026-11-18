/**
 * The reduced still copy of an uploaded image that small views load instead of
 * the original. The rules for when a copy is worth sending and which files must
 * never get one live here as plain functions over bytes and sizes; the probe
 * owns the decoding and the canvas.
 */

/** The preview fits inside this square and is never enlarged. */
export const PREVIEW_MAX_EDGE = 1280;
/** The server refuses a preview above this size. */
export const PREVIEW_MAX_BYTES = 1024 * 1024;
/** A preview larger than this share of the original saves too little to send. */
export const PREVIEW_MAX_SHARE = 0.6;
export const PREVIEW_WEBP_QUALITY = 0.8;
export const PREVIEW_JPEG_QUALITY = 0.82;
/**
 * How much of the file's start is read to tell a still from an animation. A
 * PNG's animation chunk comes before its first image data, so a head that ends
 * before that point is treated as animated: a missing preview costs bytes, a
 * still of a moving picture costs the picture.
 */
export const ANIMATION_HEAD_BYTES = 64 * 1024;

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
/** The animation flag in the VP8X chunk's flag byte. */
const WEBP_ANIMATION_FLAG = 0x02;

function ascii(bytes: Uint8Array, offset: number, length: number): string {
  let text = "";
  for (let at = offset; at < offset + length && at < bytes.length; at += 1) {
    text += String.fromCharCode(bytes[at] ?? 0);
  }
  return text;
}

/** True for a GIF, a PNG with an animation control chunk, or an animated WebP. */
export function isAnimatedImage(head: Uint8Array): boolean {
  if (ascii(head, 0, 3) === "GIF") return true;
  if (PNG_SIGNATURE.every((byte, at) => head[at] === byte)) return isAnimatedPng(head);
  if (ascii(head, 0, 4) === "RIFF" && ascii(head, 8, 4) === "WEBP") {
    // Only an extended WebP can carry an animation, and its VP8X chunk comes first.
    return ascii(head, 12, 4) === "VP8X" && ((head[20] ?? 0) & WEBP_ANIMATION_FLAG) !== 0;
  }
  return false;
}

function isAnimatedPng(head: Uint8Array): boolean {
  const view = new DataView(head.buffer, head.byteOffset, head.byteLength);
  let offset = PNG_SIGNATURE.length;
  while (offset + 8 <= head.length) {
    const type = ascii(head, offset + 4, 4);
    if (type === "acTL") return true;
    if (type === "IDAT") return false;
    offset += 12 + view.getUint32(offset);
  }
  return true;
}

/** Whether a file of this type is ever given a preview; SVG is already small and sharp. */
export function mayHavePreview(type: string): boolean {
  return type.startsWith("image/") && type !== "image/svg+xml" && type !== "image/gif";
}

/** The size the preview is drawn at: inside the square, never larger than the original. */
export function previewSize(width: number, height: number): { width: number; height: number } {
  const scale = Math.min(1, PREVIEW_MAX_EDGE / width, PREVIEW_MAX_EDGE / height);
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  };
}

/** Whether an encoded preview is small enough, in itself and against the original, to send. */
export function previewIsWorthSending(previewBytes: number, originalBytes: number): boolean {
  return previewBytes <= PREVIEW_MAX_BYTES && previewBytes <= originalBytes * PREVIEW_MAX_SHARE;
}
