import assert from "node:assert/strict";
import { resolveObjectURL } from "node:buffer";
import test from "node:test";
import { createGifFixtures } from "../host/gif-fixtures.js";
import { createMedia } from "../media.js";

// The frame's half of the media bridge, with a fake parent on the other port.
function mediaFrame(t, permissions, answer) {
  const previousWindow = globalThis.window;
  const posted = [];
  let host;
  globalThis.window = { parent: { postMessage(type, _origin, [port]) {
    assert.equal(type, "stellarr:media:v1");
    host = port;
    port.onmessage = ({ data }) => { posted.push(data); answer?.(data, port); };
  } } };
  const media = createMedia({ context: {}, permissions });
  t.after(() => {
    media.dispose();
    host?.close();
    if (previousWindow) globalThis.window = previousWindow;
    else delete globalThis.window;
  });
  return { media, posted };
}

test("a GIF search and load cross the bridge, and the image is a URL of the frame's own", async t => {
  const bytes = new TextEncoder().encode("GIF89a-not-really");
  const item = { id: "AbCd1234", alt: "A cat dances.", width: 480, height: 270 };
  const { media, posted } = mediaFrame(t, ["media.gifs"], (data, port) => {
    if (data.op === "gifSearch") port.postMessage({ op: "reply", id: data.id, value: { items: [item], offset: 0, total: 1 } });
    if (data.op === "gifLoad") {
      const copy = bytes.slice().buffer;
      port.postMessage({ op: "reply", id: data.id,
        value: { bytes: copy, type: "image/gif", width: 200, height: 113, alt: item.alt } }, [copy]);
    }
  });

  assert.deepEqual(await media.gifs.search("cat", { limit: 6 }), { items: [item], offset: 0, total: 1 });
  const { id, ...search } = posted[0];
  assert.ok(Number.isSafeInteger(id));
  assert.deepEqual(search, { op: "gifSearch", query: "cat", offset: 0, limit: 6 });

  const image = await media.gifs.load("AbCd1234");
  assert.equal(posted[1].op, "gifLoad");
  assert.equal(posted[1].gif, "AbCd1234");
  assert.equal(posted[1].size, "preview");
  assert.match(image.url, /^blob:/);
  assert.deepEqual({ type: image.type, width: image.width, height: image.height, alt: image.alt },
    { type: "image/gif", width: 200, height: 113, alt: item.alt });
  const blob = resolveObjectURL(image.url);
  assert.equal(blob.type, "image/gif");
  assert.deepEqual(new Uint8Array(await blob.arrayBuffer()), bytes);
  image.release();
  assert.equal(resolveObjectURL(image.url), undefined);

  const kept = await media.gifs.load("AbCd1234", "full");
  assert.equal(posted[2].size, "full");
  media.dispose();
  assert.equal(resolveObjectURL(kept.url), undefined, "closing the session revokes the images left");
  await assert.rejects(media.gifs.search("cat"), { code: "unavailable" });
});

test("without the media.gifs permission a GIF call rejects and nothing is posted", async t => {
  const { media, posted } = mediaFrame(t, ["media.cameraTile"]);
  await assert.rejects(media.gifs.search("cat"), { code: "permission" });
  await assert.rejects(media.gifs.load("AbCd1234"), { code: "permission" });
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.deepEqual(posted, []);
});

test("the host's reason for a refusal reaches the activity as error.code", async t => {
  const { media } = mediaFrame(t, ["media.gifs"], (data, port) => {
    port.postMessage({ op: "reply", id: data.id, error: "Refused: " + data.query, code: data.query });
  });
  for (const code of ["unavailable", "declined", "rateLimited", "notFound", "failed"])
    await assert.rejects(media.gifs.search(code), { code, message: "Refused: " + code });
  await assert.rejects(media.gifs.search("teapot"), { code: "failed", message: "Refused: teapot" });
});

test("the simulator's offline GIFs load in another tab and follow its GIFs menu", async () => {
  let mode = "on";
  const searching = createGifFixtures({ mode: () => mode });
  const page = await searching.search({ query: "ż".repeat(50), offset: 0, limit: 24 });
  assert.equal(page.items.length, 24);
  assert.equal(page.total, 48);
  for (const item of page.items) assert.match(item.id, /^[A-Za-z0-9]{1,64}$/);

  // A New player tab has a provider of its own and never saw the search.
  const pick = page.items[3];
  const image = await createGifFixtures().load({ gif: pick.id, size: "preview", maxBytes: 4 * 1024 * 1024 });
  assert.equal(image.type, "image/svg+xml");
  assert.equal(image.alt, pick.alt);
  assert.equal(image.width, 200);
  assert.match(new TextDecoder().decode(image.bytes), new RegExp("^<svg .*" + pick.alt + ".*</svg>$"));
  assert.equal((await searching.load({ gif: pick.id, size: "full", maxBytes: 8 * 1024 * 1024 })).width, pick.width);

  for (const gif of ["fx99q00", "fx1qzz", "AbCd1234"])
    await assert.rejects(searching.load({ gif, size: "preview", maxBytes: 1024 }), { code: "notFound" });
  mode = "declined";
  await assert.rejects(searching.search({ query: "cat", offset: 0, limit: 12 }), { code: "declined" });
  mode = "unavailable";
  await assert.rejects(searching.load({ gif: pick.id, size: "preview", maxBytes: 1024 }), { code: "unavailable" });
});
