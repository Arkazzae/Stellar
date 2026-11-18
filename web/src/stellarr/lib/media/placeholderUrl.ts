import { placeholderDataUrl, placeholderFromBase64 } from "./placeholder";

/** Decoding and painting a placeholder is not free, and the same value is asked for on every render. */
const urls = new Map<string, string | null>();

/**
 * The `data:` URL of the blurred stand-in for an image, from the base64 text
 * the server returned. Null for text that does not decode, so a bad value
 * draws nothing. Computed once per value.
 */
export function placeholderUrlFor(text: string | undefined): string | null {
  if (text === undefined) return null;
  let url = urls.get(text);
  if (url === undefined) {
    const bytes = placeholderFromBase64(text);
    url = bytes === null ? null : placeholderDataUrl(bytes);
    urls.set(text, url);
  }
  return url;
}
