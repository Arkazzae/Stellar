import { afterEach, expect, it, vi } from "vitest";

import { bindMediaHost, gifLimits, type ActivityGifs } from "../../../../../sdk/host/media-host.js";
import { createMedia } from "../../../../../sdk/media.js";

type Load = ActivityGifs["load"];
type Search = ActivityGifs["search"];

const image = () => ({ bytes: new Uint8Array(16).buffer, type: "image/gif", width: 200, height: 150 });

// The public SDK in the frame and the media host in the page, over their usual MessageChannel.
function connect({ search, load, granted = ["media.gifs"], provided = true }: {
  search?: Search; load?: Load; granted?: string[]; provided?: boolean;
} = {}) {
  const frame = document.createElement("iframe");
  const controls = document.createElement("span");
  document.body.append(frame, controls);
  vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:gif");
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
  vi.spyOn(window, "parent", "get").mockReturnValue({
    postMessage(data: unknown, _origin: string, ports: MessagePort[]) {
      window.dispatchEvent(Object.assign(new Event("message"), {
        source: frame.contentWindow, origin: "https://game.test", data, ports,
      }));
    },
  } as unknown as Window);
  const provider: ActivityGifs = {
    search: search ?? (async () => ({ items: [], offset: 0, total: 0 })),
    load: load ?? (async () => image()),
  };
  const host = bindMediaHost({ frame, origin: "https://game.test", title: "Game", permissions: granted,
    controls, volume: document.createElement("input"), gifs: provided ? provider : undefined });
  const media = createMedia({ context: {}, permissions: ["media.gifs"] });
  return { media, dispose() { media.dispose(); host.dispose(); frame.remove(); controls.remove(); } };
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

it("passes the frame only ids, descriptions and sizes, and the bytes of the size it asked for", async () => {
  // Whatever else a provider knows stays on this side.
  const known = { id: "AbCd1234", alt: "A cat", width: 480, height: 360, url: "https://media.giphy.com/x.gif" };
  const search = vi.fn<Search>(async () => ({ items: [known], offset: 0, total: 1 }));
  const load = vi.fn<Load>(async () => image());
  const { media, dispose } = connect({ search, load });
  try {
    expect(await media.gifs.search("  cat  ", { limit: 500 })).toEqual({
      items: [{ id: "AbCd1234", alt: "A cat", width: 480, height: 360 }], offset: 0, total: 1,
    });
    expect(search.mock.calls[0]?.[0]).toEqual({ query: "cat", offset: 0, limit: gifLimits.pageSize });
    const shown = await media.gifs.load("AbCd1234", "full");
    expect(shown).toMatchObject({ type: "image/gif", width: 200, height: 150 });
    expect(load.mock.calls[0]?.[0]).toEqual({ gif: "AbCd1234", size: "full", maxBytes: gifLimits.bytes.full });
  } finally { dispose(); }
});

it("refuses an activity without media.gifs on its launch, and calls nothing", async () => {
  const search = vi.fn<Search>(), load = vi.fn<Load>();
  const { media, dispose } = connect({ search, load, granted: ["media.cameraTile"] });
  try {
    await expect(media.gifs.search("cat")).rejects.toMatchObject({ code: "permission" });
    await expect(media.gifs.load("AbCd1234")).rejects.toMatchObject({ code: "permission" });
    expect(search).not.toHaveBeenCalled();
    expect(load).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it("answers unavailable when the page offers no GIFs", async () => {
  const { media, dispose } = connect({ provided: false });
  try {
    await expect(media.gifs.search("cat")).rejects.toMatchObject({ code: "unavailable" });
  } finally { dispose(); }
});

it("limits one frame's searches per minute, and a reconnect does not refill the budget", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  const search = vi.fn<Search>(async () => ({ items: [], offset: 0, total: 0 }));
  const { media, dispose } = connect({ search });
  try {
    for (let count = 0; count < gifLimits.searchesPerMinute; count++) await media.gifs.search("cat");
    await expect(media.gifs.search("cat")).rejects.toMatchObject({ code: "rateLimited" });
    expect(search).toHaveBeenCalledTimes(gifLimits.searchesPerMinute);

    // A frame that reloads gets a new SDK and port, and the same budget.
    const again = createMedia({ context: {}, permissions: ["media.gifs"] });
    await expect(again.gifs.search("cat")).rejects.toMatchObject({ code: "rateLimited" });
    vi.setSystemTime(Date.now() + 60_000);
    await expect(again.gifs.search("cat")).resolves.toMatchObject({ total: 0 });
    again.dispose();
  } finally { dispose(); }
});

it("loads a few GIFs at a time and lets the rest wait their turn", async () => {
  const finish: (() => void)[] = [];
  const load = vi.fn<Load>(() => new Promise(resolve => { finish.push(() => resolve(image())); }));
  const { media, dispose } = connect({ load });
  try {
    const loads = Array.from({ length: gifLimits.concurrentLoads + 2 }, (_, index) => media.gifs.load("Gif" + index));
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(gifLimits.concurrentLoads));
    await new Promise(resolve => setTimeout(resolve, 30));
    expect(load).toHaveBeenCalledTimes(gifLimits.concurrentLoads);
    finish.shift()?.();
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(gifLimits.concurrentLoads + 1));
    while (finish.length) {
      finish.shift()?.();
      await new Promise(resolve => setTimeout(resolve, 0));
    }
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(gifLimits.concurrentLoads + 2));
    finish.splice(0).forEach(done => done());
    expect(await Promise.all(loads)).toHaveLength(gifLimits.concurrentLoads + 2);
  } finally { dispose(); }
});

it("refuses an image over the size's cap", async () => {
  const load = vi.fn<Load>(async () => ({ ...image(), bytes: new ArrayBuffer(gifLimits.bytes.preview + 1) }));
  const { media, dispose } = connect({ load });
  try {
    await expect(media.gifs.load("AbCd1234")).rejects.toMatchObject({ code: "failed" });
  } finally { dispose(); }
});
