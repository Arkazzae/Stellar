// What clients of the hub and of the two moderation routes observe of voice, as
// a transcript, from one server with a fake media server behind it. Run it
// against this server and against the first one, on copies of one database
// (see voice-oracle.sh), and compare the two transcripts.
//
//   node voice-oracle.mjs --base http://127.0.0.1:5000 --fake http://127.0.0.1:5232 \
//        --ids ids.json --out transcript.txt
//
// Needs Node 26 (the global WebSocket takes request headers). The accounts and
// rooms it uses are made by the shell script. Ids are replaced by names so that
// the two transcripts can be compared; the requests the fake took are written
// at the end.

import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({
  options: { base: { type: "string" }, fake: { type: "string" }, ids: { type: "string" }, out: { type: "string" }, password: { type: "string", default: "Corona!2026-long" }, only: { type: "string", default: "" } },
});
const base = args.base;
const fake = args.fake;
const ids = JSON.parse(readFileSync(args.ids, "utf8"));
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const only = args.only ? args.only.split(",") : null;

const names = new Map();
for (const [name, id] of Object.entries(ids)) if (typeof id === "string") names.set(id.toLowerCase(), name);
const lines = [];
const out = (text) => lines.push(text);

function scrub(value) {
  let text = typeof value === "string" ? value : JSON.stringify(value);
  for (const [id, name] of names) text = text.replaceAll(id, `<${name}>`).replaceAll(id.toUpperCase(), `<${name}>`);
  return text
    .replace(/\b1[67]\d{11}\b/g, "<ms>")
    .replace(/"playbackId":"[0-9a-f-]{36}"/g, '"playbackId":"<guid>"')
    .replace(/"traceId":"[^"]*"/g, '"traceId":"<trace>"');
}

async function signIn(username) {
  const response = await fetch(`${base}/api/auth/login`, { method: "POST", headers: clientHeaders, body: JSON.stringify({ username, password: args.password }) });
  if (response.status !== 200) throw new Error(`signing in ${username}: ${response.status}`);
  const body = await response.json();
  const cookie = response.headers.getSetCookie().map((line) => line.split(";")[0]).join("; ");
  names.set(body.id.toLowerCase(), username);
  ids[username] = body.id;
  return { username, cookie, id: body.id };
}

class Client {
  constructor(label, person, query = "") {
    Object.assign(this, { label, person, query, records: [], waiting: new Map(), next: 0, buffer: "", closed: false });
  }

