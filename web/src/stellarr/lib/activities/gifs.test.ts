import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { giphyCustomerId } from "@/lib/giphy/customerId";
import { createGiphyChoice } from "@/lib/giphy/giphyChoice";

const fetchMock = vi.fn();
const KEY = "instance-secret-key";
const signal = new AbortController().signal;
const preview = { gif: "AbCd1234", size: "preview", maxBytes: 1024 * 1024 } as const;

/** A GIF as GIPHY's API describes it, with its renditions under `media`. */
function rawGif(id: string, media = `https://media2.giphy.com/media/${id}`) {
  return {
    id,
    alt_text: "A cat dances.",
    images: {
      fixed_width: { url: `${media}/200w.gif`, webp: `${media}/200w.webp`, width: "200", height: "150" },
      downsized: { url: `${media}/giphy.gif`, width: "480", height: "360" },
    },
    analytics: { onload: { url: `https://giphy-analytics.giphy.com/v2/pingback_simple?payload=${id}` } },
  };
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

/** The start of a GIF of the given size, padded to `length` bytes. */
function gifBytes(width: number, height: number, length = 32) {
  const bytes = new Uint8Array(length);
  bytes.set(new TextEncoder().encode("GIF89a"));
  const view = new DataView(bytes.buffer);
  view.setUint16(6, width, true);
  view.setUint16(8, height, true);
  return bytes;
}

interface Giphy {
  config?: unknown;
  lookup?: (url: URL) => Response;
  media?: (url: URL) => Response;
}

/** Stellarr's API and GIPHY, as the page would reach them. */
function serve({ config = { enabled: true, apiKey: KEY }, lookup, media }: Giphy = {}) {
  fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), "https://stellarr.test");
    if (url.pathname === "/api/config/giphy") return json(config);
    if (url.hostname === "api.giphy.com") {
      if (/\/(search|trending)$/.test(url.pathname)) {
        return json({ data: [rawGif("AbCd1234")], pagination: { offset: 0, count: 1, total_count: 1 } });
      }
      return lookup?.(url) ?? json({ data: rawGif(url.pathname.split("/").at(-1) ?? "") });
    }
    if (url.hostname === "giphy-analytics.giphy.com") return new Response(null);
    return media?.(url) ?? new Response(gifBytes(200, 150), { headers: { "content-type": "image/gif" } });
  });
}

/** Every address the page asked, but its own API. */
function outside(): URL[] {
  return fetchMock.mock.calls
    .map(([input]) => new URL(String(input), "https://stellarr.test"))
    .filter((url) => url.hostname !== "stellarr.test");
}

async function setUp(access: "on" | "off" | "waiting" = "on") {
  // The page's GIPHY configuration starts afresh, and so does its GIF cache.
  vi.resetModules();
  const { createActivityGifs, createGifCache } = await import("./gifs");
  const choice = createGiphyChoice();
  if (access === "off") choice.set(false);
  if (access !== "waiting") choice.accountRead();
  const onUse = vi.fn();
  const cache = createGifCache();
  const frame = () => createActivityGifs({ locale: () => "en", onUse, choice, cache });
  return { gifs: frame(), anotherFrame: frame, choice, onUse };
}

beforeEach(() => {
  window.localStorage.clear();
  fetchMock.mockReset();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the person's own GIPHY switch", () => {
  it("off, the activity is refused and GIPHY is asked nothing", async () => {
    serve();
    const { gifs } = await setUp("off");

    await expect(gifs.search({ query: "cat", offset: 0, limit: 12 }, signal)).rejects.toMatchObject({ code: "declined" });
    await expect(gifs.load(preview, signal)).rejects.toMatchObject({ code: "declined" });
    expect(outside()).toEqual([]);
  });

  it("turned off later, holds back even a GIF this page already has", async () => {
    serve();
    const { gifs, choice } = await setUp();
    await gifs.search({ query: "cat", offset: 0, limit: 12 }, signal);
    await gifs.load(preview, signal);
    const asked = outside().length;

    choice.set(false);

    await expect(gifs.load(preview, signal)).rejects.toMatchObject({ code: "declined" });
    await expect(gifs.search({ query: "dog", offset: 0, limit: 12 }, signal)).rejects.toMatchObject({ code: "declined" });
    expect(outside()).toHaveLength(asked);
  });

  it("still on its way, is waited for and then followed", async () => {
    serve();
    const refused = await setUp("waiting");
    const refusedWaits = vi.spyOn(refused.choice, "subscribe");
    const declined = refused.gifs.search({ query: "cat", offset: 0, limit: 12 }, signal);
    await vi.waitFor(() => expect(refusedWaits).toHaveBeenCalled());
    refused.choice.set(false);
    await expect(declined).rejects.toMatchObject({ code: "declined" });
    expect(outside()).toEqual([]);

    window.localStorage.clear();
    const allowed = await setUp("waiting");
    const allowedWaits = vi.spyOn(allowed.choice, "subscribe");
    const found = allowed.gifs.search({ query: "cat", offset: 0, limit: 12 }, signal);
    await vi.waitFor(() => expect(allowedWaits).toHaveBeenCalled());
    expect(outside()).toEqual([]);
    allowed.choice.accountRead();
    await expect(found).resolves.toMatchObject({ total: 1 });
  });
});

