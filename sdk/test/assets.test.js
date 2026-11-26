import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdir, mkdtemp, writeFile, rm, symlink } from "node:fs/promises";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { manifestOf, pack, readRelease, unpack } from "../dev/files.js";
import { startLocal } from "../dev/local.js";
import { createTileRelay } from "../dev/tiles.js";

const manifest = { protocol: 1, id: "example", name: "Example", description: "SDK test.", version: "1.3.0-beta.2+dev" };

function localRequest(origin, token, action, route = "room") {
  return fetch(origin + "/_stellarr/dev/" + route, {
    method: "POST", headers: { Origin: origin, "Content-Type": "application/json" },
    body: JSON.stringify({ token, action }),
  });
}

test("the manifest enforces known grants, seats and SemVer", () => {
  assert.deepEqual(manifestOf(JSON.stringify(manifest)), manifest);
  for (const invalid of [{ permissions: ["microphone"] }, { version: "1.0" }, { version: "1.0.0-beta.01" },
    { seats: 0 }, { authority: "owner" }, { devices: ["watch"] }])
    assert.throws(() => manifestOf(JSON.stringify({ ...manifest, ...invalid })), { status: 400 });
});

test("launch modes are optional and a declaration contains unique supported modes", () => {
  assert.equal(manifestOf(JSON.stringify(manifest)).launchModes, undefined);
  for (const launchModes of [["voice"], ["personal"], ["personal", "voice"]])
    assert.deepEqual(manifestOf(JSON.stringify({ ...manifest, launchModes })).launchModes, launchModes);
  for (const launchModes of [[], ["other"], ["voice", "voice"], ["personal", "personal"], "voice", null])
    assert.throws(() => manifestOf(JSON.stringify({ ...manifest, launchModes })), /Expected launchModes/);
});

test("local open chooses the default mode and honors a declared override", async t => {
  const folder = await mkdtemp(join(tmpdir(), "activity-launch-modes-"));
  t.after(() => rm(folder, { recursive: true, force: true }));
  await writeFile(join(folder, "index.html"), "<title>Launch modes</title>");
  for (const { launchModes, mode, expected } of [
    { expected: "voice" },
    { launchModes: ["personal", "voice"], expected: "voice" },
    { launchModes: ["personal", "voice"], mode: "personal", expected: "personal" },
    { launchModes: ["personal", "voice"], mode: "voice", expected: "voice" },
    { launchModes: ["personal"], expected: "personal" },
  ]) {
    await writeFile(join(folder, "stellarr.json"), JSON.stringify({ ...manifest, launchModes }));
    const { origin, server } = await startLocal(folder, 0, { mode });
    try {
      const response = await localRequest(origin, randomUUID(), { op: "open", frame: randomUUID(), initialState: {} });
      assert.equal(response.status, 200);
      assert.equal((await response.json()).launchMode, expected);
    } finally {
      await new Promise(resolve => server.close(resolve));
    }
  }
  await assert.rejects(startLocal(folder, 0, { mode: "voice" }), /does not declare the voice launch mode/);
  await assert.rejects(startLocal(folder, 0, { mode: "other" }), /Choose --mode personal or --mode voice/);
});

