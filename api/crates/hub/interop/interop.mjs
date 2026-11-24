#!/usr/bin/env node
// Drives the real @microsoft/signalr client, and a raw WebSocket, against the
// Rust hub (through a small test server) or against the C# API it replaces,
// and checks that both answer the same way.
//
// Run by hand. It needs Node 22 or later and the repository's node_modules
// (the client library and its `ws` package); set STELLARR_NODE_MODULES when
// they are not at the top of the checkout. Nothing is installed or written.
//
// Against the Rust hub (from the api/ directory):
//
//   acargo build -p stellarr-hub --example interop_server
//   node crates/hub/interop/interop.mjs --spawn <target dir>/debug/examples/interop_server
//
// or start `interop_server 5300` yourself and pass --url http://127.0.0.1:5300.
//
// Against the C# API, started with `oracle start hub 5192` and an account
// that has signed in (the session cookie goes in --cookie):
//
//   node crates/hub/interop/interop.mjs --oracle --url http://127.0.0.1:5192 \
//        --cookie stellarr-dev-session=VALUE
//
// Options:
//   --only a,b      run only the scenarios whose names contain one of these
//   --skip-idle     skip the 70-second idle scenario
//   --verbose       print what the client library logs
//
// The scenarios that need the server to act by itself (a restart, an abort)
// use control endpoints of the test server, so they run against it only.
// Exit status is 0 when every scenario that ran passed.

import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const modules = [process.env.STELLARR_NODE_MODULES, resolve(here, "../../../../node_modules"), resolve(process.cwd(), "node_modules")]
  .filter(Boolean)
  .find((dir) => existsSync(resolve(dir, "@microsoft/signalr")));
if (!modules) {
  console.error("Cannot find node_modules/@microsoft/signalr; set STELLARR_NODE_MODULES.");
  process.exit(2);
}
const require = createRequire(resolve(modules, "@microsoft/signalr/package.json"));
const signalr = require("@microsoft/signalr");
const WebSocket = require("ws");
const { HubConnectionBuilder, HubConnectionState, LogLevel } = signalr;

// ---------------------------------------------------------------- arguments

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const value = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const oracle = flag("--oracle");
const verbose = flag("--verbose");
const only = value("--only")?.split(",");
const RS = "\x1e";

let baseUrl = value("--url");
let child = null;
if (value("--spawn")) {
  child = spawn(value("--spawn"), ["0"], { stdio: ["ignore", "pipe", "inherit"] });
  baseUrl = await new Promise((ok, fail) => {
    child.stdout.once("data", (d) => ok(`http://127.0.0.1:${/listening on (\d+)/.exec(d.toString())[1]}`));
    child.once("exit", () => fail(new Error("the test server exited")));
  });
}
if (!baseUrl) {
  console.error("Say where the server is: --url URL or --spawn PATH. See the header of this file.");
  process.exit(2);
}
const cookie = value("--cookie") ?? "session=ann";
const headers = { Cookie: cookie, "X-Stellarr-Client": "web" };
const hubUrl = `${baseUrl}/hubs/app`;
const wsUrl = (suffix = "") => `${baseUrl.replace(/^http/, "ws")}/hubs/app${suffix}`;

// ------------------------------------------------------------------ helpers

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (what, check, ms = 8000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await check()) return;
    await sleep(25);
  }
  throw new Error(`timed out waiting for ${what}`);
};
const control = (path) => fetch(`${baseUrl}/control/${path}`, { method: "POST" }).then((r) => r.text());

const quietLogger = { log: (level, message) => verbose && console.log(`    [signalr ${LogLevel[level]}] ${message}`) };

/** A real client, with the events it receives collected. */
function client({ retry = [0, 100, 100, 100, 100, 100] } = {}) {
  const connection = new HubConnectionBuilder()
    .withUrl(hubUrl, { headers, withCredentials: true })
    .withAutomaticReconnect(retry)
    .configureLogging(quietLogger)
    .build();
  const events = [];
  const log = [];
  connection.on("PresenceChanged", (p) => events.push(["PresenceChanged", p]));
  connection.on("Said", (t) => events.push(["Said", t]));
  connection.onreconnecting((e) => log.push(["reconnecting", e?.message]));
  connection.onreconnected((id) => log.push(["reconnected", id]));
  connection.onclose((e) => log.push(["closed", e?.message]));
  return { connection, events, log };
}

