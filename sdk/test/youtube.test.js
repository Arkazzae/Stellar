import assert from "node:assert/strict";
import test from "node:test";
import { createYouTubeHost } from "../host/youtube.js";

test("the hosted player clips to the activity and forwards playback for its current video", t => {
  const origin = "https://chat.example.test";
  const provider = "https://www.youtube-nocookie.com";
  const messages = [], updates = [];
  const bus = new EventTarget();
  const element = {
    style: {}, setAttribute() {}, remove() { this.removed = true; },
    contentWindow: { postMessage(json, target) { messages.push([JSON.parse(json), target]); } },
  };
  const previous = new Map();
  for (const [key, value] of Object.entries({
    window: bus, location: { origin }, document: { createElement: () => element },
    ResizeObserver: class { observe() {} disconnect() {} },
  })) {
    previous.set(key, Object.getOwnPropertyDescriptor(globalThis, key));
    Object.defineProperty(globalThis, key, { value, configurable: true });
  }
  t.after(() => {
    for (const [key, descriptor] of previous) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
  });
  const box = { left: 10, top: 30, width: 800, height: 500 };
  const activity = { parentElement: { getBoundingClientRect: () => box },
    getBoundingClientRect: () => box, after(frame) { assert.equal(frame, element); } };
  const host = createYouTubeHost(activity, data => updates.push(data));
  try {
    const videoId = "M7lc1UVf-VE", id = 4;
    host.handle({ op: "youtubeOpen", id, videoId });
    const url = new URL(element.src);
    assert.equal(url.origin, provider);
    assert.equal(url.searchParams.get("origin"), origin);
    host.handle({ op: "youtubeLayout", id, bounds: [-20, 50, 500, 250, 400, 250] });
    assert.deepEqual(element.style.width, "800px");
    assert.deepEqual(element.style.height, "400px");
    const fromPlayer = data => bus.dispatchEvent(Object.assign(new Event("message"), {
      source: element.contentWindow, origin: provider, data: JSON.stringify({ id: element.id, ...data }),
    }));
    fromPlayer({ event: "onReady" });
    host.handle({ op: "youtubeCommand", id, command: "loadVideoById", value: 12 });
    assert.deepEqual(messages.at(-1)[0].args, [videoId, 12]);
    assert.equal(messages.at(-1)[1], provider);
    fromPlayer({ event: "infoDelivery", info: { currentTime: 13, duration: 120, playerState: 1 } });
    assert.deepEqual(updates.at(-1).info, { time: 13, duration: 120, state: 1 });
    host.handle({ op: "youtubeClose", id });
    assert.equal(element.removed, true);
  } finally { host.dispose(); }
});