test("personal preview tokens isolate state, host and camera tiles, and resume on reload", async t => {
  const folder = await mkdtemp(join(tmpdir(), "activity-personal-preview-"));
  t.after(() => rm(folder, { recursive: true, force: true }));
  await writeFile(join(folder, "index.html"), "<title>Personal preview</title>");
  await writeFile(join(folder, "stellarr.json"), JSON.stringify({
    ...manifest, launchModes: ["voice", "personal"], authority: "host", permissions: ["media.cameraTile"],
  }));
  let time = 1000;
  const { origin, server } = await startLocal(folder, 0, { mode: "personal", clock: () => time });
  t.after(() => new Promise(resolve => server.close(resolve)));
  const alice = { token: randomUUID(), frame: randomUUID() };
  const bob = { token: randomUUID(), frame: randomUUID() };
  const action = (player, input) => localRequest(origin, player.token, { frame: player.frame, ...input });
  const open = (player, score) => action(player, { op: "open", initialState: { score } }).then(response => response.json());
  const [first, second] = await Promise.all([open(alice, 1), open(bob, 99)]);
  assert.notEqual(first.sessionId, second.sessionId);
  assert.equal(first.host, first.me.id);
  assert.equal(second.host, second.me.id);
  assert.deepEqual(first.participants.map(person => person.id), [first.me.id]);
  assert.deepEqual(second.participants.map(person => person.id), [second.me.id]);
  assert.equal((await action(alice, { op: "write", patch: { score: 5 }, sequence: 1, epoch: first.epoch })).status, 200);
  const updatedBob = await (await action(bob, { op: "read" })).json();
  assert.deepEqual(updatedBob.changes[0].state, { score: 99 });

  const image = "data:image/jpeg;base64,/9j/2Q==";
  assert.equal((await localRequest(origin, alice.token, { op: "put", image }, "tile")).status, 200);
  for (const [player, expected] of [[alice, image], [bob, null]]) {
    const tiles = await localRequest(origin, player.token, { op: "get", ids: [first.me.id] }, "tile");
    assert.deepEqual(await tiles.json(), { [first.me.id]: expected });
  }

  assert.equal((await action(alice, { op: "disconnect" })).status, 200);
  const reloaded = { token: alice.token, frame: randomUUID() };
  const resumed = await open(reloaded, 0);
  assert.equal(resumed.sessionId, first.sessionId);
  assert.equal(resumed.me.id, first.me.id);
  assert.equal(resumed.host, first.host);
  assert.deepEqual(resumed.changes[0].state, { score: 5 });
  assert.equal((await action(alice, { op: "read" })).status, 403);

  time += (first.limits.graceSeconds + first.limits.emptySeconds + 60) * 1000;
  const later = await open({ token: alice.token, frame: randomUUID() }, 0);
  assert.notEqual(later.sessionId, first.sessionId);
  assert.deepEqual(later.changes[0].state, { score: 0 });
});

test("a dual-mode activity still shares one room across local voice tabs", async t => {
  const folder = await mkdtemp(join(tmpdir(), "activity-voice-preview-"));
  t.after(() => rm(folder, { recursive: true, force: true }));
  await writeFile(join(folder, "index.html"), "<title>Voice preview</title>");
  await writeFile(join(folder, "stellarr.json"), JSON.stringify({ ...manifest, launchModes: ["personal", "voice"], authority: "host" }));
  const { origin, server } = await startLocal(folder, 0, { mode: "voice" });
  t.after(() => new Promise(resolve => server.close(resolve)));
  const open = async score => (await localRequest(origin, randomUUID(), {
    op: "open", frame: randomUUID(), initialState: { score },
  })).json();
  const first = await open(1);
  const second = await open(99);
  assert.equal(second.sessionId, first.sessionId);
  assert.equal(second.host, first.me.id);
  assert.deepEqual(second.changes[0].state, { score: 1 });
  assert.deepEqual(second.participants.map(person => person.id), [first.me.id, second.me.id]);
});
test("the local CLI serves its shell and activity on different hosts and supports media seeks", async t => {
  const folder = await mkdtemp(join(tmpdir(), "activity-assets-"));
  t.after(() => rm(folder, { recursive: true, force: true }));
  await writeFile(join(folder, "stellarr.json"), JSON.stringify(manifest));
  await writeFile(join(folder, "index.html"), "<title>Test activity</title>");
  await writeFile(join(folder, "clip.mp4"), Buffer.from("0123456789"));
  const { server, origin } = await startLocal(folder, 0);
  t.after(() => new Promise(resolve => server.close(resolve)));
  const { frame } = await (await fetch(origin + "/_stellarr/config")).json();
  assert.notEqual(frame, origin);
  assert.match(await (await fetch(origin)).text(), /iframe id="activity"/);
  assert.equal((await fetch(origin + "/clip.mp4")).status, 404);
  function asset(path, headers = {}) {
    return new Promise((resolve, reject) => {
      request(origin + path, { headers: { host: new URL(frame).host, ...headers } }, response => {
        const chunks = [];
        response.on("data", bytes => chunks.push(bytes));
        response.on("end", () => resolve({ status: response.statusCode, headers: response.headers, body: Buffer.concat(chunks).toString() }));
      }).on("error", reject).end();
    });
  }
  assert.equal((await asset("/_stellarr/host/devices.js")).status, 404);
  assert.match((await asset("/_stellarr/sdk/media.js")).headers["permissions-policy"], /camera=\(\)/);
  const seek = await asset("/clip.mp4", { range: "bytes=3-6" });
  assert.equal(seek.status, 206);
  assert.equal(seek.body, "3456");
  assert.equal(seek.headers["content-range"], "bytes 3-6/10");
  assert.equal((await asset("/clip.mp4", { range: "bytes=11-" })).status, 416);
});

