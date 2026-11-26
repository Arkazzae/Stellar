import { afterEach, expect, it, vi } from "vitest";

import { bindMediaHost, type ActivityCamera } from "../../../../../sdk/host/media-host.js";
import { attachEmbed, createMedia } from "../../../../../sdk/media.js";

const embedHosts = ["video.example", "another.example"];

// Connect the public SDK and the parent host with their normal MessageChannel.
function mountActivity(camera?: ActivityCamera, ask = vi.fn(async () => true), hostedEmbeds = true) {
  const viewport = document.createElement("div");
  const frame = document.createElement("iframe");
  frame.src = "/_activity/lease/index.html";
  const slot = document.createElement("div");
  document.body.append(viewport);
  viewport.append(frame, slot);
  const rect = new DOMRect(0, 0, 320, 180);
  vi.spyOn(slot, "getBoundingClientRect").mockReturnValue(rect);
  vi.spyOn(frame, "getBoundingClientRect").mockReturnValue(rect);
  const nonce = crypto.randomUUID();
  window.location.hash = "stellarr=" + nonce;
  vi.stubGlobal("origin", "null");
  vi.spyOn(window, "parent", "get").mockReturnValue({
    postMessage(data: unknown, _origin: string, ports: MessagePort[]) {
      window.dispatchEvent(Object.assign(new Event("message"), { source: frame.contentWindow, origin: "null", data, ports }));
    },
  } as Window);
  const permissions = ["media.cameraTile"];
  const host = bindMediaHost({ frame, origin: "null", nonce, embedHosts, title: "Game", permissions,
    controls: viewport, volume: document.createElement("input"), overlay: viewport, camera: camera && (() => camera), ask });
  const media = createMedia({ context: {}, embedHosts, hostedEmbeds, permissions });
  return { viewport, slot, media, dispose() { media.dispose(); host.dispose(); viewport.remove(); } };
}

afterEach(() => {
  window.location.hash = "";
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it.each([true, false])("keeps an embedded player's position across visibility changes (hosted %s)", async hosted => {
  const { viewport, slot, media, dispose } = mountActivity(undefined, undefined, hosted);
  try {
    const url = "https://video.example/embed/film?id=one";
    const receive = vi.fn(), loaded = vi.fn();
    const player = media.embed.mount(url, slot, { title: "Film", onMessage: receive, onLoad: loaded });
    let doc = document;
    let shell: HTMLIFrameElement | null = null;
    if (hosted) {
      await expect.poll(() => viewport.querySelector("iframe[title=Film]")).not.toBeNull();
      shell = viewport.querySelector<HTMLIFrameElement>("iframe[title=Film]")!;
      expect(new URL(shell.src).pathname).toBe("/_activity/lease/_stellarr/embed.html");
      doc = shell.contentDocument!;
      // jsdom does not fetch the wrapper or run its trusted bootstrap module.
      doc.open();
      doc.write("<html><body></body></html>");
      doc.close();
      Object.assign(doc.defaultView!, { mountStellarrEmbed: attachEmbed });
      shell.dispatchEvent(new Event("load"));
    }
    const findFrame = () => (hosted ? doc : slot).querySelector<HTMLIFrameElement>("iframe[title=Film]");
    const frame = findFrame()!;
    expect(frame.src).toBe(url);
    const provider = frame.contentWindow!;
    const providerDoc = provider.document;
    const video = providerDoc.createElement("video");
    providerDoc.appendChild(video);
    video.currentTime = 42;
    const post = vi.spyOn(provider, "postMessage");
    frame.dispatchEvent(new Event("load"));
    await expect.poll(() => loaded.mock.calls.length).toBe(1);
    const report = (data: unknown, origin = new URL(url).origin, source: Window = provider) =>
      doc.defaultView!.dispatchEvent(Object.assign(new Event("message"), { origin, source, data }));
    // This provider exposes its playback position through its own message protocol.
    report({ position: video.currentTime });
    await expect.poll(() => receive.mock.lastCall).toEqual([{ position: 42 }]);
    const overlay = shell ?? frame;
    player.setVisible(false);
    await expect.poll(() => overlay).not.toBeVisible();
    video.currentTime = 43;
    report({ position: video.currentTime });
    await expect.poll(() => receive.mock.lastCall).toEqual([{ position: 43 }]);
    player.setVisible(true);
    await expect.poll(() => overlay).toBeVisible();
    expect(findFrame()).toBe(frame);
    expect(frame.contentWindow).toBe(provider);
    expect(frame.contentDocument?.querySelector("video")?.currentTime).toBe(43);
    expect(loaded).toHaveBeenCalledOnce();
    expect(post).not.toHaveBeenCalled();
    report("wrong origin", "https://another.example");
    report("wrong window", new URL(url).origin, window);
    player.postMessage({ command: "readPosition" });
    await expect.poll(() => post.mock.lastCall).toEqual([{ command: "readPosition" }, "https://video.example"]);
    report({ position: video.currentTime });
    await expect.poll(() => receive.mock.calls).toEqual([[{ position: 42 }], [{ position: 43 }], [{ position: 43 }]]);
    player.destroy();
    player.setVisible(true);
    await expect.poll(() => overlay.isConnected).toBe(false);
  } finally { dispose(); }
});

it("rejects embed URLs outside the manifest grant", () => {
  const { slot, media, dispose } = mountActivity();
  try {
    for (const url of ["https://other.example/", "http://video.example/", "https://video.example:444/", "https://user@video.example/", "data:text/html,test"]) {
      expect(() => media.embed.mount(url, slot)).toThrow(/HTTPS embed URL/);
    }
  } finally { dispose(); }
});

it("hides and shows a camera tile while keeping its original consent, capture and subscription", async () => {
  const track = Object.assign(new EventTarget(), { stop: vi.fn() });
  const stream = { getTracks: () => [track] } as unknown as MediaStream;
  const capture = vi.fn(async () => stream);
  vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: capture } });
  const ask = vi.fn(async () => true);
  const unwatch = vi.fn();
  const camera = { publish: vi.fn(), dispose: vi.fn(), watch: vi.fn((_id: string, show: (value: MediaStream) => void) => {
    show(stream);
    return unwatch;
  }) };
  const { viewport, slot, media, dispose } = mountActivity(camera, ask);
  try {
    const device = await media.cameraTile.open();
    const tile = media.cameraTile.mount("alice", slot);
    const findVideo = () => viewport.querySelector("video");
    await expect.poll(findVideo).not.toBeNull();
    const video = findVideo()!;
    tile.setVisible(false);
    await expect.poll(() => video).not.toBeVisible();
    tile.setVisible(true);
    await expect.poll(() => video).toBeVisible();
    expect(findVideo()).toBe(video);
    expect(video.srcObject).toBe(stream);
    expect(ask).toHaveBeenCalledOnce();
    expect(capture).toHaveBeenCalledOnce();
    expect(camera.publish).toHaveBeenCalledOnce();
    expect(camera.watch).toHaveBeenCalledOnce();
    expect(unwatch).not.toHaveBeenCalled();
    expect(track.stop).not.toHaveBeenCalled();
    tile();
    await expect.poll(findVideo).toBeNull();
    expect(unwatch).toHaveBeenCalledOnce();
    device.stop();
    await expect.poll(() => track.stop.mock.calls.length).toBe(1);
  } finally { dispose(); }
});
