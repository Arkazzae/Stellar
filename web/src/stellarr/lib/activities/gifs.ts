import type { ActivityGifs } from "../../../../../sdk/host/media-host.js";
import type { GifErrorCode } from "../../../../../sdk/media.js";
import type { SupportedLocale } from "@/i18n";
import {
  fetchGiphyGif,
  fetchGiphyPage,
  GiphyError,
  GIPHY_REFERRER_POLICY,
  resolveGiphyApiKey,
  trackGiphyAction,
  type GiphyGif,
} from "@/lib/giphy/giphy";
import { giphyChoice, type GiphyAccess, type GiphyChoice } from "@/lib/giphy/giphyChoice";

/**
 * GIF search for activities (`media.gifs`), answered by this page.
 *
 * The activity's frame never reaches GIPHY. This page asks GIPHY under the
 * rules chat follows: the operator's switch and key, the person's own switch,
 * this browser's customer id, no cookies and no referrer. The frame receives
 * ids, descriptions and sizes, and later the bytes of one rendition; never a
 * URL, the key or GIPHY's analytics addresses. Nothing goes through or stays
 * on Stellarr's server, and nothing outlives the page.
 */

/** GIPHY's media hosts, the only places a GIF's bytes are fetched from. `infra/nginx.conf` allows the same. */
export const GIPHY_MEDIA_HOSTS: ReadonlySet<string> = new Set([
  "media.giphy.com",
  "media0.giphy.com",
  "media1.giphy.com",
  "media2.giphy.com",
  "media3.giphy.com",
  "media4.giphy.com",
  "i.giphy.com",
]);

/** The animated formats GIPHY serves for the preview and full renditions. */
const IMAGE_TYPES: ReadonlySet<string> = new Set(["image/gif", "image/webp"]);
/** How long a request waits for the account's copy of the person's switch. */
export const SWITCH_WAIT_MS = 10_000;
/** How long one rendition may take to arrive. */
const FETCH_MS = 30_000;
const KNOWN_GIFS = 512;
const CACHED_BYTES = 32 * 1024 * 1024;

interface Rendition {
  bytes: ArrayBuffer;
  type: string;
  width: number;
  height: number;
  gif: GiphyGif;
}

/** What this page learned about GIFs: never stored, gone with the page. */
export interface GifCache {
  gifs: Map<string, GiphyGif>;
  renditions: Map<string, Rendition>;
  bytes: number;
  running: Map<string, Promise<Rendition>>;
}

export function createGifCache(): GifCache {
  return { gifs: new Map(), renditions: new Map(), bytes: 0, running: new Map() };
}

const pageCache = createGifCache();

type GifFailure = Error & { code: GifErrorCode };

function failure(code: GifErrorCode, message: string): GifFailure {
  return Object.assign(new Error(message), { code });
}

function isFailure(error: unknown): error is GifFailure {
  return error instanceof Error && "code" in error && typeof error.code === "string";
}

/** What a failure of `lib/giphy` means to the activity. */
function fromGiphy(error: unknown): GifFailure {
  if (isFailure(error)) return error;
  if (error instanceof GiphyError) {
    switch (error.messageKey) {
      case "giphy.keyMissing":
        return failure("unavailable", "GIFs are off on this instance.");
      case "giphy.rateLimited":
        return failure("rateLimited", "GIPHY is limiting this instance's requests.");
      case "giphy.badId":
      case "giphy.gone":
      case "giphy.badGif":
        return failure("notFound", "GIPHY has no such GIF.");
    }
  }
  return failure("failed", "GIPHY could not be reached.");
}

/** A GIPHY media address the bytes may come from, or null. */
function mediaUrl(raw: string): URL | null {
  try {
    const url = new URL(raw);
    const plain = url.protocol === "https:" && url.port === "" && url.username === "" && url.password === "";
    return plain && GIPHY_MEDIA_HOSTS.has(url.hostname) ? url : null;
  } catch {
    return null;
  }
}

/** The switch once it has settled, or "waiting" when it did not within `SWITCH_WAIT_MS`. */
function settledAccess(choice: GiphyChoice, signal: AbortSignal): Promise<GiphyAccess> {
  if (choice.access() !== "waiting" || signal.aborted) return Promise.resolve(choice.access());
  return new Promise((resolve) => {
    const finish = () => {
      clearTimeout(timer);
      unsubscribe();
      signal.removeEventListener("abort", finish);
      resolve(choice.access());
    };
    const unsubscribe = choice.subscribe(() => {
      if (choice.access() !== "waiting") finish();
    });
    const timer = setTimeout(finish, SWITCH_WAIT_MS);
    signal.addEventListener("abort", finish, { once: true });
  });
}