const rejection = async (promise) => {
  try {
    await promise;
  } catch (error) {
    return error.message;
  }
  throw new Error("expected the call to fail");
};

function assert(condition, message) {
  if (!condition) throw new Error(message);
}
function equal(actual, expected, what) {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a !== b) throw new Error(`${what}\n      got      ${a}\n      expected ${b}`);
}

/** Talks to the hub without the client library: raw frames and records. */
async function raw({ handshake = `{"protocol":"json","version":1}${RS}`, send = [], window = 500, id } = {}) {
  const negotiated = id ?? (await (await fetch(`${hubUrl}/negotiate?negotiateVersion=1`, { method: "POST", headers })).json()).connectionToken;
  const ws = new WebSocket(wsUrl(`?id=${negotiated}`), { headers });
  const frames = [];
  let closed = null;
  ws.on("message", (d) => frames.push(d.toString()));
  ws.on("close", (code) => (closed = code));
  ws.on("error", () => {});
  await new Promise((ok, fail) => ws.once("open", ok).once("error", fail));
  if (handshake !== null) ws.send(handshake);
  await sleep(150);
  for (const item of send) {
    ws.send(item);
    await sleep(60);
  }
  await sleep(window);
  const open = ws.readyState === 1;
  if (open) ws.close();
  // The records, whichever way they were packed into frames, without the
  // server's own events, which differ by design.
  const records = frames
    .join("")
    .split(RS)
    .filter(Boolean)
    .filter((r) => !/"target":"(PresenceChanged|Hello)"/.test(r) && r !== '{"type":6}');
  return { records, closed: open ? null : closed };
}
const call = (id, target, ...a) => JSON.stringify({ type: 1, invocationId: id, target, arguments: a }) + RS;

// --------------------------------------------------------------- scenarios

const scenarios = [];
const scenario = (name, run, { ours = true, theirs = true, slow = false } = {}) => scenarios.push({ name, run, ours, theirs, slow });

scenario("negotiate: version 1 answer", async () => {
  const response = await fetch(`${hubUrl}/negotiate?negotiateVersion=1`, { method: "POST", headers });
  assert(response.status === 200, `status ${response.status}`);
  const answer = await response.json();
  equal(answer.negotiateVersion, 1, "negotiateVersion");
  assert(answer.connectionId.length === 22 && answer.connectionToken.length === 22, "ids are 22 characters");
  assert(answer.connectionId !== answer.connectionToken, "id and token differ");
  const ws = answer.availableTransports.find((t) => t.transport === "WebSockets");
  equal(ws, { transport: "WebSockets", transferFormats: ["Text", "Binary"] }, "the WebSockets entry");
  console.log(`      transports offered: ${answer.availableTransports.map((t) => t.transport).join(", ")}`);
});

scenario("negotiate: refusals", async () => {
  equal((await fetch(`${hubUrl}/negotiate?negotiateVersion=1`, { method: "POST" })).status, 401, "no cookie");
  equal((await fetch(`${hubUrl}/negotiate?negotiateVersion=1`, { headers })).status, 405, "GET");
  const v0 = await (await fetch(`${hubUrl}/negotiate`, { method: "POST", headers })).json();
  equal(v0.negotiateVersion, 0, "no version asks for 0");
  assert(!("connectionToken" in v0), "version 0 has no token");
  const bad = await (await fetch(`${hubUrl}/negotiate?negotiateVersion=x`, { method: "POST", headers })).json();
  equal(bad, { error: "The client requested a non-integer protocol version." }, "a version that is not a number");
  equal((await fetch(`${hubUrl}`, { headers })).status, 400, "a plain GET with no id");
  equal((await fetch(`${hubUrl}?id=nope`, { headers })).status, 404, "an unknown id");
});