test("artwork is optional for previews and mandatory for release validation", async t => {
  const folder = await mkdtemp(join(tmpdir(), "activity-artwork-"));
  t.after(() => rm(folder, { recursive: true, force: true }));
  const manifestFile = join(folder, "stellarr.json");
  const describe = fields => writeFile(manifestFile, JSON.stringify({ ...manifest, ...fields }));
  const png = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const webp = Buffer.concat([Buffer.from("RIFF"), Buffer.alloc(4), Buffer.from("WEBP")]);
  await writeFile(join(folder, "index.html"), "<title>Artwork fixture</title>");
  await describe({});
  assert.equal(unpack(await pack(folder)).manifest.icon, undefined);
  await assert.rejects(readRelease(folder), /Add an icon path/);

  await mkdir(join(folder, "art"));
  await writeFile(join(folder, "art", "scene.webp"), webp);
  await describe({ cover: "art/scene.webp" });
  assert.equal(unpack(await pack(folder)).manifest.cover, "art/scene.webp");
  await assert.rejects(readRelease(folder), /Add an icon path/);

  await describe({ icon: "art/logo.PNG" });
  await assert.rejects(startLocal(folder, 0), /declared icon is missing/);
  await assert.rejects(readRelease(folder), /declared icon is missing/);
  await writeFile(join(folder, "art", "logo.PNG"), png);
  assert.equal((await readRelease(folder)).manifest.cover, undefined);
  await describe({ icon: "art/logo.PNG", cover: "art/scene.webp" });
  assert.equal((await readRelease(folder)).manifest.cover, "art/scene.webp");

  await writeFile(join(folder, "art", "scene.webp"), png);
  await assert.rejects(startLocal(folder, 0), /cover must match its image extension/);
  await assert.rejects(pack(folder), /cover must match its image extension/);
  await assert.rejects(readRelease(folder), /cover must match its image extension/);
  const invalid = Object.fromEntries([
    ["stellarr.json", Buffer.from(JSON.stringify({ ...manifest, icon: "logo.jpg" })).toString("base64")],
    ["index.html", "eA=="], ["logo.jpg", png.toString("base64")],
  ]);
  assert.throws(() => unpack(invalid), /icon must match its image extension/);

  await writeFile(join(folder, "art", "scene.webp"), Buffer.concat([webp, Buffer.alloc(2 * 1024 * 1024)]));
  await assert.rejects(readRelease(folder), /2048 KiB/);
  await describe({ icon: "art/logo.PNG" });
  await writeFile(join(folder, "art", "logo.PNG"), Buffer.concat([png, Buffer.alloc(512 * 1024)]));
  await assert.rejects(readRelease(folder), /512 KiB/);
  await writeFile(join(folder, "art", "logo.jpg"), Buffer.from([0xff, 0xd8, 0xff]));
  await describe({ icon: "art/logo.jpg" });
  assert.equal((await readRelease(folder)).manifest.icon, "art/logo.jpg");

  if (process.platform !== "win32") {
    await symlink("logo.jpg", join(folder, "art", "linked.jpg"));
    await describe({ icon: "art/linked.jpg" });
    await assert.rejects(startLocal(folder, 0), /must be a file inside the activity/);
    await assert.rejects(readRelease(folder), /declared icon is missing/);
  }
});

test("artwork paths use normal relative raster filenames", () => {
  for (const path of ["/icon.png", "../icon.png", "dir/../icon.png", "dir//icon.png",
    ".hidden/icon.png", "node_modules/icon.png", "_stellarr/icon.png", "dir\\icon.png",
    "dir/a:b.png", "dir/hash#.png", "dir/query?.png", "dir/%20.png", "dir/icon\n.png",
    "https://external.invalid/logo.png", "art/icon.svg", "icon.gif", "icon.avif", null]) {
    assert.throws(() => manifestOf(JSON.stringify({ ...manifest, icon: path })), { status: 400 }, String(path));
  }
  assert.equal(manifestOf(JSON.stringify({ ...manifest, icon: "art/large logo.jpeg" })).icon, "art/large logo.jpeg");
});
test("camera relay clears expired and stopped images", () => {
  let now = 0;
  const tile = createTileRelay(() => now);
  const image = "data:image/jpeg;base64,/9j/2Q==";
  tile("bob", { op: "put", image });
  assert.equal(tile("alice", { op: "get", ids: ["bob"] }).bob, image);
  now = 2000;
  assert.equal(tile("alice", { op: "get", ids: ["bob"] }).bob, null);
  tile("bob", { op: "put", image });
  tile("bob", { op: "clear" });
  assert.equal(tile("alice", { op: "get", ids: ["bob"] }).bob, null);
});