it("is refused when the operator has GIFs off, and GIPHY is asked nothing", async () => {
  serve({ config: { enabled: false, apiKey: "" } });
  const { gifs } = await setUp();

  await expect(gifs.search({ query: "cat", offset: 0, limit: 12 }, signal)).rejects.toMatchObject({ code: "unavailable" });
  await expect(gifs.load(preview, signal)).rejects.toMatchObject({ code: "unavailable" });
  expect(outside()).toEqual([]);
});

it("hands the frame ids, descriptions and sizes, never the key, an address or analytics", async () => {
  serve();
  const { gifs, onUse } = await setUp();

  const page = await gifs.search({ query: "cat", offset: 0, limit: 12 }, signal);

  expect(page).toEqual({ items: [{ id: "AbCd1234", alt: "A cat dances.", width: 480, height: 360 }], offset: 0, total: 1 });
  const sent = JSON.stringify(page);
  for (const secret of [KEY, "giphy.com", "https:", giphyCustomerId()]) expect(sent).not.toContain(secret);
  expect(onUse).toHaveBeenCalled();
});

describe("loading a GIF's bytes", () => {
  it("asks GIPHY's media host itself, without cookies or a referrer", async () => {
    serve();
    const { gifs } = await setUp();
    await gifs.search({ query: "cat", offset: 0, limit: 12 }, signal);

    const image = await gifs.load(preview, signal);

    expect(image).toMatchObject({ type: "image/gif", width: 200, height: 150, alt: "A cat dances." });
    expect(image.bytes.byteLength).toBe(32);
    const [, init] = fetchMock.mock.calls.find(
      ([input]) => new URL(String(input), "https://stellarr.test").hostname === "media2.giphy.com",
    ) as [URL, RequestInit];
    expect(init).toMatchObject({ credentials: "omit", referrerPolicy: "no-referrer", mode: "cors" });
  });

  it("refuses a GIF whose address is not on GIPHY's media hosts, without asking that address", async () => {
    serve({ lookup: () => json({ data: rawGif("EvIl1234", "https://media.giphy.com.evil.example/media/EvIl1234") }) });
    const { gifs } = await setUp();

    await expect(gifs.load({ ...preview, gif: "EvIl1234" }, signal)).rejects.toMatchObject({ code: "failed" });
    expect(outside().map((url) => url.hostname)).toEqual(["api.giphy.com"]);
  });

  it("refuses a body over the cap, a declared size over it, and a body that is not a GIF", async () => {
    const bodies: Response[] = [
      new Response(gifBytes(200, 150, 2048), { headers: { "content-type": "image/gif" } }),
      new Response(gifBytes(200, 150), { headers: { "content-type": "image/gif", "content-length": "99999999" } }),
      new Response("<html></html>", { headers: { "content-type": "text/html" } }),
      new Response("<html></html>", { headers: { "content-type": "image/gif" } }),
    ];
    serve({ media: () => bodies.shift() ?? new Response(null, { status: 500 }) });
    const { gifs } = await setUp();
    await gifs.search({ query: "cat", offset: 0, limit: 12 }, signal);

    for (let attempt = 0; attempt < 4; attempt++) {
      await expect(gifs.load({ ...preview, maxBytes: 1024 }, signal)).rejects.toMatchObject({ code: "failed" });
    }
    expect(bodies).toEqual([]);
  });

  it("fetches each rendition once for the page and reports one view per frame", async () => {
    serve();
    const { gifs, anotherFrame } = await setUp();
    await gifs.search({ query: "cat", offset: 0, limit: 12 }, signal);

    const [first, second] = await Promise.all([gifs.load(preview, signal), gifs.load(preview, signal)]);
    const third = await gifs.load(preview, signal);
    await anotherFrame().load(preview, signal);

    const hosts = outside().map((url) => url.hostname);
    expect(hosts.filter((host) => host === "media2.giphy.com")).toHaveLength(1);
    expect(hosts.filter((host) => host === "giphy-analytics.giphy.com")).toHaveLength(2);
    // Each answer is a copy of its own, so sending one leaves the others whole.
    for (const image of [first, second, third]) expect(image.bytes.byteLength).toBe(32);
  });

  it("looks up a GIF it has not seen, and tells a missing GIF from a rate limit", async () => {
    serve({ lookup: (url) => (url.pathname.endsWith("/Gone1234") ? json({}, 404) : json({}, 429)) });
    const { gifs } = await setUp();

    await expect(gifs.load({ ...preview, gif: "Gone1234" }, signal)).rejects.toMatchObject({ code: "notFound" });
    await expect(gifs.load({ ...preview, gif: "Busy1234" }, signal)).rejects.toMatchObject({ code: "rateLimited" });
  });
});
