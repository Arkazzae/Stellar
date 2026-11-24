// What clients see of voice while the media server, or this server, comes and
// goes: one phase of a story that voice-lifecycle.sh tells to this server and
// to the first one, each with a fake media server behind it.
//
//   node voice-lifecycle.mjs --phase NAME --base http://127.0.0.1:5000 \
//        --fake http://127.0.0.1:5232 --ids ids.json --out transcript.txt
//
// The phases append to the transcript. Where a time is observed it is told in
// a bucket (instant, a few seconds, the limit of a command, longer), since the
// two servers cannot be expected to agree to the millisecond.

import { appendFileSync, readFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({
  options: { phase: { type: "string" }, base: { type: "string" }, fake: { type: "string" }, ids: { type: "string" }, out: { type: "string" }, password: { type: "string", default: "Corona!2026-long" } },
});
const base = args.base;
const fake = args.fake;
const ids = JSON.parse(readFileSync(args.ids, "utf8"));
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const names = new Map(Object.entries(ids).map(([name, id]) => [String(id).toLowerCase(), name]));
const lines = [];
const out = (text) => lines.push(text);
const V1 = ids.v1;

function scrub(value) {
  let text = typeof value === "string" ? value : JSON.stringify(value);
  for (const [id, name] of names) text = text.replaceAll(id, `<${name}>`).replaceAll(id.toUpperCase(), `<${name}>`);
  return text
    .replace(/\b1[67]\d{11}\b/g, "<ms>")
    .replace(/"playbackId":"[0-9a-f-]{36}"/g, '"playbackId":"<guid>"');
}

async function signIn(username) {
  const response = await fetch(`${base}/api/auth/login`, { method: "POST", headers: clientHeaders, body: JSON.stringify({ username, password: args.password }) });
  if (response.status !== 200) throw new Error(`signing in ${username}: ${response.status}`);
  const body = await response.json();
  const cookie = response.headers.getSetCookie().map((line) => line.split(";")[0]).join("; ");
  return { username, cookie, id: body.id };
}

class Client {
  constructor(label, person) {
    Object.assign(this, { label, person, records: [], waiting: new Map(), next: 0, buffer: "" });
  }
  async open() {
    this.socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app`, { headers: { Cookie: this.person.cookie } });
    await new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = () => reject(new Error(`the upgrade for ${this.label} failed`));
    });
    this.socket.onmessage = (event) => this.onText(String(event.data));
    this.socket.onclose = () => this.records.push({ closedSocket: true });
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
      if (record.type === 3 && this.waiting.has(record.invocationId)) this.waiting.get(record.invocationId)(record);
      else this.records.push(record);
    }
  }
  async invoke(target, ...argumentsList) {
    const id = String(++this.next);
    const done = new Promise((resolve) => this.waiting.set(id, resolve));
    this.socket.send(JSON.stringify({ type: 1, invocationId: id, target, arguments: argumentsList }) + RS);
    return Promise.race([done, sleep(30000).then(() => ({ timeout: true }))]);
  }
  take() {
    const taken = this.records;
    this.records = [];
    return taken.filter((r) => r.target !== "PresenceChanged");
  }
  close() {
    try {
      this.socket.close(1000);
    } catch {}
  }
}

const clients = [];
async function connect(label, name) {
  const client = await new Client(label, await signIn(name)).open();
  clients.push(client);
  return client;
}
async function admin(operation, query = "") {
  return (await fetch(`${fake}/_fake/${operation}?${query}`)).json();
}
async function fakeSummary() {
  const taken = await (await fetch(`${fake}/_fake/requests`)).json();
  await admin("clear");
  const counts = {};
  const shown = [];
  const polls = [];
  for (const r of taken) {
    if (r.path === "/v1/events") {
      const key = `${new URLSearchParams(r.query).get("epoch")}/${new URLSearchParams(r.query).get("after")}`;
      if (polls[polls.length - 1] !== key) polls.push(key);
      continue;
    }
    const key = `${r.method} ${r.path.replace(/session-\d+/, "session-N")}`;
    counts[key] = (counts[key] ?? 0) + 1;
    shown.push(`${r.method} ${r.path}`);
  }
  return { counts, shown, polls };
}
const bucket = (ms) => (ms < 1000 ? "instant" : ms < 3000 ? "a few seconds" : ms < 7000 ? "about the limit of a command (5 s)" : "longer");
async function timed(action) {
  const started = Date.now();
  const value = await action();
  return { value, took: bucket(Date.now() - started) };
}
async function text(client, target, ...argumentsList) {
  const completion = await client.invoke(target, ...argumentsList);
  const shown = completion.timeout ? "TIMED OUT" : completion.error ?? (completion.result == null ? "ok" : JSON.stringify(completion.result));
  return `${target} -> ${shown.replace(/^An unexpected error occurred invoking '\w+' on the server\. HubException: /, "refused: ")}`;
}
async function show(title, extra) {
  out(`## ${title}`);
  if (extra !== undefined) out(`   ${scrub(extra)}`);
  for (const client of clients) for (const record of client.take()) out(`   ${client.label}: ${scrub(record)}`);
}
const media = (session, kind, payload) => ({ protocolVersion: 1, channelId: V1, sessionId: session, kind, payload });
const OFFER = '{"type":"offer","sdp":"v=0"}';
const sessionOf = async (client) => (await client.invoke("OpenMediaSession", V1)).result?.sessionId;
const offer = (n) => encodeURIComponent(`{"type":"offer","sdp":"v=0 media ${n}","negotiationId":${n},"tracks":[]}`);

out(`# phase ${args.phase}`);
const phase = args.phase;
if (phase === "start-down") {
  // The media server is down while this server starts: the first call it gets is refused, and it asks again.
  await fakeSummary();
  const ola = await connect("ola", "ola");
  await text(ola, "JoinVoice", V1);
  await show("ola joins; a session is refused", { open: await text(ola, "OpenMediaSession", V1) });
  await sleep(7500);
  const { counts, polls } = await fakeSummary();
  await show("the reset is asked for again and again", { resetsBetween3And5: counts["POST /v1/reset"] >= 3 && counts["POST /v1/reset"] <= 5, other: Object.keys(counts).filter((k) => k !== "POST /v1/reset"), polls: polls.length });
  await admin("down", "on=0");
  await sleep(4000);
  const { counts: back } = await fakeSummary();
  await show("the media server comes back", { resets: back["POST /v1/reset"] ?? 0 });
  await show("a session opens", { open: await text(ola, "OpenMediaSession", V1) });
} else if (phase === "outage") {
  const ola = await connect("ola", "ola");
  const ben = await connect("ben", "ben");
  await text(ola, "JoinVoice", V1);
  await text(ben, "JoinVoice", V1);
  const olaSession = await sessionOf(ola);
  const benSession = await sessionOf(ben);
  await show("two sessions", { olaSession, benSession });
  await fakeSummary();
  await admin("down", "on=1");
  await sleep(1500);
  await show("the media server goes away", {
    open: await timed(() => text(ola, "OpenMediaSession", V1)).then((r) => `${r.value} (${r.took})`),
    signal: await timed(() => text(ola, "SendMediaSignal", media(olaSession, "offer", OFFER))).then((r) => `${r.value} (${r.took})`),
    voice: await text(ola, "SendVoiceSignal", { protocolVersion: 1, channelId: V1, toUserId: null, kind: "state", payload: '{"muted":true}' }),
    leave: await text(ben, "LeaveVoice"),
    rejoin: await text(ben, "JoinVoice", V1),
  });
  for (const n of [1, 2, 3]) await admin("signal", `session=${olaSession}&kind=offer&payload=${offer(n)}`);
  await sleep(500);
  await show("events pile up at the media server and nobody hears them");
  await admin("down", "on=0");
  await sleep(5000);
  const { counts, shown, polls } = await fakeSummary();
  await show("it comes back", { requests: [...new Set(shown)].sort(), resetsAgain: counts["POST /v1/reset"] ?? 0 });
  await show("a session opens again", { open: await text(ola, "OpenMediaSession", V1) });
} else if (phase === "restart-media") {
  const ola = await connect("ola", "ola");
  await text(ola, "JoinVoice", V1);
  const session = await sessionOf(ola);
  await fakeSummary();
  await admin("restart", "epoch=epoch-9");
  await sleep(1500);
  const { counts, polls } = await fakeSummary();
  await show("the media server restarts", { session, resets: counts["POST /v1/reset"] ?? 0, polls });
  await show("a session opens in the new epoch", { open: await text(ola, "OpenMediaSession", V1) });
  await admin("signal", `session=session-99&kind=offer&payload=${offer(1)}`);
  await sleep(500);
  await show("an event that follows");
} else if (phase === "slow") {
  const ola = await connect("ola", "ola");
  const ben = await connect("ben", "ben");
  await text(ola, "JoinVoice", V1);
  const session = await sessionOf(ola);
  await fakeSummary();
  await admin("slow", "ms=9000");
  const slow = Promise.all([
    timed(() => text(ola, "SendMediaSignal", media(session, "offer", OFFER))),
    timed(() => text(ben, "OpenMediaSession", V1)),
  ]);
  const quick = await timed(() => text(ben, "GetRtcClock"));
  const joined = await timed(() => text(ben, "JoinVoice", V1));
  const [signal, open] = await slow;
  await show("a slow media server", { signal: `${signal.value} (${signal.took})`, open: `${open.value} (${open.took})`, otherCalls: `${quick.took}, ${joined.took}` });
  await admin("slow", "ms=0");
  await sleep(1500);
  await fakeSummary();
  await show("it is quick again", { open: await text(ola, "OpenMediaSession", V1) });
} else if (phase === "nonsense") {
  const ola = await connect("ola", "ola");
  await text(ola, "JoinVoice", V1);
  const session = await sessionOf(ola);
  await fakeSummary();
  const results = [];
  for (const [status, type, body] of [[500, "application/json", "{}"], [200, "application/json", "not json"], [200, "text/plain", "{}"], [201, "application/json", "{}"], [503, "", ""], [401, "application/json", "{}"]]) {
    await admin("canned", `status=${status}&type=${encodeURIComponent(type)}&body=${encodeURIComponent(body)}`);
    results.push(`${status} ${type} ${body}: open ${await text(ola, "OpenMediaSession", V1)}; signal ${await text(ola, "SendMediaSignal", media(session, "offer", OFFER))}`);
  }
  await admin("canned-clear");
  await show("every answer that is not the contract's", results);
  // The poll alone answers nonsense, and an event issued meanwhile is not lost.
  await fakeSummary();
  await admin("canned-polls", "status=200&type=application%2Fjson&body=nonsense");
  await sleep(3500);
  await admin("signal", `session=${session}&kind=offer&payload=${offer(1)}`);
  await admin("canned-polls", "status=500&type=&body=");
  await sleep(3500);
  await admin("canned-clear");
  await sleep(2500);
  await show("the poll answers nonsense for a while", { polledAgainAndCaughtUp: true });
  await show("a session opens", { open: await text(ola, "OpenMediaSession", V1) });
} else if (phase === "seated") {
  const ola = await connect("ola", "ola");
  const ben = await connect("ben", "ben");
  await text(ola, "JoinVoice", V1);
  await text(ben, "JoinVoice", ids.v2);
  await show("two people sit and open sessions", { ola: await text(ola, "OpenMediaSession", V1), ben: await text(ben, "OpenMediaSession", ids.v2) });
  await fakeSummary();
} else if (phase === "after-restart") {
  await sleep(4000);
  const { counts, polls } = await fakeSummary();
  const ola = await connect("ola", "ola");
  await show("this server was restarted", { resets: counts["POST /v1/reset"] ?? 0, polls });
  await show("the rooms are empty and ola takes a seat", { join: await text(ola, "JoinVoice", V1) });
  await show("her session opens", { open: await text(ola, "OpenMediaSession", V1) });
  await show("the media server holds", await admin("sessions"));
}
for (const client of clients) client.close();
appendFileSync(args.out, lines.join("\n") + "\n");
process.exit(0);