function untilAborted<T>(work: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(failure("failed", "GIF request cancelled."));
  return new Promise((resolve, reject) => {
    const stop = () => reject(failure("failed", "GIF request cancelled."));
    signal.addEventListener("abort", stop, { once: true });
    work.then(resolve, reject).finally(() => signal.removeEventListener("abort", stop));
  });
}

/** The body, refused as soon as it grows past `limit`. */
async function readCapped(response: Response, limit: number): Promise<ArrayBuffer> {
  const tooLarge = () => failure("failed", "The GIF is too large.");
  if (Number(response.headers.get("content-length")) > limit) {
    void response.body?.cancel().catch(() => undefined);
    throw tooLarge();
  }
  const reader = response.body?.getReader();
  if (reader === undefined) {
    const whole = await response.arrayBuffer();
    if (whole.byteLength > limit) throw tooLarge();
    return whole;
  }
  const chunks: Uint8Array[] = [];
  let received = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    received += value.byteLength;
    if (received > limit) {
      void reader.cancel().catch(() => undefined);
      throw tooLarge();
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(received);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes.buffer;
}

function ascii(view: DataView, from: number, to: number): string {
  let text = "";
  for (let index = from; index < Math.min(to, view.byteLength); index++) {
    text += String.fromCharCode(view.getUint8(index));
  }
  return text;
}

/**
 * The picture's size from its own header, or null when the bytes are not the
 * format they claim. A size of 0 means the header does not say it.
 */
function measure(bytes: ArrayBuffer, type: string): { width: number; height: number } | null {
  const view = new DataView(bytes);
  if (type === "image/gif") {
    const signature = ascii(view, 0, 6);
    if ((signature !== "GIF87a" && signature !== "GIF89a") || view.byteLength < 10) return null;
    return { width: view.getUint16(6, true), height: view.getUint16(8, true) };
  }
  if (ascii(view, 0, 4) !== "RIFF" || ascii(view, 8, 12) !== "WEBP") return null;
  if (ascii(view, 12, 16) !== "VP8X" || view.byteLength < 30) return { width: 0, height: 0 };
  const width = view.getUint16(24, true) + (view.getUint8(26) << 16) + 1;
  const height = view.getUint16(27, true) + (view.getUint8(29) << 16) + 1;
  return { width, height };
}

function remember(cache: GifCache, gif: GiphyGif): void {
  cache.gifs.delete(gif.id);
  cache.gifs.set(gif.id, gif);
  for (const id of cache.gifs.keys()) {
    if (cache.gifs.size <= KNOWN_GIFS) break;
    cache.gifs.delete(id);
  }
}

function keep(cache: GifCache, key: string, rendition: Rendition): void {
  cache.renditions.set(key, rendition);
  cache.bytes += rendition.bytes.byteLength;
  for (const [old, kept] of cache.renditions) {
    if (cache.bytes <= CACHED_BYTES) break;
    cache.renditions.delete(old);
    cache.bytes -= kept.bytes.byteLength;
  }
}

export interface ActivityGifsOptions {
  /** The interface language, which GIPHY searches in. */
  locale(): SupportedLocale;
  /** The activity searched or received a GIF, so its bar owes GIPHY's mark. */
  onUse(): void;
  choice?: GiphyChoice;
  cache?: GifCache;
}

/** The provider one activity frame's media host asks. */
export function createActivityGifs({
  locale,
  onUse,
  choice = giphyChoice,
  cache = pageCache,
}: ActivityGifsOptions): ActivityGifs {
  // GIPHY counts one view per GIF, not one per load.
  const viewed = new Set<string>();

  /** The key, once the operator and then the person allow GIPHY. */
  async function allow(signal: AbortSignal): Promise<string> {
    const key = await resolveGiphyApiKey().catch((error: unknown) => {
      throw fromGiphy(error);
    });
    const access = await settledAccess(choice, signal);
    if (access === "off") throw failure("declined", "This player turned GIPHY off in their privacy settings.");
    if (access === "waiting") throw failure("failed", "This player's GIPHY setting has not arrived yet.");
    return key;
  }

  /** Re-asked before every request, since the person may turn GIPHY off meanwhile. */
  function stillOn(): void {
    if (choice.access() !== "on") throw failure("declined", "This player turned GIPHY off in their privacy settings.");
  }

  async function fetchRendition(key: string, id: string, size: "preview" | "full", maxBytes: number) {
    const signal = AbortSignal.timeout(FETCH_MS);
    let gif = cache.gifs.get(id);
    if (gif === undefined) {
      stillOn();
      gif = await fetchGiphyGif(key, id, signal).catch((error: unknown) => {
        throw fromGiphy(error);
      });
      remember(cache, gif);
    }
    // Only an address GIPHY gave for this GIF, and only on its media hosts.
    const url = mediaUrl(size === "full" ? gif.mediaUrl : gif.previewUrl);
    if (url === null) throw failure("failed", "GIPHY gave an address outside its media hosts.");
    stillOn();
    const response = await fetch(url, {
      method: "GET",
      mode: "cors",
      credentials: "omit",
      referrerPolicy: GIPHY_REFERRER_POLICY,
      signal,
    }).catch(() => {
      throw failure("failed", "GIPHY could not be reached.");
    });
    if (!response.ok) {
      void response.body?.cancel().catch(() => undefined);
      if (response.status === 404 || response.status === 410) throw failure("notFound", "GIPHY has no such GIF.");
      if (response.status === 429) throw failure("rateLimited", "GIPHY is limiting this instance's requests.");
      throw failure("failed", "GIPHY could not send the GIF.");
    }
    if (response.url !== "" && mediaUrl(response.url) === null) {
      void response.body?.cancel().catch(() => undefined);
      throw failure("failed", "GIPHY sent the GIF from outside its media hosts.");
    }
    const type = (response.headers.get("content-type") ?? "").split(";")[0]?.trim().toLowerCase() ?? "";
    if (!IMAGE_TYPES.has(type)) {
      void response.body?.cancel().catch(() => undefined);
      throw failure("failed", "GIPHY sent something other than a GIF.");
    }
    const bytes = await readCapped(response, maxBytes);
    const measured = measure(bytes, type);
    if (measured === null) throw failure("failed", "GIPHY sent something other than a GIF.");
    // GIPHY's own figures are those of the full rendition.
    const dimensions = measured.width > 0 && measured.height > 0 ? measured : gif;
    return { bytes, type, width: dimensions.width, height: dimensions.height, gif };
  }

  return {
    async search({ query, offset, limit }, signal) {
      const key = await allow(signal);
      stillOn();
      const page = await fetchGiphyPage({ apiKey: key, locale: locale(), query, offset, limit, signal }).catch(
        (error: unknown) => {
          throw fromGiphy(error);
        },
      );
      // A result that could not be loaded is not offered.
      const items = page.items.filter(
        (gif) => mediaUrl(gif.previewUrl) !== null && mediaUrl(gif.mediaUrl) !== null,
      );
      for (const gif of items) remember(cache, gif);
      onUse();
      return {
        items: items.map(({ id, alt, width, height }) => ({
          id,
          ...(alt === undefined ? {} : { alt }),
          width,
          height,
        })),
        offset: page.offset,
        total: page.totalCount,
      };
    },

    async load({ gif: id, size, maxBytes }, signal) {
      const key = await allow(signal);
      const name = `${id}:${size}`;
      let rendition = cache.renditions.get(name);
      if (rendition !== undefined) {
        cache.renditions.delete(name);
        cache.renditions.set(name, rendition);
      } else {
        // Two players' picks may be the same GIF: one request serves both.
        let running = cache.running.get(name);
        if (running === undefined) {
          running = fetchRendition(key, id, size, maxBytes).finally(() => cache.running.delete(name));
          // Whoever waits hears the failure; one who stopped waiting leaves nothing unhandled.
          running.catch(() => undefined);
          cache.running.set(name, running);
        }
        rendition = await untilAborted(running, signal);
        if (!cache.renditions.has(name)) keep(cache, name, rendition);
      }
      onUse();
      if (!viewed.has(rendition.gif.id) && choice.access() === "on") {
        viewed.add(rendition.gif.id);
        void trackGiphyAction(rendition.gif.analytics.onload);
      }
      const { bytes, type, width, height, gif } = rendition;
      // The frame gets a copy: what is sent is detached from this page.
      return { bytes: bytes.slice(0), type, width, height, ...(gif.alt === undefined ? {} : { alt: gif.alt }) };
    },
  };
}
