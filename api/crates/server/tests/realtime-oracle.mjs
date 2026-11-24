// What a client of the hub observes, as a transcript, from one server. Run it
// against this server and against the first one, on copies of one database
// (see realtime-oracle.sh), and compare the two transcripts.
//
//   node realtime-oracle.mjs --base http://127.0.0.1:5000 --out transcript.txt
//
// Needs Node 26 (the global WebSocket takes request headers). The accounts it
// signs in as (ola, ben, cat, dan, eve and fay) are made by the shell script.
// Ids are replaced by names, so the two servers' transcripts can be compared.

import { writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({
  options: { base: { type: "string" }, out: { type: "string" }, server1: { type: "string" }, server3: { type: "string" }, password: { type: "string", default: "Corona!2026-long" } },
});
const base = args.base;
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const names = new Map();
const lines = [];
const out = (text) => lines.push(text);

function scrub(value) {
  let text = JSON.stringify(value);
  for (const [id, name] of names) text = text.replaceAll(id, `<${name}>`);
  return text.replace(/\b1[67]\d{11}\b/g, "<ms>");
}

async function signIn(username) {
  const response = await fetch(`${base}/api/auth/login`, {
    method: "POST",
    headers: clientHeaders,
    body: JSON.stringify({ username, password: args.password }),
  });
  if (response.status !== 200) throw new Error(`signing in ${username}: ${response.status}`);
  const body = await response.json();
  names.set(body.id ?? body.user?.id, username);
  const cookie = response.headers.getSetCookie().map((line) => line.split(";")[0]).join("; ");
  return { username, cookie, id: body.id ?? body.user?.id };
}

class Client {
  constructor(label, person, query = "") {
    this.label = label;
    this.person = person;
    this.query = query;
    this.records = [];
    this.waiting = new Map();
    this.next = 0;
    this.closeCode = null;
    this.buffer = "";
  }

  async open() {
    const url = `${base.replace("http", "ws")}/hubs/app${this.query}`;
    this.socket = new WebSocket(url, { headers: { Cookie: this.person.cookie } });
    await new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = () => reject(new Error(`the upgrade for ${this.label} failed`));
    });
    this.socket.onmessage = (event) => this.onText(String(event.data));
    this.socket.onclose = (event) => {
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

  async invoke(target, ...argumentsList) {
    const id = String(++this.next);
    const done = new Promise((resolve) => this.waiting.set(id, resolve));
    this.socket.send(JSON.stringify({ type: 1, invocationId: id, target, arguments: argumentsList }) + RS);
    const completion = await Promise.race([done, sleep(3000).then(() => ({ timeout: true }))]);
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

async function step(title, action) {
  out(`## ${title}`);
  try {
    const result = await action?.();
    if (result !== undefined) out(`   result: ${scrub(result)}`);
  } catch (error) {
    out(`   error: ${error.message}`);
  }
  await sleep(600);
  for (const client of clients) {
    const seen = client.take();
    for (const record of seen) out(`   ${client.label}: ${scrub(record)}`);
  }
}

async function call(client, target, ...argumentsList) {
  const completion = await client.invoke(target, ...argumentsList);
  return { call: `${target}(${argumentsList.map((a) => JSON.stringify(a)).join(", ")})`, completion };
}

const people = {};
for (const name of ["ola", "ben", "cat", "dan", "eve", "fay"]) people[name] = await signIn(name);
const second = await signIn("ben");
names.set(second.id, "ben");

// Whom they know: ola and ben share the server and are friends; ola and cat
// only talk, and cat follows no server; dan blocked ola; eve hides her status
// and shares a third server with ola; fay is on the server and follows what
// she is asked to.
const ola = await connect("ola", people.ola);
await step("ola connects and follows the first server she belongs to");
const ben1 = await connect("ben1", people.ben);
await step("ben connects: a friend who shares the server");
const ben2 = await connect("ben2", second);
await step("ben opens a second tab");
await step("ben chooses idle", () => call(ben1, "SetPresence", "idle"));
await step("ben chooses idle again", () => call(ben1, "SetPresence", "idle"));
await step("ben chooses online", () => call(ben2, "SetPresence", "online"));
for (const value of [null, 5, true, "Idle", "invisible", "", "offline", "online"]) {
  await step(`ben calls SetPresence(${JSON.stringify(value)})`, () => call(ben1, "SetPresence", value));
}
await step("ben calls SetPresence with no arguments", () => call(ben1, "SetPresence"));
await step("ben calls SetPresence with two arguments", () => call(ben1, "SetPresence", "idle", "dnd"));
await step("ben calls with another case of the name", () => call(ben1, "setpresence", "dnd"));
await step("ben calls GetRtcClock", () => call(ben1, "GetRtcClock"));
await step("ben calls GetRtcClock with an argument", () => call(ben1, "GetRtcClock", 1));
await step("ben calls a method that does not exist", () => call(ben1, "NoSuchMethod", 1));
await step("ben calls LeaveVoice", () => call(ben1, "LeaveVoice"));
await step("ben calls JoinChannel with nonsense", () => call(ben1, "JoinChannel", "not-a-guid"));
const cat = await connect("cat", people.cat);
await step("cat connects: a conversation, no server");
const dan = await connect("dan", people.dan);
await step("dan connects: a friend of ola who blocked her");
const eve = await connect("eve", people.eve);
await step("eve connects with her status hidden");
await step("eve chooses dnd", () => call(eve, "SetPresence", "dnd"));
await step("eve chooses dnd again", () => call(eve, "SetPresence", "dnd"));
// Which server a connection follows shows in whose status it hears: ben is
// on the first server only, so fay hears him when she follows that one.
const first = args.server1;
const third = args.server3;
const variants = [
  ["none", ""],
  ["the first server", `?serverId=${first}`],
  ["the third server", `?serverId=${third}`],
  ["the third server in capitals", `?serverId=${third.toUpperCase()}`],
  ["the third server without hyphens", `?serverId=${third.replaceAll("-", "")}`],
  ["the third server in braces", `?serverId=%7B${third}%7D`],
  ["the third server in parentheses", `?serverId=(${third})`],
  ["the third server with spaces around", `?serverId=%20${third}%20`],
  ["the third server with a plus for a space", `?serverId=+${third}`],
  ["the name in another case", `?SERVERID=${third}`],
  ["a text that is no id", "?serverId=garbage"],
  ["an id of no server", "?serverId=11111111-1111-1111-1111-111111111111"],
  ["an empty value", "?serverId="],
  ["the third server twice, then the first", `?serverId=${third}&serverId=${first}`],
  ["something else first", `?other=1&serverId=${third}`],
];
for (const [label, query] of variants) {
  const watcher = await connect("fay", people.fay, query);
  await step(`fay connects asking for: ${label}`);
  await step(`ben toggles idle and online; fay (${label})`, async () => {
    await call(ben1, "SetPresence", "idle");
    await call(ben1, "SetPresence", "online");
  });
  watcher.close();
  clients.splice(clients.indexOf(watcher), 1);
  await sleep(200);
}
await step("ben chooses dnd, heard by ola and cat", () => call(ben1, "SetPresence", "dnd"));
await step("ben signs out of one session", async () => {
  const response = await fetch(`${base}/api/auth/logout`, { method: "POST", headers: { ...clientHeaders, Cookie: second.cookie } });
  return { status: response.status };
});
await step("ben's other session is still there", () => call(ben1, "GetRtcClock"));
await step("ben signs out of the other", async () => {
  const response = await fetch(`${base}/api/auth/logout`, { method: "POST", headers: { ...clientHeaders, Cookie: people.ben.cookie } });
  return { status: response.status };
});
await step("ola hangs up", () => ola.close());
await step("cat hangs up", () => cat.close());

for (const client of clients) client.close();
writeFileSync(args.out, lines.join("\n") + "\n");
process.exit(0);
