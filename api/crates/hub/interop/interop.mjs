#!/usr/bin/env node
// Drives the stellarr.v1 WebSocket protocol against the example hub using Node 26.
//
// Run from api/ after building the example:
//
//   cargo build -p stellarr-hub --example interop_server
//   node crates/hub/interop/interop.mjs --spawn target/debug/examples/interop_server
//
// Or pass --url http://127.0.0.1:5300 for an already running example.
// --only a,b filters scenarios by name. --skip-idle skips the heartbeat probe.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const value = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const only = value("--only")?.split(",");
const cookie = value("--cookie") ?? "session=ann";
const user = /(?:^|;\s*)session=([^;]+)/.exec(cookie)?.[1];
const otherUser = user === "bob" ? "ann" : "bob";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

let baseUrl = value("--url");
let child;
if (value("--spawn")) {
  child = spawn(value("--spawn"), ["0"], { stdio: ["ignore", "pipe", "inherit"] });
  baseUrl = await new Promise((resolve, reject) => {
    const lines = createInterface({ input: child.stdout });
    lines.on("line", (line) => {
      const port = /listening on (\d+)/.exec(line)?.[1];
      if (port) {
        lines.close();
        resolve(`http://127.0.0.1:${port}`);
      }
    });
    child.once("error", reject);
    child.once("exit", () => reject(new Error("The test server exited")));
  });
}
if (!baseUrl) {
  console.error("Specify --url URL or --spawn PATH. See the header of this file.");
  process.exit(2);
}
const hubUrl = `${baseUrl}/hubs/app`;
const sockets = new Set();

async function until(what, check, limitMs = 8_000) {
  const end = Date.now() + limitMs;
  while (Date.now() < end) {
    const result = await check();
    if (result) return result;
    await sleep(20);
  }
  throw new Error(`Timed out waiting for ${what}`);
}

async function control(path) {
  const response = await fetch(`${baseUrl}/control/${path}`, { method: "POST" });
  assert(response.ok, `Control request ${path} returned ${response.status}`);
  return response.text();
}

const call = (id, method, ...args) => ({ type: "call", id, method, args });
const send = (method, ...args) => ({ type: "send", method, args });
const result = (id, value) => ({ type: "result", id, value });
const event = (name, data) => ({ type: "event", name, data });

async function connect(sessionCookie = cookie, query = "") {
  const url = `${hubUrl.replace(/^http/, "ws")}${query}`;
  const socket = new WebSocket(url, { protocols: ["stellarr.v1"], headers: { Cookie: sessionCookie } });
  sockets.add(socket);
  const frames = [];
  let closeCode;
  socket.addEventListener("close", ({ code }) => { closeCode = code; }, { once: true });
  socket.addEventListener("message", ({ data }) => frames.push(data));
  const client = {
    closed: () => until("the WebSocket close", () => closeCode),
    send: (frame) => socket.send(JSON.stringify(frame)),
    sendRaw: (data) => socket.send(data),
    close: () => socket.close(1000),
    async take(limitMs) {
      return JSON.parse(await until("a WebSocket message", () => frames.shift(), limitMs));
    },
  };

  assert.deepEqual(await client.take(), { type: "ready" });
  assert.equal(socket.protocol, "stellarr.v1");
  assert.deepEqual(await client.take(), event("PresenceChanged", {
    userId: /(?:^|;\s*)session=([^;]+)/.exec(sessionCookie)?.[1],
    presence: "online",
  }));
  return client;
}

