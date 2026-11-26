import assert from "node:assert/strict";
import { createServer, request } from "node:http";
import { once } from "node:events";
import { randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { pack } from "../dev/files.js";
import { previewMiddleware } from "../dev/instance.js";

async function listen(t, handler) {
  const server = createServer(handler);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => new Promise(resolve => server.close(resolve)));
  return "http://127.0.0.1:" + server.address().port;
}

// Address one test listener as several virtual hosts, as the dev reverse proxy does.
function visit(url, { method = "GET", headers, body }) {
  return new Promise((resolve, reject) => {
    const outgoing = request(url, { method, headers }, response => {
      const chunks = [];
      response.on("data", chunk => chunks.push(chunk));
      response.on("end", () => resolve(new Response(Buffer.concat(chunks), {
        status: response.statusCode, headers: response.headers,
      })));
    });
    outgoing.on("error", reject);
    outgoing.end(body);
  });
}

async function instance(t, { development = true, avatar = null } = {}) {
  let seated = true;
  let now = 1000;
  const origin = "http://localhost:3000";
  const apiOrigin = await listen(t, (req, res) => {
    const ownerCheck = req.url === "/api/dev/activity-author";
    if (ownerCheck && !development) {
      res.writeHead(404);
      return res.end();
    }
    const allowed = ownerCheck ? req.headers.cookie === "session=owner" : seated && req.headers.cookie === "session=alice";
    if (allowed && avatar && req.url === avatar) {
      res.writeHead(200, { "content-type": "image/png", "content-length": 4 });
      return res.end("face");
    }
    res.writeHead(allowed ? 200 : 403);
    res.end(JSON.stringify({ id: "alice", name: "Alice", username: "alice", avatar }));
  });
  const preview = previewMiddleware({ origin, domain: "activity.localhost", apiOrigin, clock: () => now });
  t.after(preview.close);
  const target = await listen(t, (req, res) => preview.middleware(req, res, () => res.end()));
  const call = (path, data, extra = {}) => visit(target + "/_stellarr/dev/" + path, {
    method: "POST", headers: { host: "localhost:3000", origin, "content-type": "application/json", ...extra },
    body: JSON.stringify(data),
  });
  const files = await pack(fileURLToPath(new URL("./fixtures/minimal", import.meta.url)));
  const owner = { cookie: "session=owner" };
  const tester = { cookie: "session=alice" };

  async function author() {
    const issued = await call("pair-code", {}, owner);
    assert.equal(issued.status, 200);
    const { code } = await issued.json();
    const response = await call("pair", { code });
    assert.equal(response.status, 200);
    return { authorization: "Bearer " + (await response.json()).token };
  }
  return {
    target, call, files, owner, tester, author,
    removeSeat: () => { seated = false; },
    advance: ms => { now += ms; },
  };
}

test("the owner issues a same-origin code with one redemption and an expiry", async t => {
  const { call, owner, tester, advance } = await instance(t);
  assert.equal((await call("pair-code", {}, tester)).status, 403);
  assert.equal((await call("pair-code", {}, { ...owner, origin: "https://other.example" })).status, 403);
  const first = await (await call("pair-code", {}, owner)).json();
  assert.equal(first.instance, "http://localhost:3000");
  assert.equal(first.expiresAt, 301_000);
  assert.equal((await call("launch", { code: first.code }, tester)).status, 404);
  const competing = await Promise.all([call("pair", { code: first.code }), call("pair", { code: first.code })]);
  assert.deepEqual(competing.map(response => response.status).sort(), [200, 403]);
  const second = await (await call("pair-code", {}, owner)).json();
  advance(300_000);
  assert.equal((await call("pair", { code: second.code })).status, 403);
});

test("the preview cannot issue codes against an API in Production", async t => {
  const { call, owner } = await instance(t, { development: false });
  assert.equal((await call("pair-code", {}, owner)).status, 404);
});

test("upload replaces the prior preview and stopping revokes the upload credential", async t => {
  const { call, files, author, tester } = await instance(t);
  assert.equal((await call("drafts", { files })).status, 403);
  const first = await author();
  const second = await author();
  const { code } = await (await call("drafts", { files }, first)).json();
  const next = await (await call("drafts", { files }, first)).json();
  assert.notEqual(code, next.code);
  assert.equal((await call("launch", { code, channel: randomUUID() }, tester)).status, 404);
  assert.equal((await call("pair", { code })).status, 403);
  assert.equal((await call("drafts", { files }, { authorization: "Bearer " + code })).status, 403);
  const manifest = JSON.parse(Buffer.from(files["stellarr.json"], "base64"));
  const other = { ...files, "stellarr.json": Buffer.from(JSON.stringify({ ...manifest, id: "other" })).toString("base64") };
  assert.equal((await call("drafts", { files: other }, first)).status, 403);
  assert.equal((await call("stop", { code: next.code }, second)).status, 404);
  assert.equal((await call("stop", { code: next.code }, first)).status, 200);
  assert.equal((await call("launch", { code: next.code, channel: randomUUID() }, tester)).status, 404);
  assert.equal((await call("drafts", { files }, first)).status, 403);
});