scenario("connect, call with a result, call that fails, send without a result", async () => {
  const { connection, events } = client();
  await connection.start();
  equal(connection.state, HubConnectionState.Connected, "state");
  const clock = await connection.invoke("GetRtcClock");
  assert(Math.abs(clock - Date.now()) < 60_000, `clock ${clock}`);
  equal(await connection.invoke("SetPresence", "online"), null, "a method without a result answers null");
  equal(
    await rejection(connection.invoke("SetPresence", "bogus")),
    "An unexpected error occurred invoking 'SetPresence' on the server. HubException: Unknown presence.",
    "a hub exception reaches the caller with its text",
  );
  equal(
    await rejection(connection.invoke("NoSuchMethod")),
    "Failed to invoke 'NoSuchMethod' due to an error on the server. HubException: Method does not exist.",
    "an unknown method",
  );
  equal(
    await rejection(connection.invoke("SetPresence")),
    "Failed to invoke 'SetPresence' due to an error on the server.",
    "missing arguments are not explained",
  );
  equal(
    await rejection(connection.invoke("SetPresence", 5)),
    "Failed to invoke 'SetPresence' due to an error on the server.",
    "an argument of the wrong kind",
  );
  events.length = 0;
  await connection.send("SetPresence", "idle");
  await until("the event a send caused", () => events.some(([n, p]) => n === "PresenceChanged" && p.presence === "idle"));
  await connection.send("SetPresence", "bogus"); // fails on the server; the client is never told
  await sleep(200);
  equal(connection.state, HubConnectionState.Connected, "a failed send leaves the connection alone");
  await connection.stop();
  equal(connection.state, HubConnectionState.Disconnected, "state after stop");
});

scenario("events sent to a group reach its members", async () => {
  const a = client();
  const b = client();
  await Promise.all([a.connection.start(), b.connection.start()]);
  a.events.length = b.events.length = 0;
  // The account's other tab hears about a change one tab makes.
  await a.connection.invoke("SetPresence", "dnd");
  await until("the other tab's event", () => b.events.some(([, p]) => p.presence === "dnd"));
  await until("the caller's own event", () => a.events.some(([, p]) => p.presence === "dnd"));
  await a.connection.invoke("SetPresence", "online");
  await Promise.all([a.connection.stop(), b.connection.stop()]);
});

scenario("events sent by the server to a group", async () => {
  const a = client();
  await a.connection.start();
  const sent = JSON.parse(await control("send?group=user:ann&text=hello-group"));
  assert(sent.sent >= 1, "the group had a member");
  await until("the event", () => a.events.some(([n, t]) => n === "Said" && t === "hello-group"));
  await a.connection.stop();
}, { theirs: false });

scenario("idle for 70 seconds with only the keep-alive between us", async () => {
  const { connection, log } = client();
  await connection.start();
  const id = connection.connectionId;
  for (let waited = 0; waited < 70; waited += 10) {
    await sleep(10_000);
    equal(connection.state, HubConnectionState.Connected, `state after ${waited + 10} s`);
  }
  equal(connection.connectionId, id, "still the same connection");
  equal(log, [], "no reconnect, no close");
  assert(Number.isFinite(await connection.invoke("GetRtcClock")), "still answers");
  await connection.stop();
}, { slow: true });

/** The length of the record the client library writes for a call, found by looking. */
async function recordLength(connection, arg) {
  let length = 0;
  const original = connection.connection.send.bind(connection.connection);
  connection.connection.send = (data) => {
    length = data.length;
    return original(data);
  };
  await rejection(connection.invoke("SetPresence", arg));
  connection.connection.send = original;
  return length;
}

const LIMIT = 160 * 1024;
scenario("a record just under the limit is served, one byte over closes the connection and the client starts over", async () => {
  const { connection, log } = client();
  await connection.start();
  const base = await recordLength(connection, "");
  const first = connection.connectionId;
  const fits = "x".repeat(LIMIT - base);
  equal(await recordLength(connection, fits), LIMIT, "the record is exactly at the limit");
  equal(connection.state, HubConnectionState.Connected, "still connected after the largest allowed record");
  // One more byte.
  const pending = rejection(connection.invoke("SetPresence", "x".repeat(LIMIT - base + 1)));
  const why = await pending;
  console.log(`      the pending call failed with: ${why}`);
  await until("the automatic reconnect", () => log.some(([n]) => n === "reconnected"));
  equal(log.map(([n]) => n), ["reconnecting", "reconnected"], "the client reconnected once");
  assert(log[0][1]?.includes("Connection closed with an error."), `the close carried the error: ${log[0][1]}`);
  assert(connection.connectionId !== first, "a new connection");
  assert(Number.isFinite(await connection.invoke("GetRtcClock")), "the new connection works");
  await connection.stop();
});