const cases = [
  ["ready, clock, and a group event", async () => {
    const client = await connect(cookie, "?serverId=abc");
    client.send(call(1, "GetRtcClock"));
    const clock = await client.take();
    assert.deepEqual(clock, result(1, clock.value));
    assert.equal(typeof clock.value, "number");
    assert(Math.abs(clock.value - Date.now()) < 60_000);
    assert.deepEqual(JSON.parse(await control("send?text=hello-group")), { sent: 1 });
    assert.deepEqual(await client.take(), event("Said", "hello-group"));
  }],
  ["call and send dispatch order and error envelopes", async () => {
    const client = await connect();
    client.send(call(1, "echo", "naïve"));
    client.send(send("setpresence", "idle"));
    client.send(call(2, "Nope"));
    client.send(call(3, "JoinChannel"));
    client.send(call(4, "JoinChannel", "invalid"));
    client.send(call(Number.MAX_SAFE_INTEGER, "echo", { value: null }));
    for (const expected of [
      result(1, "naïve"),
      event("PresenceChanged", { userId: user, presence: "idle" }),
      { type: "error", id: 2, code: "unknown_method" },
      { type: "error", id: 3, code: "invalid_arguments" },
      { type: "error", id: 4, code: "visible", message: "Invalid channelId." },
      result(Number.MAX_SAFE_INTEGER, { value: null }),
    ]) {
      assert.deepEqual(await client.take(), expected);
    }
  }],
  ["presence reaches both account tabs, and a group reaches everyone", async () => {
    const first = await connect();
    const second = await connect();
    const outsider = await connect(`session=${otherUser}`);
    first.send(call(1, "SetPresence", "dnd"));
    const presence = event("PresenceChanged", { userId: user, presence: "dnd" });
    assert.deepEqual(await first.take(), presence);
    assert.deepEqual(await first.take(), result(1, null));
    assert.deepEqual(await second.take(), presence);
    assert.deepEqual(JSON.parse(await control("send?text=all-tabs")), { sent: 3 });
    for (const tab of [first, second, outsider]) {
      assert.deepEqual(await tab.take(), event("Said", "all-tabs"));
    }
  }],
  ["restart closes both tabs and a fresh tab receives only fresh traffic", async () => {
    const first = await connect();
    const second = await connect();
    assert.equal(await control("restart"), "ok");
    assert.deepEqual(await Promise.all([first.closed(), second.closed()]), [1012, 1012]);
    await until("closed connections to be removed", async () => (await control("count")) === "0");
    assert.deepEqual(JSON.parse(await control("send?text=missed")), { sent: 0 });
    const fresh = await connect();
    fresh.send(call(1, "echo", "fresh"));
    assert.deepEqual(await fresh.take(), result(1, "fresh"));
  }],
  ["session revocation closes only the affected account tabs", async () => {
    const first = await connect();
    const second = await connect();
    const outsider = await connect(`session=${otherUser}`);
    assert.equal(await control(`end-session?user=${encodeURIComponent(user)}`), "ok");
    assert.deepEqual(await Promise.all([first.closed(), second.closed()]), [4401, 4401]);
    outsider.send(call(1, "echo", "still connected"));
    assert.deepEqual(await outsider.take(), result(1, "still connected"));
  }],
  ["peer and server close handshakes", async () => {
    const first = await connect();
    first.close();
    assert.equal(await first.closed(), 1000);
    const second = await connect();
    assert.equal(await control("abort"), "ok");
    assert.equal(await second.closed(), 1000);
  }],
  ["malformed messages close with a protocol error", async () => {
    for (const data of [
      "{not JSON",
      JSON.stringify([call(1, "echo")]),
      JSON.stringify(call(0, "echo")),
      JSON.stringify(call(Number.MAX_SAFE_INTEGER + 1, "echo")),
      JSON.stringify(send("echo")) + JSON.stringify(send("echo")),
      Buffer.from(JSON.stringify(send("echo"))),
    ]) {
      const client = await connect();
      client.sendRaw(data);
      assert.equal(await client.closed(), 1002);
    }
  }],
  ["maximum message size counts all UTF-8 bytes", async () => {
    const client = await connect();
    const limit = 160 * 1024;
    const emptySize = Buffer.byteLength(JSON.stringify(call(1, "SetPresence", "")));
    const room = limit - emptySize;
    const padding = "ż".repeat(Math.floor(room / 2)) + "x".repeat(room % 2);
    client.send(call(1, "SetPresence", padding));
    assert.deepEqual(await client.take(), {
      type: "error", id: 1, code: "visible", message: "Unknown presence.",
    });
    client.send(call(1, "SetPresence", padding + "x"));
    assert.equal(await client.closed(), 1009);
  }],
  ["a quiet client receives an idle heartbeat", async () => {
    const client = await connect();
    assert.deepEqual(await client.take(20_000), { type: "ping" });
  }, { slow: true }],
];

let failed = 0;
let ran = 0;
try {
  for (const [name, run, { slow = false } = {}] of cases) {
    if (only && !only.some((term) => name.includes(term))) continue;
    if (slow && flag("--skip-idle")) continue;
    ran++;
    try {
      await run();
      console.log(`PASS  ${name}`);
    } catch (error) {
      failed++;
      console.error(`FAIL  ${name}\n      ${error.message}`);
    } finally {
      for (const socket of sockets) socket.close();
      sockets.clear();
      await until("scenario connections to be removed", async () => (await control("count")) === "0");
    }
  }
  console.log(`\n${ran - failed} of ${ran} scenarios passed against the Rust hub.`);
  process.exitCode = failed || ran === 0 ? 1 : 0;
} finally {
  child?.kill();
}