test("voice channel and frame leases guard assets and live state after the seat is lost", async t => {
  const { target, call, files, tester, author, removeSeat } = await instance(t);
  const auth = await author();
  const { code } = await (await call("drafts", { files }, auth)).json();
  const channel = randomUUID();
  assert.equal((await call("launch", { channel, code }, { ...tester, origin: "https://other.example" })).status, 403);
  const { src, access, permissions } = await (await call("launch", { channel, code }, tester)).json();
  assert.deepEqual(permissions, []);
  const url = new URL(src);
  const asset = (path, extra = {}) => visit(target + path, { headers: { host: url.host, ...extra } });
  assert.equal((await asset("/index.html")).status, 403);
  const launch = await asset(url.pathname + url.search);
  assert.equal(launch.status, 302);
  const lease = { cookie: launch.headers.get("set-cookie").split(";")[0] };
  assert.equal((await asset(url.pathname + url.search)).status, 403);
  const document = await asset("/", lease);
  assert.equal(document.status, 200);
  const scripts = document.headers.get("content-security-policy").split(";").find(directive => directive.trim().startsWith("script-src "));
  assert.match(scripts, /'wasm-unsafe-eval'/);
  assert.doesNotMatch(scripts, /'unsafe-eval'/);
  assert.equal((await asset("/_stellarr/sdk/media.js", lease)).status, 200);
  assert.equal((await call("launch", { channel: randomUUID(), code }, tester)).status, 403);
  const frame = randomUUID();
  const action = input => call("room", { code, access, channel, action: { ...input, frame } }, tester);
  const opened = await (await action({ op: "open", initialState: {} })).json();
  assert.equal(opened.me.id, "alice");
  assert.equal(opened.launchMode, "voice");
  assert.deepEqual(opened.place, { id: channel });
  assert.equal((await call("tile", { code, access, channel, action: { op: "get", ids: [] } }, tester)).status, 403);
  removeSeat();
  assert.equal((await action({ op: "read", after: 0 })).status, 403);
  assert.equal((await action({ op: "write", patch: { a: 1 }, sequence: 1 })).status, 403);
  assert.equal((await asset("/", lease)).status, 403);
  assert.equal((await call("stop", { code }, auth)).status, 200);
  assert.equal((await asset("/", lease)).status, 404);
  assert.equal((await call("launch", { channel, code }, tester)).status, 404);
});

test("the private voice preview respects a personal-only manifest", async t => {
  const { call, files, tester, author } = await instance(t);
  const auth = await author();
  const manifest = JSON.parse(Buffer.from(files["stellarr.json"], "base64"));
  manifest.launchModes = ["personal"];
  const updated = { ...files, "stellarr.json": Buffer.from(JSON.stringify(manifest)).toString("base64") };
  const { code } = await (await call("drafts", { files: updated }, auth)).json();
  const response = await call("launch", { code, channel: randomUUID() }, tester);
  assert.equal(response.status, 400);
  assert.match(await response.text(), /supports personal launches/);
});

test("a draft and launch ticket have independent expiry", async t => {
  const { call, files, tester, author, advance, target } = await instance(t);
  const auth = await author();
  const { code } = await (await call("drafts", { files }, auth)).json();
  const channel = randomUUID();
  const { src } = await (await call("launch", { channel, code }, tester)).json();
  const frame = new URL(src);
  advance(30_000);
  assert.equal((await visit(target + frame.pathname + frame.search, { headers: { host: frame.host } })).status, 403);
  assert.equal((await call("launch", { channel, code }, tester)).status, 200);
  advance(2 * 60 * 60 * 1000);
  assert.equal((await call("launch", { channel, code }, tester)).status, 404);
  assert.equal((await call("drafts", { files }, auth)).status, 403);
});

test("a private tester resumes across uploads and sees the avatar through the game origin", async t => {
  const { target, call, files, author, tester } = await instance(t, { avatar: "/assets/" + randomUUID() });
  const auth = await author();
  const channel = randomUUID();
  const manifest = JSON.parse(Buffer.from(files["stellarr.json"], "base64"));
  files["stellarr.json"] = Buffer.from(JSON.stringify({ ...manifest, devices: ["tablet"] })).toString("base64");
  async function openDraft() {
    const { code } = await (await call("drafts", { files }, auth)).json();
    assert.equal((await call("launch", { channel, code, device: "phone" }, tester)).status, 400);
    const { access, src } = await (await call("launch", { channel, code, device: "tablet" }, tester)).json();
    const url = new URL(src);
    const fetch = (path, cookie = "") => visit(target + path, { headers: { host: url.host, cookie } });
    const cookie = (await fetch(url.pathname + url.search)).headers.get("set-cookie").split(";")[0];
    const frame = randomUUID();
    const action = async input => (await call("room", { channel, code, access, action: { ...input, frame } }, tester)).json();
    const { me, progress } = await action({ op: "open", initialState: {} });
    assert.equal(progress, true);
    assert.equal((await fetch(me.avatar)).status, 403);
    assert.equal(await (await fetch(me.avatar, cookie)).text(), "face");
    return action;
  }
  const first = await openDraft();
  assert.deepEqual(await first({ op: "saveProgress", expected: 0, data: { level: 3 } }), { revision: 1 });
  const next = await openDraft();
  assert.deepEqual(await next({ op: "loadProgress" }), { revision: 1, data: { level: 3 } });
});