scenario("a close started by the server is followed by the client's reconnect", async () => {
  const { connection, log } = client();
  await connection.start();
  const first = connection.connectionId;
  await control("restart");
  await until("the reconnect", () => log.some(([n]) => n === "reconnected"));
  assert(connection.connectionId !== first, "a new connection");
  assert(Number.isFinite(await connection.invoke("GetRtcClock")), "the new connection works");
  await connection.stop();
}, { theirs: false });

scenario("a close with an error is reported to the client, which reconnects", async () => {
  const { connection, log } = client();
  await connection.start();
  await control("close-with-error");
  await until("the reconnect", () => log.some(([n]) => n === "reconnected"));
  assert(String(log[0][1]).includes("test close"), `the error reached the client: ${log[0][1]}`);
  await connection.stop();
}, { theirs: false });

scenario("an abort by the server is final: the client does not come back", async () => {
  const { connection, log } = client();
  await connection.start();
  await control("abort");
  await until("the close", () => connection.state === HubConnectionState.Disconnected);
  await sleep(600);
  equal(log.map(([n]) => n), ["closed"], "closed once, no reconnect");
  equal(await (await fetch(`${baseUrl}/control/count`, { method: "POST" })).text(), "0", "the server has no connections left");
}, { theirs: false });

scenario("an abrupt drop of the connection is followed by a reconnect", async () => {
  const { connection, log } = client();
  await connection.start();
  const first = connection.connectionId;
  connection.connection.transport._webSocket.terminate();
  await until("the reconnect", () => log.some(([n]) => n === "reconnected"));
  equal(log.map(([n]) => n), ["reconnecting", "reconnected"], "reconnecting, then reconnected");
  assert(connection.connectionId !== first, "a new connection");
  assert(Number.isFinite(await connection.invoke("GetRtcClock")), "the new connection works");
  await connection.stop();
});

scenario("stopping the client leaves nothing behind on the server", async () => {
  await until("the server to be quiet", async () => Number(await control("count")) === 0);
  const before = 0;
  const { connection } = client();
  await connection.start();
  equal(Number(await control("count")), before + 1, "one more connection");
  await connection.stop();
  await until("the connection to be forgotten", async () => Number(await control("count")) === before);
}, { theirs: false });

