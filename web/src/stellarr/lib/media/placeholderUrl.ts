import { placeholderDataUrl } from "./placeholder";
import { placeholderFromBase64 } from "./placeholderText";

/** Each entry is a few kilobytes, so a long session scrolling through history keeps only the latest. */
export const PLACEHOLDER_URL_CACHE_LIMIT = 300;

/** Decoding and painting a placeholder is not free, and the same value is asked for on every render. */
const urls = new Map<string, string | null>();

/**
 * The `data:` URL of the blurred stand-in for an image, from the base64 text
 * the server returned. Null for text that does not decode, so a bad value
 * draws nothing. Computed once per value while it stays among the most recently
 * used `PLACEHOLDER_URL_CACHE_LIMIT`.
 */
export function placeholderUrlFor(text: string | undefined): string | null {
  if (text === undefined) return null;
  let url = urls.get(text);
  if (url === undefined) {
    const bytes = placeholderFromBase64(text);
    url = bytes === null ? null : placeholderDataUrl(bytes);
  } else {
    // Re-inserted, so the map's order is the order of last use.
    urls.delete(text);
  }
  urls.set(text, url);
  const oldest = urls.keys().next();
  if (urls.size > PLACEHOLDER_URL_CACHE_LIMIT && !oldest.done) urls.delete(oldest.value);
  return url;
}