  async open() {
    this.socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app${this.query}`, { headers: { Cookie: this.person.cookie } });
    await new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = () => reject(new Error(`the upgrade for ${this.label} failed`));
    });
    this.socket.onmessage = (event) => this.onText(String(event.data));
    this.socket.onclose = () => {
      this.closed = true;
      this.records.push({ closedSocket: true });
    };
    this.socket.send(`{"protocol":"json","version":1}${RS}`);
    return this;
  }

  onText(text) {
    this.buffer += text;
    let end;
    while ((end = this.buffer.indexOf(RS)) >= 0) {
      const record = JSON.parse(this.buffer.slice(0, end));
      this.buffer = this.buffer.slice(end + 1);
      if (record.type === 6) continue;
      if (record.type === 3 && this.waiting.has(record.invocationId)) {
        this.waiting.get(record.invocationId)(record);
        continue;
      }
      this.records.push(record);
    }
  }

  send(target, ...argumentsList) {
    const id = String(++this.next);
    const done = new Promise((resolve) => this.waiting.set(id, resolve));
    this.socket.send(JSON.stringify({ type: 1, invocationId: id, target, arguments: argumentsList }) + RS);
    return done;
  }

  async invoke(target, ...argumentsList) {
    const completion = await Promise.race([this.send(target, ...argumentsList), sleep(8000).then(() => ({ timeout: true }))]);
    return completion;
  }

  take() {
    const taken = this.records;
    this.records = [];
    return taken;
  }

  close() {
    try {
      this.socket.close(1000);
    } catch {}
  }
}

const clients = [];
async function connect(label, person, query = "") {
  const client = await new Client(label, person, query).open();
  clients.push(client);
  return client;
}

// What the fake took since the last step, without the polls, which come as often
// as the clock says.
async function fakeRequests() {
  const taken = await (await fetch(`${fake}/_fake/requests`)).json();
  await fetch(`${fake}/_fake/clear`);
  const shown = [];
  let polls = [];
  for (const request of taken) {
    if (request.path === "/v1/events") {
      const after = new URLSearchParams(request.query).get("epoch") + "/" + new URLSearchParams(request.query).get("after");
      if (polls[polls.length - 1] !== after) polls.push(after);
      continue;
    }
    shown.push(`${request.method} ${request.path}${request.query ? "?" + request.query : ""} ${request.body}`);
  }
  return { shown, polls };
}

async function admin(operation, query = "") {
  return (await fetch(`${fake}/_fake/${operation}?${query}`)).json();
}

let section = "";
// PresenceChanged is left out but where it is looked at: who hears of a status
// first is the order the first server happened to connect people in.
const looked = new Set(["disconnecting"]);
const polled = new Set();
async function step(title, action, settle = 500, quiet = false) {
  if (only && !only.includes(section)) return;
  out(`## ${title}`);
  try {
    const result = await action?.();
    if (result !== undefined) out(`   result: ${scrub(result)}`);
  } catch (error) {
    out(`   error: ${error.message}`);
  }
  await sleep(settle);
  for (const client of clients) {
    const taken = client.take();
    if (quiet) continue;
    for (const record of taken) {
      if (record.target === "PresenceChanged" && !looked.has(section)) continue;
      out(`   ${client.label}: ${scrub(record)}`);
    }
  }
  const { shown, polls } = await fakeRequests();
  // Closing the sessions of a room's seats is done in the order the first server
  // happened to keep them in, which nobody can tell.
  for (let first = 0; first < shown.length; ) {
    let last = first;
    while (last < shown.length && shown[last].startsWith("DELETE ")) last += 1;
    if (last > first) shown.splice(first, last - first, ...shown.slice(first, last).sort());
    first = Math.max(last, first + 1);
  }
  for (const request of shown) out(`   media <- ${scrub(request)}`);
  for (const place of polls) polled.add(place);
}

function sectionOf(name) {
  section = name;
  if (!only || only.includes(name)) out(`# ${name}`);
}

async function call(client, target, ...argumentsList) {
  const completion = await client.invoke(target, ...argumentsList);
  return { call: `${target}(${argumentsList.map((a) => JSON.stringify(a)).join(", ")})`, completion };
}

// A call that comes back as a text the way the hub answers a refusal.
async function text(client, target, ...argumentsList) {
  const completion = await client.invoke(target, ...argumentsList);
  const shown = completion.error ?? (completion.result === undefined || completion.result === null ? "ok" : JSON.stringify(completion.result));
  return `${target}(${argumentsList.map((a) => JSON.stringify(a)).join(", ").slice(0, 160)}) -> ${shown.replace(/^An unexpected error occurred invoking '\w+' on the server\. HubException: /, "refused: ")}`;
}

async function http(person, method, path, body, headers = {}) {
  const response = await fetch(`${base}${path}`, {
    method,
    headers: { ...clientHeaders, ...(person ? { Cookie: person.cookie } : {}), ...headers },
    body: body === undefined ? undefined : typeof body === "string" ? body : JSON.stringify(body),
  });
  const bodyText = await response.text();
  return `${method} ${path} -> ${response.status}${bodyText ? " " + bodyText.slice(0, 200) : ""}`;
}

const sig = (room, over = {}) => ({ protocolVersion: 1, channelId: room, toUserId: null, kind: "state", payload: '{"muted":false}', ...over });
const OFFER = '{"type":"offer","sdp":"v=0 browser a+b"}';
const ANSWER = '{"type":"answer","sdp":"v=0 answer","negotiationId":7}';
const CANDIDATE = '{"candidate":"candidate:1 1 udp 2122260223 192.0.2.1 54321 typ host","sdpMid":"0","sdpMLineIndex":0}';
const media = (room, session, kind, payload, over = {}) => ({ protocolVersion: 1, channelId: room, sessionId: session, kind, payload, ...over });
const V = (n) => ids[`v${n}`];

const people = {};
for (const name of ["ola", "ben", "cat", "dan", "eve", "fay"]) people[name] = await signIn(name);
const second = await signIn("ola");

// ---------------------------------------------------------------- connecting
sectionOf("connecting");
const ola = await connect("ola", people.ola);
const ben = await connect("ben", people.ben);
const cat = await connect("cat", people.cat);
const dan = await connect("dan", people.dan);
const eve = await connect("eve", people.eve);
const fay = await connect("fay", people.fay);
await step("everybody connects");

// ----------------------------------------------------------- joining rooms
sectionOf("rooms");
for (const value of [null, 5, true, "", "bad", ["x"], {}, V(1).toUpperCase(), V(1).replaceAll("-", ""), `{${V(1)}}`, ` ${V(1)} `, ids.t1, ids.far, "00000000-0000-0000-0000-000000000000"]) {
  await step(`fay joins ${JSON.stringify(value)}`, () => text(fay, "JoinVoice", value));
  await step("fay leaves", () => text(fay, "LeaveVoice"));
}
await step("JoinVoice with no arguments", () => text(ben, "JoinVoice"));
await step("JoinVoice with two arguments", () => text(ben, "JoinVoice", V(1), V(1)));
await step("LeaveVoice with an argument", () => text(ben, "LeaveVoice", 1));
await step("ben joins the first room", () => text(ben, "JoinVoice", V(1)));
await step("cat joins the first room", () => text(cat, "JoinVoice", V(1)));
await step("ben joins it again", () => text(ben, "JoinVoice", V(1)));
await step("ben moves to the second room without leaving", () => text(ben, "JoinVoice", V(2)));
await step("ola joins the first room", () => text(ola, "JoinVoice", V(1)));
const olaTwo = await connect("ola2", second);
await step("ola opens a second tab (idle)");
await step("ola's second tab joins the first room, and takes her seat", () => text(olaTwo, "JoinVoice", V(1)));
await step("ola's first tab signals from a seat it lost", () => text(ola, "SendVoiceSignal", sig(V(1))));
await step("ola's first tab leaves, which changes nothing", () => text(ola, "LeaveVoice"));
await step("ola's second tab leaves", () => text(olaTwo, "LeaveVoice"));
await step("cat leaves", () => text(cat, "LeaveVoice"));
await step("cat leaves again", () => text(cat, "LeaveVoice"));
await step("ben leaves", () => text(ben, "LeaveVoice"));

// -------------------------------------------------------------- permissions
sectionOf("permissions");
await step("eve joins a room she is denied Connect at", () => text(eve, "JoinVoice", V(2)));
await step("eve opens media without a seat", () => text(eve, "OpenMediaSession", V(2)));
await step("fay joins a room she cannot see", () => text(fay, "JoinVoice", V(2)));
await step("fay joins a private room", () => text(fay, "JoinVoice", V(3)));
await step("dan joins a private room", () => text(dan, "JoinVoice", V(3)));
await step("ola joins the private room", () => text(ola, "JoinVoice", V(3)));
await step("dan joins a room that needs Connect", () => text(dan, "JoinVoice", V(5)));
await step("ola joins that room", () => text(ola, "JoinVoice", V(5)));
await step("ola leaves", () => text(ola, "LeaveVoice"));
await step("eve joins the first room", () => text(eve, "JoinVoice", V(1)));
await step("eve moves to the room she may not connect to", () => text(eve, "JoinVoice", V(2)));
await step("eve signals to the first room", () => text(eve, "SendVoiceSignal", sig(V(1))));
await step("eve leaves", () => text(eve, "LeaveVoice"));
await step("ben fills the small room", () => text(ben, "JoinVoice", V(4)));
await step("cat tries the small room", () => text(cat, "JoinVoice", V(4)));
await step("ben joins the small room again", () => text(ben, "JoinVoice", V(4)));
await step("ben leaves, cat joins it", async () => [await text(ben, "LeaveVoice"), await text(cat, "JoinVoice", V(4))]);
await step("cat leaves", () => text(cat, "LeaveVoice"));
await step("a room of the third server, which ola is a member of", () => text(ola, "JoinVoice", ids.v6));
await step("ola leaves it", () => text(ola, "LeaveVoice"));
await step("dan is no member of the third server", () => text(dan, "JoinVoice", ids.v6));

// ------------------------------------------------------------------ signals
sectionOf("signals");
await step("ben, cat and eve join the first room", async () => [await text(ben, "JoinVoice", V(1)), await text(cat, "JoinVoice", V(1)), await text(eve, "JoinVoice", V(1))]);
await step("ben signals cat alone", () => text(ben, "SendVoiceSignal", sig(V(1), { toUserId: ids.cat, payload: '{"screenStreamId":"screen-1"}' })));
await step("ben signals the room", () => text(ben, "SendVoiceSignal", sig(V(1), { payload: '{"screenStreamId":null,"cameraStreamId":"c"}' })));
await step("ben mutes himself", () => text(ben, "SendVoiceSignal", sig(V(1), { payload: '{"muted":true,"screenStreamId":null}' })));
await step("ben says muted twice", () => text(ben, "SendVoiceSignal", sig(V(1), { payload: '{"muted":true,"muted":false}' })));
await step("ben says Muted", () => text(ben, "SendVoiceSignal", sig(V(1), { payload: '{"Muted":true}' })));
await step("ben says muted with something else", () => text(ben, "SendVoiceSignal", sig(V(1), { payload: '{"muted":"yes"}' })));
await step("ben signals himself", () => text(ben, "SendVoiceSignal", sig(V(1), { toUserId: ids.ben })));
await step("ben signals a target in capitals", () => text(ben, "SendVoiceSignal", sig(V(1), { toUserId: ids.cat.toUpperCase() })));
const refusals = [
  ["a kind of no use", sig(V(1), { kind: "hello" })],
  ["an offer", sig(V(1), { kind: "offer", payload: OFFER })],
  ["a kind in capitals", sig(V(1), { kind: "STATE" })],
  ["no kind", (() => { const s = sig(V(1)); delete s.kind; return s; })()],
  ["a null kind", sig(V(1), { kind: null })],
  ["a number for a kind", sig(V(1), { kind: 5 })],
  ["version 2", sig(V(1), { protocolVersion: 2 })],
  ["version 0", sig(V(1), { protocolVersion: 0 })],
  ["a version in text", sig(V(1), { protocolVersion: "1" })],
  ["a null version", sig(V(1), { protocolVersion: null })],
  ["no version", (() => { const s = sig(V(1)); delete s.protocolVersion; return s; })()],
  ["a fractional version", sig(V(1), { protocolVersion: 1.5 })],
  ["the members in capitals", { PROTOCOLVERSION: 1, CHANNELID: V(1), KIND: "state", PAYLOAD: '{"muted":false}' }],
  ["an extra member", sig(V(1), { extra: [1] })],
  ["an object for a payload", sig(V(1), { payload: { muted: true } })],
  ["a null payload", sig(V(1), { payload: null })],
  ["an empty payload", sig(V(1), { payload: "" })],
  ["a payload that is a list", sig(V(1), { payload: "[]" })],
  ["a payload of null", sig(V(1), { payload: "null" })],
  ["a payload with a comment", sig(V(1), { payload: '{"muted":true /*c*/}' })],
  ["a payload with a trailing comma", sig(V(1), { payload: '{"muted":true,}' })],
  ["a payload after a byte order mark", sig(V(1), { payload: '﻿{"muted":true}' })],
  ["a payload nested 64 deep", sig(V(1), { payload: '{"a":'.repeat(64) + "1" + "}".repeat(64) })],
  ["a payload nested 65 deep", sig(V(1), { payload: '{"a":'.repeat(65) + "1" + "}".repeat(65) })],
  ["a list nested 65 deep", sig(V(1), { payload: "[".repeat(65) + "]".repeat(65) })],
  ["a payload of exactly 128 KiB", sig(V(1), { payload: `{"pad":"${"x".repeat(128 * 1024 - 10)}"}` })],
  ["a payload of one byte more", sig(V(1), { payload: `{"pad":"${"x".repeat(128 * 1024 - 9)}"}` })],
  ["a payload of multibyte letters at the limit", sig(V(1), { payload: `{"pad":"${"ż".repeat(65531)}"}` })],
  ["a payload of multibyte letters over it", sig(V(1), { payload: `{"pad":"${"ż".repeat(65532)}"}` })],
  ["another room", sig(V(2))],
  ["a room that is not one", sig("bad")],
  ["no room", (() => { const s = sig(V(1)); delete s.channelId; return s; })()],
  ["a null room", sig(V(1), { channelId: null })],
  ["a number for a room", sig(V(1), { channelId: 5 })],
  ["a room in braces", sig(V(1), { channelId: `{${V(1)}}` })],
  ["a target that is not in the room", sig(V(1), { toUserId: ids.dan })],
  ["a target that is no id", sig(V(1), { toUserId: "nobody" })],
  ["a number for a target", sig(V(1), { toUserId: 5 })],
];
for (const [label, value] of refusals) await step(`ben sends ${label}`, () => text(ben, "SendVoiceSignal", value), 250);
for (const value of [null, "x", 5, [], [sig(V(1)), 1]]) await step(`SendVoiceSignal(${JSON.stringify(value)?.slice(0, 40)})`, () => text(ben, "SendVoiceSignal", ...(Array.isArray(value) && value.length === 2 ? value : [value])), 200);
await step("SendVoiceSignal with no arguments", () => text(ben, "SendVoiceSignal"));
await step("dan, who sits nowhere, signals the room", () => text(dan, "SendVoiceSignal", sig(V(1))));

// -------------------------------------------------------------------- media
sectionOf("media");
await step("ben opens media in a room he is not in", () => text(ben, "OpenMediaSession", V(2)));
for (const value of [null, "bad", 5, ""]) await step(`ben opens media for ${JSON.stringify(value)}`, () => text(ben, "OpenMediaSession", value), 200);
await step("ben opens media", async () => { const r = await ben.invoke("OpenMediaSession", V(1)); return r; });
const benSession = "session-1";
await step("cat opens media", () => text(cat, "OpenMediaSession", V(1)));
await step("ben opens a second session, which closes the first", () => text(ben, "OpenMediaSession", V(1)));
const benNow = "session-3";
await step("the closed session is of no use", () => text(ben, "SendMediaSignal", media(V(1), benSession, "offer", OFFER)));
await step("an offer, a candidate, the end of candidates and an answer", async () => [
  await text(ben, "SendMediaSignal", media(V(1), benNow, "offer", OFFER)),
  await text(ben, "SendMediaSignal", media(V(1), benNow, "candidate", CANDIDATE)),
  await text(ben, "SendMediaSignal", media(V(1), benNow, "candidate", "null")),
  await text(ben, "SendMediaSignal", media(V(1), benNow, "answer", ANSWER)),
  await text(ben, "SendMediaSignal", media(V(1), benNow, "offer", '{ "type" : "offer", "sdp" : "a+b <c> \\u00e9\\r\\n" }')),
  await text(ben, "SendMediaSignal", media(V(1), benNow, "candidate", ' { "candidate" : "a+b" , "n": 1.50 } ')),
]);
await step("subscriptions", async () => [
  await text(ben, "SendMediaSignal", media(V(1), benNow, "subscriptions", '{"mids":[]}')),
  await text(ben, "SendMediaSignal", media(V(1), benNow, "subscriptions", '{"mids":["0","screen-1","!~","abcdefghijklmnop","A","a"]}')),
  await text(ben, "SendMediaSignal", media(V(1), benNow, "subscriptions", '{"mids":["b","a"]}')),
]);
for (const payload of ['{not json', 'null', '[]', '{}', '{"mids":null}', '{"mids":"0"}', '{"mids":[null]}', '{"mids":[0]}', '{"mids":[""]}', '{"mids":["abcdefghijklmnopq"]}', '{"mids":["a b"]}', '{"mids":["\\u007f"]}', '{"mids":["é"]}', '{"mids":["0","0"]}', '{"mids":["0","\\u0030"]}', '{"mids":[],"mids":["0"]}', `{"mids":[${Array.from({ length: 4097 }, (_, n) => `"${String(n).padStart(16, "0")}"`)}]}`, `{"mids":[${Array.from({ length: 4096 }, (_, n) => `"${String(n).padStart(16, "0")}"`)}]}`]) {
  await step(`subscriptions ${payload.slice(0, 40)}`, () => text(ben, "SendMediaSignal", media(V(1), benNow, "subscriptions", payload)), 150);
}
const envelope = [
  ["version 2", media(V(1), benNow, "offer", OFFER, { protocolVersion: 2 })],
  ["version 0", media(V(1), benNow, "offer", OFFER, { protocolVersion: 0 })],
  ["a kind of no use", media(V(1), benNow, "state", "{}")],
  ["a kind in capitals", media(V(1), benNow, "OFFER", OFFER)],
  ["a server kind", media(V(1), benNow, "tracks", "{}")],
  ["no kind", media(V(1), benNow, null, OFFER)],
  ["no payload", media(V(1), benNow, "offer", null)],
  ["too big a payload", media(V(1), benNow, "offer", `{"type":"offer","sdp":"${"x".repeat(128 * 1024)}"}`)],
  ["a payload of exactly the limit", media(V(1), benNow, "offer", `{"type":"offer","sdp":"${"x".repeat(128 * 1024 - 24)}"}`)],
  ["another room", media(V(2), benNow, "offer", OFFER)],
  ["somebody else's session", media(V(1), "session-2", "offer", OFFER)],
  ["no session", media(V(1), null, "offer", OFFER)],
  ["an unknown session", media(V(1), "session-nobody", "offer", OFFER)],
  ["a room that is no id", media("bad", benNow, "offer", OFFER)],
  ["a number for a session", media(V(1), 5, "offer", OFFER)],
  ["the members in capitals", { PROTOCOLVERSION: 1, CHANNELID: V(1), SESSIONID: benNow, KIND: "offer", PAYLOAD: OFFER }],
  ["not JSON", media(V(1), benNow, "offer", "{not json")],
  ["a null offer", media(V(1), benNow, "offer", "null")],
  ["an offer that is an answer", media(V(1), benNow, "offer", '{"type":"answer","sdp":"x"}')],
  ["an offer without a description", media(V(1), benNow, "offer", '{"type":"offer"}')],
  ["a number for a description", media(V(1), benNow, "offer", '{"type":"offer","sdp":7}')],
  ["an answer without a number", media(V(1), benNow, "answer", '{"type":"answer","sdp":"x"}')],
  ["an answer with a text for the number", media(V(1), benNow, "answer", '{"type":"answer","sdp":"x","negotiationId":"7"}')],
  ["an answer with a fraction", media(V(1), benNow, "answer", '{"type":"answer","sdp":"x","negotiationId":1.5}')],
  ["an answer with a long number", media(V(1), benNow, "answer", '{"type":"answer","sdp":"x","negotiationId":9223372036854775807}')],
  ["an answer with a number that is too long", media(V(1), benNow, "answer", '{"type":"answer","sdp":"x","negotiationId":9223372036854775808}')],
  ["an answer typed as an offer", media(V(1), benNow, "answer", '{"type":"offer","sdp":"x","negotiationId":7}')],
  ["a candidate that is a text", media(V(1), benNow, "candidate", '"candidate:1"')],
  ["a candidate that is a list", media(V(1), benNow, "candidate", "[]")],
  ["a payload nested 65 deep", media(V(1), benNow, "candidate", '{"a":'.repeat(65) + "1" + "}".repeat(65))],
];
for (const [label, value] of envelope) await step(`ben sends ${label}`, () => text(ben, "SendMediaSignal", value), 150);
await step("SendMediaSignal with nothing, or with a text", async () => [await text(ben, "SendMediaSignal"), await text(ben, "SendMediaSignal", null), await text(ben, "SendMediaSignal", "x"), await text(ben, "SendMediaSignal", [])]);
await step("an unseated connection of ben's account", async () => {
  const other = await connect("ben2", await signIn("ben"));
  return text(other, "SendMediaSignal", media(V(1), benNow, "offer", OFFER));
});
await step("the media server answers 409, 422, 413, 500, 401, 404 in turn", async () => {
  const seen = [];
  for (const status of [409, 422, 413, 500, 401]) {
    await admin("answer-signals", `session=${benNow}&status=${status}`);
    seen.push(`${status}: ${await text(ben, "SendMediaSignal", media(V(1), benNow, "offer", OFFER))}`);
  }
  await admin("answer-signals", `session=${benNow}&status=204`);
  seen.push(await text(ben, "SendMediaSignal", media(V(1), benNow, "answer", ANSWER)));
  return seen;
});
await step("a session the media server lost", async () => {
  await admin("drop", `session=${benNow}`);
  const first = await text(ben, "SendMediaSignal", media(V(1), benNow, "candidate", CANDIDATE));
  await sleep(300);
  return [first, await text(ben, "SendMediaSignal", media(V(1), benNow, "candidate", CANDIDATE)), await text(ben, "OpenMediaSession", V(1))];
});
await step("the media server speaks to ben and cat", async () => {
  await admin("signal", `session=session-4&kind=offer&payload=${encodeURIComponent('{"type":"offer","sdp":"v=0 media 1","negotiationId":1,"tracks":[]}')}`);
  await admin("signal", `session=session-2&kind=answer&payload=${encodeURIComponent('{"type":"answer","sdp":"v=0 media"}')}`);
  await admin("signal", `session=session-4&kind=tracks&payload=${encodeURIComponent('{ "tracks" : [ {"mid":"4","kind":"audio","userId":"x","streamId":"","heldBack":false} ] }')}`);
  await admin("signal", `session=session-4&kind=candidate&payload=${encodeURIComponent('{"candidate":"x"}')}`);
  await admin("signal", `session=session-4&kind=offer&payload=${encodeURIComponent("[]")}`);
  await admin("signal", `session=session-nobody&kind=offer&payload=${encodeURIComponent("{}")}`);
  await admin("signal", `session=session-4&kind=offer&payload=${encodeURIComponent('{"type":"offer","sdp":"v=0 media 2","negotiationId":2}')}`);
});
await step("the media server closes sessions", async () => {
  await admin("closed", "session=session-2&reason=transport-timeout");
  await admin("closed", "session=session-4&reason=error");
  await admin("closed", "session=session-4&reason=error");
});
await step("cat opens media again, ben too", async () => [await text(cat, "OpenMediaSession", V(1)), await text(ben, "OpenMediaSession", V(1))]);
await step("the media server restarts", async () => {
  await admin("restart", "epoch=epoch-2");
});
await step("ben and cat open sessions in the new epoch", async () => [await text(ben, "OpenMediaSession", V(1)), await text(cat, "OpenMediaSession", V(1))]);
await step("the media server loses its window of events", async () => {
  await admin("lose-window");
}, 1500);
await step("and they open again", async () => [await text(ben, "OpenMediaSession", V(1)), await text(cat, "OpenMediaSession", V(1))]);
await step("changes of the seat close the session without an event", async () => [
  await text(ben, "JoinVoice", V(1)),
  await text(ben, "JoinVoice", V(2)),
  await text(cat, "LeaveVoice"),
]);
await step("the media server is told in order", async () => [await text(ben, "OpenMediaSession", V(2))]);

// --------------------------------------------------------------- throttling
sectionOf("throttles");
await step("ben joins the first room", () => text(ben, "JoinVoice", V(1)));
await step("a burst of voice signals", async () => {
  const done = [];
  for (let n = 0; n < 230; n++) done.push(ben.send("SendVoiceSignal", sig(V(1), { payload: '{"screenStreamId":null}' })));
  const results = await Promise.all(done);
  const refused = results.filter((r) => r.error).map((r) => r.error.replace(/^.*HubException: /, ""));
  return { accepted: results.length - refused.length >= 200 && results.length - refused.length <= 230, refused: [...new Set(refused)], firstRefusedAtLeast200: results.slice(0, 200).every((r) => !r.error) };
}, 1500, true);
await step("a burst of opens", async () => {
  const results = await Promise.all(Array.from({ length: 9 }, () => ben.send("OpenMediaSession", V(1))));
  const refused = results.filter((r) => r.error).map((r) => r.error.replace(/^.*HubException: /, ""));
  return { opened: results.length - refused.length >= 6 && results.length - refused.length <= 8, refused: [...new Set(refused)] };
}, 1500, true);
await step("a burst of media signals", async () => {
  const session = (await ben.invoke("OpenMediaSession", V(1)))?.result?.sessionId;
  return { session: session ?? "refused (the opens are counted)" };
});
const throttled = await connect("ben3", await signIn("ben"));
await step("a connection of its own for the media signals", async () => {
  await text(throttled, "JoinVoice", V(1));
  const session = (await throttled.invoke("OpenMediaSession", V(1)))?.result?.sessionId;
  const results = await Promise.all(Array.from({ length: 70 }, () => throttled.send("SendMediaSignal", media(V(1), session, "subscriptions", '{"mids":[]}'))));
  const refused = results.filter((r) => r.error).map((r) => r.error.replace(/^.*HubException: /, ""));
  return { accepted: results.length - refused.length >= 60 && results.length - refused.length <= 70, refused: [...new Set(refused)] };
}, 1500, true);

// ------------------------------------------------------------ moderation
sectionOf("moderation");
await step("cat and ben, then fay and eve sit in rooms", async () => [
  await text(cat, "JoinVoice", V(1)),
  await text(fay, "JoinVoice", V(1)),
  await text(eve, "JoinVoice", V(1)),
  await text(olaTwo, "JoinVoice", V(1)),
]);
const room = (n, who) => `/api/admin/channels/${V(n)}/members/${ids[who]}`;
const owner = people.ola;
await step("rank: ben (mod) kicks fay (base)", async () => http(people.ben, "DELETE", room(1, "fay")));
await step("rank: ben kicks cat (senior)", async () => http(people.ben, "DELETE", room(1, "cat")));
await step("rank: ben kicks the owner", async () => http(people.ben, "DELETE", room(1, "ola")));
await step("rank: ben kicks himself", async () => http(people.ben, "DELETE", room(1, "ben")));
await step("rank: cat (senior) kicks ben", async () => http(people.cat, "DELETE", room(1, "ben")));
await step("rank: a base member kicks somebody", async () => http(people.eve, "DELETE", room(1, "fay")));
await step("rank: the owner moves cat to the second room", async () => http(owner, "PATCH", room(1, "cat"), { targetChannelId: V(2) }));
await step("rank: ben moves eve to the second room (eve may not connect there)", async () => http(people.ben, "PATCH", room(1, "eve"), { targetChannelId: V(2) }));
await step("rank: ben moves cat (senior) back", async () => http(people.ben, "PATCH", room(2, "cat"), { targetChannelId: V(1) }));
await step("rank: cat moves ben (not seated)", async () => http(people.cat, "PATCH", room(1, "ben"), { targetChannelId: V(2) }));
await step("a move to the same room", async () => http(owner, "PATCH", room(1, "eve"), { targetChannelId: V(1) }));
await step("a move to a room nobody sees", async () => http(people.ben, "PATCH", room(1, "eve"), { targetChannelId: V(3) }));
await step("a move from a room nobody sees", async () => http(people.ben, "PATCH", room(3, "eve"), { targetChannelId: V(1) }));
await step("a move to a room that is not there", async () => http(owner, "PATCH", room(1, "eve"), { targetChannelId: "11111111-1111-4111-8111-111111111111" }));
await step("a move to a text channel", async () => http(owner, "PATCH", room(1, "eve"), { targetChannelId: ids.t1 }));
await step("a kick in a text channel", async () => http(owner, "DELETE", `/api/admin/channels/${ids.t1}/members/${ids.eve}`));
await step("a kick of somebody who is no member", async () => http(owner, "DELETE", `/api/admin/channels/${V(1)}/members/11111111-1111-4111-8111-111111111111`));
await step("bodies that are not one", async () => [
  await http(owner, "PATCH", room(1, "eve"), {}),
  await http(owner, "PATCH", room(1, "eve"), { targetChannelId: null }),
  await http(owner, "PATCH", room(1, "eve"), { targetChannelId: "x" }),
  await http(owner, "PATCH", room(1, "eve"), "not json"),
  await http(owner, "PATCH", room(1, "eve"), []),
  await http(owner, "PATCH", room(1, "eve")),
  await http(owner, "PATCH", room(1, "eve"), { TargetChannelId: V(1) }),
  await http(owner, "PATCH", room(1, "eve"), { targetChannelId: V(1).toUpperCase() }),
]);
await step("no session, no membership, a server named wrongly", async () => [
  await http(null, "DELETE", room(1, "fay")),
  await http(null, "PATCH", room(1, "fay"), { targetChannelId: V(2) }),
  await http(people.ben, "DELETE", room(1, "fay"), undefined, { "X-Stellarr-Server": "11111111-1111-4111-8111-111111111111" }),
  await http(people.ben, "DELETE", room(1, "fay"), undefined, { "X-Stellarr-Server": "nonsense" }),
  await http(people.dan, "DELETE", room(1, "fay"), undefined, { "X-Stellarr-Server": ids.server3 }),
  await http(people.ben, "GET", room(1, "fay")),
  await http(people.ben, "PUT", room(1, "fay")),
]);
await step("the owner kicks fay", async () => http(owner, "DELETE", room(1, "fay")));
await step("the owner kicks fay again", async () => http(owner, "DELETE", room(1, "fay")));
await step("the owner kicks eve from the room she is not in", async () => http(owner, "DELETE", room(2, "eve")));
await step("the owner moves eve", async () => http(owner, "PATCH", room(1, "eve"), { targetChannelId: V(2) }));
await step("eve confirms the move she may not make by joining", () => text(eve, "JoinVoice", V(2)));
await step("a moderator with local grants only", async () => http(people.cat, "PATCH", room(2, "eve"), { targetChannelId: V(1) }));

// ------------------------------------------------------------- direct calls
sectionOf("calls");
await step("ola calls cat", () => text(ola, "JoinVoice", ids.conv_oc));
await step("a stranger tries to join", () => text(fay, "JoinVoice", ids.conv_oc));
await step("a stranger signals it", () => text(fay, "SendVoiceSignal", sig(ids.conv_oc)));
await step("cat answers", () => text(cat, "JoinVoice", ids.conv_oc));
await step("ola signals cat", () => text(ola, "SendVoiceSignal", sig(ids.conv_oc, { toUserId: ids.cat, payload: '{"muted":true,"encrypted":true}' })));
await step("ola opens media in the call", () => text(ola, "OpenMediaSession", ids.conv_oc));
await step("cat opens media in the call", () => text(cat, "OpenMediaSession", ids.conv_oc));
await step("a third connection of cat's account connects during the call", async () => {
  const device = await connect("cat2", await signIn("cat"));
  return "connected";
});
await step("fay declines a call that is not hers", () => text(fay, "DeclineDirectCall", ids.conv_oc));
await step("DeclineDirectCall with nonsense", async () => [await text(cat, "DeclineDirectCall", "x"), await text(cat, "DeclineDirectCall", null), await text(cat, "DeclineDirectCall", 5), await text(cat, "DeclineDirectCall"), await text(cat, "DeclineDirectCall", V(1))]);
await step("cat declines", () => text(cat, "DeclineDirectCall", ids.conv_oc));
await step("cat declines again", () => text(cat, "DeclineDirectCall", ids.conv_oc));
await step("ola and cat are in a call again", async () => [await text(ola, "JoinVoice", ids.conv_oc), await text(cat, "JoinVoice", ids.conv_oc), await text(ola, "OpenMediaSession", ids.conv_oc)]);
await step("cat blocks ola, which ends the call", async () => http(people.cat, "POST", `/api/social/blocks/${ids.ola}`));
await step("cat blocks ola again", async () => http(people.cat, "POST", `/api/social/blocks/${ids.ola}`));
await step("cat lifts the block", async () => http(people.cat, "DELETE", `/api/social/blocks/${ids.ola}`));
await step("ola calls dan, who blocked her", () => text(ola, "JoinVoice", ids.conv_od));
await step("dan answers a call that was never made", () => text(dan, "JoinVoice", ids.conv_od));
await step("dan declines it all the same", () => text(dan, "DeclineDirectCall", ids.conv_od));
await step("ben calls eve, who takes no messages and is no friend", () => text(ben, "JoinVoice", ids.conv_be));
await step("eve calls ben", () => text(eve, "JoinVoice", ids.conv_be));
await step("ben answers, which he may, since eve is ringing", () => text(ben, "JoinVoice", ids.conv_be));
await step("ben ends it", () => text(ben, "DeclineDirectCall", ids.conv_be));
await step("ben calls fay, a friend of his", () => text(ben, "JoinVoice", ids.conv_bf));
await step("ben leaves the call (not a decline)", () => text(ben, "LeaveVoice"));

// --------------------------------------------------------------- soundboard
sectionOf("soundboard");
await step("ola joins the first room, with an arrival sound chosen", () => text(ola, "JoinVoice", V(1)));
await step("cat joins it", () => text(cat, "JoinVoice", V(1)));
await step("ola plays the sound", () => text(ola, "PlaySoundboard", V(1), ids.sound1));
await step("ola plays it again (a cooldown)", () => text(ola, "PlaySoundboard", V(1), ids.sound1));
await step("cat plays it", () => text(cat, "PlaySoundboard", V(1), ids.sound1));
await step("cat leaves and ola's other tab joins, without a second arrival sound", async () => [await text(cat, "LeaveVoice"), await text(olaTwo, "JoinVoice", V(1))]);
await step("a sound that is not there, or of another server", async () => [
  await text(olaTwo, "PlaySoundboard", V(1), "11111111-1111-4111-8111-111111111111"),
  await text(olaTwo, "PlaySoundboard", V(1), ids.sound3),
  await text(olaTwo, "PlaySoundboard", ids.v6, ids.sound1),
  await text(olaTwo, "PlaySoundboard", ids.t1, ids.sound1),
  await text(olaTwo, "PlaySoundboard", V(2), ids.sound1),
  await text(olaTwo, "PlaySoundboard", "x", ids.sound1),
  await text(olaTwo, "PlaySoundboard", V(1), "x"),
  await text(olaTwo, "PlaySoundboard", null, null),
  await text(olaTwo, "PlaySoundboard", V(1)),
  await text(olaTwo, "PlaySoundboard", V(1), ids.sound1, 1),
  await text(olaTwo, "PlaySoundboard", 5, ids.sound1),
]);
await step("the third server keeps the soundboard to staff", async () => [
  await text(eve, "LeaveVoice"),
  await text(eve, "JoinVoice", ids.v6),
  await text(fay, "JoinVoice", ids.v6),
  await text(eve, "PlaySoundboard", ids.v6, ids.sound3),
  await text(fay, "PlaySoundboard", ids.v6, ids.sound3),
  await text(fay, "PlaySoundboard", ids.v6, ids.sound3),
]);

// ------------------------------------------------------------ disconnecting
sectionOf("disconnecting");
await step("everybody leaves", async () => {
  for (const client of [ola, olaTwo, ben, cat, dan, eve, fay]) await text(client, "LeaveVoice");
});
const tab = await connect("tab1", await signIn("cat"));
const tabTwo = await connect("tab2", await signIn("cat"));
await step("two tabs of cat; the first takes a seat and a session", async () => [await text(tab, "JoinVoice", V(1)), await text(tab, "OpenMediaSession", V(1))]);
await step("the second joins, which replaces the first", () => text(tabTwo, "JoinVoice", V(1)));
await step("the first tab's connection ends without leaving", async () => { tab.close(); });
await step("the second opens a session and ends without leaving", async () => {
  const opened = await text(tabTwo, "OpenMediaSession", V(1));
  tabTwo.close();
  return opened;
}, 1500);
await step("the first server's account of who sits where after all that", async () => {
  const tabThree = await connect("tab3", await signIn("cat"));
  const result = await text(tabThree, "JoinVoice", V(1));
  return result;
});

const { shown } = await fakeRequests();
out("# the places the event poll asked from, whenever");
out([...polled].sort().join(" "));
out("# the sessions the media server holds at the end");
out(scrub(await admin("sessions")));
for (const client of clients) client.close();
writeFileSync(args.out, lines.join("\n") + "\n");
process.exit(0);