// A record at a time, what the hub answers to things the client library
// never sends but other clients might. The expected answers are what the
// C# server was recorded giving.
const GENERAL = `{"error":"An unexpected error occurred during connection handshake."}`;
const CLOSED = `{"type":7,"error":"Connection closed with an error.","allowReconnect":true}`;
const completion = (id, error) => `{"type":3,"invocationId":"${id}","error":${JSON.stringify(error)}}`;
const noMethod = (t) => `Failed to invoke '${t}' due to an error on the server. HubException: Method does not exist.`;
const probes = [
  ["handshake: messagepack", { handshake: `{"protocol":"messagepack","version":1}${RS}` }, [`{"error":"The protocol 'messagepack' is not supported."}`], 1000],
  ["handshake: fractional version", { handshake: `{"protocol":"json","version":1.5}${RS}` }, [GENERAL], 1000],
  ["handshake: not JSON", { handshake: `garbage${RS}` }, [GENERAL], 1000],
  ["handshake: no protocol", { handshake: `{"version":1}${RS}` }, [GENERAL], 1000],
  ["handshake: no version", { handshake: `{"protocol":"json"}${RS}` }, [GENERAL], 1000],
  ["handshake: empty", { handshake: RS }, [GENERAL], 1000],
  ["handshake: version 0 and 2 and any case are fine", { handshake: `{"protocol":"JSON","version":2}${RS}` }, ["{}"], null],
  ["handshake and a call in one message", { handshake: `{"protocol":"json","version":1}${RS}` + call("1", "Nope") }, ["{}", completion("1", noMethod("Nope"))], null],
  ["unknown method, in any spelling", { send: [call("1", "Nope"), call("2", "OnConnectedAsync"), call("3", "")] }, [completion("1", noMethod("Nope")), completion("2", noMethod("OnConnectedAsync")), completion("3", noMethod(""))], null],
  ["method names ignore case", { send: [call("1", "setpresence", "online")] }, [`{"type":3,"invocationId":"1","result":null}`], null],
  ["two calls with the same id are both answered", { send: [call("1", "SetPresence", "online") + call("1", "SetPresence", "online")] }, [`{"type":3,"invocationId":"1","result":null}`, `{"type":3,"invocationId":"1","result":null}`], null],
  ["calls with no id or an empty id get no answer", { send: [JSON.stringify({ type: 1, target: "SetPresence", arguments: ["bogus"] }) + RS, JSON.stringify({ type: 1, invocationId: "", target: "Nope", arguments: [] }) + RS] }, [], null],
  ["a stream invocation of a plain method", { send: [JSON.stringify({ type: 4, invocationId: "1", target: "GetRtcClock", arguments: [], streamIds: [] }) + RS] }, [completion("1", "The client attempted to invoke the non-streaming 'GetRtcClock' method with a streaming invocation.")], null],
  ["a stream invocation of an unknown method", { send: [JSON.stringify({ type: 4, invocationId: "1", target: "Nope", arguments: [], streamIds: [] }) + RS] }, [completion("1", noMethod("Nope"))], null],
  ["upload streams the method does not take", { send: [JSON.stringify({ type: 1, invocationId: "1", target: "SetPresence", arguments: ["online"], streamIds: ["a"] }) + RS] }, [completion("1", "An unexpected error occurred invoking 'SetPresence' on the server. HubException: Client sent 1 stream(s), Hub method expects 0.")], null],
  ["messages that are skipped", { send: [{ type: 6 }, { type: 5, invocationId: "1" }, { type: 8, sequenceId: 1 }, { type: 9, sequenceId: 1 }, { type: 2, invocationId: "1", item: 1 }, { type: 3, invocationId: "1", result: 1 }].map((m) => JSON.stringify(m) + RS).concat([call("9", "SetPresence", "online")]) }, [`{"type":3,"invocationId":"9","result":null}`], null],
  ["unknown fields are skipped", { send: [JSON.stringify({ type: 1, invocationId: "1", target: "SetPresence", arguments: ["online"], zzz: { a: [1] } }) + RS] }, [`{"type":3,"invocationId":"1","result":null}`], null],
  ["the client closes", { send: [JSON.stringify({ type: 7 }) + RS] }, [`{"type":7}`], 1000],
  ["a record that is not JSON", { send: [`{not json${RS}`] }, [CLOSED], 1000],
  ["a record that is not an object", { send: [`[1,2]${RS}`] }, [CLOSED], 1000],
  ["an empty record", { send: [RS] }, [CLOSED], 1000],
  ["a record with no type", { send: [JSON.stringify({ invocationId: "1", target: "Nope", arguments: [] }) + RS] }, [CLOSED], 1000],
  ["an invocation with no arguments", { send: [JSON.stringify({ type: 1, invocationId: "1", target: "Nope" }) + RS] }, [CLOSED], 1000],
  ["an invocation with a null target", { send: [JSON.stringify({ type: 1, invocationId: "1", target: null, arguments: [] }) + RS] }, [CLOSED], 1000],
  ["an invocation id that is a number", { send: [JSON.stringify({ type: 1, invocationId: 5, target: "Nope", arguments: [] }) + RS] }, [CLOSED], 1000],
  ["an invocation id that is null", { send: [JSON.stringify({ type: 1, invocationId: null, target: "Nope", arguments: [] }) + RS] }, [CLOSED], 1000],
  ["arguments that are an object", { send: [JSON.stringify({ type: 1, invocationId: "1", target: "SetPresence", arguments: {} }) + RS] }, [CLOSED], 1000],
  ["a known message missing its parts", { send: [JSON.stringify({ type: 2, invocationId: "1" }) + RS] }, [CLOSED], 1000],
  ["a completion with a result and an error", { send: [JSON.stringify({ type: 3, invocationId: "1", result: 1, error: "x" }) + RS] }, [CLOSED], 1000],
  ["a cancel whose id is a number", { send: [JSON.stringify({ type: 5, invocationId: 1 }) + RS] }, [CLOSED], 1000],
  ["a sequence message with no sequence", { send: [JSON.stringify({ type: 9 }) + RS] }, [CLOSED], 1000],
  ["stream ids that are null", { send: [JSON.stringify({ type: 1, invocationId: "1", target: "GetRtcClock", arguments: [], streamIds: null }) + RS] }, [CLOSED], 1000],
  ["headers are skipped when they are strings", { send: [JSON.stringify({ type: 1, invocationId: "1", target: "SetPresence", arguments: ["online"], headers: { a: "b" } }) + RS] }, [`{"type":3,"invocationId":"1","result":null}`], null],
  ["an unknown message type is not an error", { send: [JSON.stringify({ type: 99 }) + RS] }, [], null],
  ["a type that is not a number", { send: [JSON.stringify({ type: "6" }) + RS] }, [CLOSED], 1000],
  ["a record without its separator waits", { send: [`{"type":6}`] }, [], null],
  ["a binary message is read like text", { send: [Buffer.from(call("1", "SetPresence", "online"))] }, [`{"type":3,"invocationId":"1","result":null}`], null],
  ["a record of 163840 bytes is served", { send: [(() => { const b = call("1", "SetPresence", "").length; return call("1", "SetPresence", "x".repeat(LIMIT - b)); })()], window: 800 }, [completion("1", "An unexpected error occurred invoking 'SetPresence' on the server. HubException: Unknown presence.")], null],
  ["a record of 163841 bytes is refused", { send: [(() => { const b = call("1", "SetPresence", "").length; return call("1", "SetPresence", "x".repeat(LIMIT - b + 1)); })()], window: 800 }, [CLOSED], 1000],
  ["163840 bytes with no separator still wait", { send: ["x".repeat(LIMIT)], window: 800 }, [], null],
  ["163841 bytes with no separator are refused", { send: ["x".repeat(LIMIT + 1)], window: 800 }, [CLOSED], 1000],
  ["a handshake over the limit", { handshake: `{"protocol":"json","version":1,"pad":"${"x".repeat(200_000)}"}${RS}`, window: 800 }, [`{"error":"Handshake was canceled."}`], 1000],
];
for (const [name, options, expected, closed] of probes) {
  scenario(`raw: ${name}`, async () => {
    const got = await raw(options);
    // Calls that follow the handshake show the handshake's `{}` first.
    const records = got.records[0] === "{}" && !expected.includes("{}") ? got.records.slice(1) : got.records;
    equal(records, expected, "records");
    equal(got.closed, closed, "close code (null when the connection stays open)");
  });
}

scenario("raw: the server's ping beat", async () => {
  const got = await raw({ window: 16_500 });
  assert(got.records !== undefined, "ran");
}, { slow: true });

scenario("raw: a handshake that never comes is cancelled after 15 seconds", async () => {
  const got = await raw({ handshake: null, window: 16_000 });
  equal(got.records, [`{"error":"Handshake was canceled."}`], "records");
  equal(got.closed, 1000, "close code");
}, { slow: true });

// --------------------------------------------------------------------- run

let failed = 0;
let ran = 0;
for (const s of scenarios) {
  if ((oracle && !s.theirs) || (!oracle && !s.ours)) continue;
  if (only && !only.some((o) => s.name.includes(o))) continue;
  if (s.slow && flag("--skip-idle")) continue;
  ran++;
  const started = Date.now();
  try {
    await s.run();
    console.log(`PASS  ${s.name} (${((Date.now() - started) / 1000).toFixed(1)} s)`);
  } catch (error) {
    failed++;
    console.log(`FAIL  ${s.name}\n      ${error.message}`);
  }
}
console.log(`\n${ran - failed} of ${ran} scenarios passed against ${oracle ? "the C# API" : "the Rust hub"}.`);
child?.kill();
process.exit(failed ? 1 : 0);
