// What a client of the friends routes and the inbox observes, as a transcript,
// from one server. Run it against this server and against the first one on
// copies of one database (see social-oracle.sh) and compare the transcripts.
//
//   node social-oracle.mjs --base http://127.0.0.1:5000 --out transcript.txt
//
// Needs Node 26 (the global WebSocket takes request headers). The accounts
// (ola, ben, cat, dan, eve) and the inbox rows are made by the shell script.
// Ids are replaced by names and by the order they were first seen in.

import { writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({
  options: { base: { type: "string" }, out: { type: "string" }, password: { type: "string", default: "Corona!2026-long" } },
});
const base = args.base;
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const names = new Map();
const seen = new Map();
const lines = [];
const out = (text) => lines.push(text);

function scrub(value) {
  let text = typeof value === "string" ? value : JSON.stringify(value);
  for (const [id, name] of names) text = text.replaceAll(id, `<${name}>`).replaceAll(id.replaceAll("-", ""), `<${name}>`);
  text = text.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, (id) => {
    const key = id.toLowerCase();
    if (!seen.has(key)) seen.set(key, `id${seen.size + 1}`);
    return `<${seen.get(key)}>`;
  });
  text = text.replace(/[0-9a-f]{32}:[0-9a-f]{32}/g, "<pairkey>");
  text = text.replace(/\d{4}-\d\d-\d\dT[\d:.]+(Z?)/g, "<time>$1");
  text = text.replace(/00-[0-9a-f]{32}-[0-9a-f]{16}-00/g, "<trace>");
  return text.replace(/\b1[67]\d{11}\b/g, "<ms>");
}

async function http(who, method, path, body, extra = {}) {
  const response = await fetch(base + path, {
    method,
    headers: { ...clientHeaders, ...(who?.cookie ? { Cookie: who.cookie } : {}), ...extra },
    body: body === undefined ? undefined : typeof body === "string" ? body : JSON.stringify(body),
  });
  const text = await response.text();
  const keep = ["content-type", "cache-control", "pragma", "x-content-type-options", "location", "allow"];
  const headers = keep.filter((h) => response.headers.has(h)).map((h) => `${h}: ${response.headers.get(h)}`).join("; ");
  return { status: response.status, text, headers, json: () => JSON.parse(text) };
}

async function signIn(username) {
  const response = await http(null, "POST", "/api/auth/login", { username, password: args.password });
  if (response.status !== 200) throw new Error(`signing in ${username}: ${response.status}`);
  const body = response.json();
  const id = body.id ?? body.user?.id;
  names.set(id, username);
  const cookie = (await fetch(base + "/api/auth/login", { method: "POST", headers: clientHeaders, body: JSON.stringify({ username, password: args.password }) }));
  const set = cookie.headers.getSetCookie().map((line) => line.split(";")[0]).join("; ");
  return { username, cookie: set, id };
}

class Hub {
  constructor(label, person) {
    this.label = label;
    this.person = person;
    this.records = [];
    this.waiting = new Map();
    this.next = 0;
    this.buffer = "";
  }
  async open() {
    this.socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app`, { headers: { Cookie: this.person.cookie } });
    await new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = () => reject(new Error(`the upgrade for ${this.label} failed`));
    });
    this.socket.onmessage = (event) => this.onText(String(event.data));
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
    return Promise.race([done, sleep(3000).then(() => ({ timeout: true }))]);
  }
  take() {
    const taken = this.records;
    this.records = [];
    return taken;
  }
}

const hubs = [];
async function connect(label, person) {
  const hub = await new Hub(label, person).open();
  hubs.push(hub);
  return hub;
}

async function step(title, action) {
  out(`## ${title}`);
  try {
    const result = await action?.();
    if (result !== undefined) out(`   => ${scrub(result)}`);
  } catch (error) {
    out(`   error: ${error.message}`);
  }
  await sleep(500);
  // Which of the statuses a new connection hears comes first is the hub's
  // business and is compared elsewhere; here they are listed in order of text.
  for (const hub of hubs) {
    const heard = hub.take().map((record) => `   ${hub.label} hears: ${scrub(record)}`);
    if (title.includes("connects")) heard.sort();
    for (const line of heard) out(line);
  }
}

const P = {};
for (const name of ["ola", "ben", "cat", "dan", "eve"]) P[name] = await signIn(name);

const answer = async (who, method, path, body, extra) => {
  const r = await http(P[who], method, path, body, extra);
  return `${who} ${method} ${path} -> ${r.status} [${r.headers}] ${r.text}`;
};
const ask = (who, username) => answer(who, "POST", "/api/social/requests", { username });
const requestOf = async (who) => {
  const state = (await http(P[who], "GET", "/api/social")).json();
  return state.requests[0]?.id ?? "00000000-0000-0000-0000-00000000dead";
};
const accept = async (who) => answer(who, "POST", `/api/social/requests/${await requestOf(who)}/accept`);
const drop = async (who) => answer(who, "DELETE", `/api/social/requests/${await requestOf(who)}`);
const unfriend = (who, other) => answer(who, "DELETE", `/api/social/friends/${P[other].id}`);
const block = (who, other) => answer(who, "POST", `/api/social/blocks/${P[other].id}`);
const unblock = (who, other) => answer(who, "DELETE", `/api/social/blocks/${P[other].id}`);

async function views() {
  const parts = [];
  for (const who of ["ola", "ben", "cat"]) {
    const s = await http(P[who], "GET", "/api/social");
    parts.push(`   ${who} social: ${scrub(s.text)}`);
    const i = await http(P[who], "GET", "/api/notifications");
    parts.push(`   ${who} inbox: ${scrub(i.text)}`);
  }
  return parts.join("\n");
}
const checkpoint = (title) => step(`${title}: what each sees`, async () => `\n${await views()}`);

const hola = await connect("ola", P.ola);
const hben = await connect("ben", P.ben);
const hcat = await connect("cat", P.cat);
await step("everybody connects");
await checkpoint("start");

const presenceRound = (label) => step(`ben changes status (${label})`, async () => {
  await hben.invoke("SetPresence", "dnd");
  await sleep(300);
  await hben.invoke("SetPresence", "online");
});

// Strangers.
await presenceRound("strangers");
await step("ola asks ben", () => ask("ola", "ben"));
await step("ola asks ben again", () => ask("ola", "ben"));
await step("ben asks ola while ola's request waits", () => ask("ben", "ola"));
await checkpoint("request waiting");
await step("ola withdraws", () => drop("ola"));
await step("ola asks ben", () => ask("ola", "@Ben"));
await step("ben declines", () => drop("ben"));
await step("ben asks ola", () => ask("ben", "ola"));
await step("ola accepts", () => accept("ola"));
await step("ola accepts again", () => answer("ola", "POST", `/api/social/requests/${"00000000-0000-0000-0000-00000000dead"}/accept`));
await step("ola asks ben as friends", () => ask("ola", "ben"));
await step("ben asks ola as friends", () => ask("ben", "ola"));
await checkpoint("friends");
await presenceRound("friends");
await step("ola removes ben", () => unfriend("ola", "ben"));
await step("ola removes ben again", () => unfriend("ola", "ben"));
await presenceRound("removed");
await step("ben asks ola after removal", () => ask("ben", "ola"));
await step("ola accepts", () => accept("ola"));
await step("ben removes ola", () => unfriend("ben", "ola"));

// Blocks in every state.
await step("ola asks ben", () => ask("ola", "ben"));
await step("ben accepts", () => accept("ben"));
await step("ola blocks ben while friends", () => block("ola", "ben"));
await step("ola blocks ben again", () => block("ola", "ben"));
await checkpoint("blocked while friends");
await presenceRound("blocked");
await step("ben asks ola", () => ask("ben", "ola"));
await step("ola asks ben", () => ask("ola", "ben"));
await step("ben lifts a block he does not have", () => unblock("ben", "ola"));
await step("ola unblocks ben", () => unblock("ola", "ben"));
await step("ola unblocks ben again", () => unblock("ola", "ben"));
await checkpoint("unblocked");
await step("ola asks ben", () => ask("ola", "ben"));
await step("ben blocks ola while ola's request waits", () => block("ben", "ola"));
await checkpoint("blocked with a request waiting");
await step("ben unblocks ola", () => unblock("ben", "ola"));
await step("ben asks ola", () => ask("ben", "ola"));
await step("ola blocks ben while ben's request waits", () => block("ola", "ben"));
await step("ola unblocks ben", () => unblock("ola", "ben"));
await step("ola blocks cat, a stranger", () => block("ola", "cat"));
await step("cat asks ola", () => ask("cat", "ola"));
await step("ola unblocks cat", () => unblock("ola", "cat"));
await step("cat asks ola", () => ask("cat", "ola"));
await step("ola declines", () => drop("ola"));
await step("ola asks ben", () => ask("ola", "ben"));
await checkpoint("end of the story");

// Refusals.
await step("ola asks herself", () => ask("ola", "ola"));
await step("ola asks nobody", () => ask("ola", "nobody-here"));
await step("ola asks dan, who takes no requests", () => ask("ola", "dan"));
await step("dan asks ola", () => ask("dan", "ola"));
await step("dan asks himself", () => ask("dan", "dan"));
await step("ola asks eve, who blocked her in the data", () => ask("ola", "eve"));
await step("eve asks ola", () => ask("eve", "ola"));
await step("ola asks with an empty name", () => ask("ola", ""));
await step("ola asks with an at sign", () => ask("ola", "@"));
await step("ola asks with a null name", () => answer("ola", "POST", "/api/social/requests", { username: null }));
await step("ola asks with no name", () => answer("ola", "POST", "/api/social/requests", {}));
await step("ola asks with a number", () => answer("ola", "POST", "/api/social/requests", { username: 5 }));
await step("ola asks with another case of the key", () => answer("ola", "POST", "/api/social/requests", '{"USERNAME":"cat"}'));
await step("ola asks as text", () => answer("ola", "POST", "/api/social/requests", '{"username":"cat"}', { "Content-Type": "text/plain" }));
await step("ola asks a very long name", () => ask("ola", "x".repeat(5000)));
await step("ola asks cat with spaces and signs", () => ask("ola", "  @@Cat  "));
await step("ola blocks herself", () => block("ola", "ola"));
await step("ola blocks nobody", () => answer("ola", "POST", `/api/social/blocks/${"11111111-1111-1111-1111-111111111111"}`));
await step("ola blocks a name that is no id", () => answer("ola", "POST", "/api/social/blocks/not-an-id"));
await step("ola blocks by an id in braces", () => answer("ola", "POST", `/api/social/blocks/{${P.cat.id}}`));
await step("ola unblocks by an id without hyphens", () => answer("ola", "DELETE", `/api/social/blocks/${P.cat.id.replaceAll("-", "")}`));
await step("ola accepts a request of nobody", () => answer("ola", "POST", `/api/social/requests/${"11111111-1111-1111-1111-111111111111"}/accept`));
await step("ola removes a friend she has not", () => unfriend("ola", "eve"));
await step("anonymous reads the state", () => http(null, "GET", "/api/social").then((r) => `${r.status} ${r.text}`));
await step("anonymous asks", () => http(null, "POST", "/api/social/requests", { username: "ola" }).then((r) => `${r.status} ${r.text}`));
await step("anonymous reads the inbox", () => http(null, "GET", "/api/notifications").then((r) => `${r.status} ${r.text}`));
for (const method of ["HEAD", "PUT", "PATCH"]) await step(`${method} /api/social`, () => answer("ola", method, "/api/social"));
await step("GET /api/social/requests", () => answer("ola", "GET", "/api/social/requests"));
await step("trailing slash", () => answer("ola", "GET", "/api/social/"));

// The inbox, on rows the shell script arranged for ben.
const inbox = (who, query = "", extra) => answer(who, "GET", `/api/notifications${query}`, undefined, extra);
await step("ben's inbox, first page", () => inbox("ben"));
await step("ben pages of 7", async () => {
  const seenPages = [];
  let before = "";
  for (let n = 0; n < 40; n++) {
    const r = await http(P.ben, "GET", `/api/notifications?limit=7${before}`);
    const body = r.json();
    seenPages.push(`${body.items.map((i) => i.title).join("|")} next=${body.nextBeforeSequence === null ? "null" : "set"} unread=${body.unreadCount} last=${body.lastSequence}`);
    if (body.nextBeforeSequence === null) break;
    before = `&beforeSequence=${body.nextBeforeSequence}`;
  }
  return seenPages.join("\n      ");
});
for (const query of ["?limit=100", "?limit=101", "?limit=0", "?limit=-1", "?limit=abc", "?limit=", "?Limit=3", "?limit=+3", "?limit=%203", "?limit=3&limit=4", "?limit=1e1", "?limit=3.0", "?beforeSequence=0", "?beforeSequence=-1", "?beforeSequence=5", "?beforeSequence=abc", "?beforeSequence=9999999999999999999999", "?limit=99999999999", "?serverId=zzz", "?serverId=11111111-1111-1111-1111-111111111111", "?serverId=zzz&limit=abc", "?serverId=11111111-1111-1111-1111-111111111111&limit=0", "?limit=abc&serverId=11111111-1111-1111-1111-111111111111"]) {
  await step(`ben's inbox ${query}`, () => inbox("ben", query));
}
await step("ben's inbox for the other server by header", () => inbox("ben", "", { "X-Stellarr-Server": "11111111-1111-1111-1111-111111111111" }));
await step("ben's inbox with an empty header", () => inbox("ben", "", { "X-Stellarr-Server": "" }));
await step("ben's inbox with a header that is no id", () => inbox("ben", "", { "X-Stellarr-Server": "x" }));
const mine = async (title) => (await http(P.ben, "GET", "/api/notifications?limit=100")).json().items.find((i) => i.title === title);
await step("ben reads a channel entry", async () => answer("ben", "PUT", `/api/notifications/${(await mine("channel-1")).id}/read`));
await step("ben reads it again", async () => answer("ben", "PUT", `/api/notifications/${(await mine("channel-1")).id}/read`));
await step("ben reads an invitation", async () => answer("ben", "PUT", `/api/notifications/${(await mine("usable-invite")).id}/read`));
await step("ben deletes the expired invitation", () => answer("ben", "DELETE", "/api/notifications/B0000000-0000-0000-0000-0000000000E1"));
await step("ben deletes it again", () => answer("ben", "DELETE", "/api/notifications/B0000000-0000-0000-0000-0000000000E1"));
await step("ben reads the expired used-up invitation", () => answer("ben", "PUT", "/api/notifications/B0000000-0000-0000-0000-0000000000E2/read"));
await step("ben reads an entry of another server that is no invitation", () => answer("ben", "PUT", "/api/notifications/B0000000-0000-0000-0000-0000000000E3/read"));
await step("ben reads ola's entry", () => answer("ben", "PUT", "/api/notifications/A0000000-0000-0000-0000-000000000001/read"));
await step("ben deletes ola's entry", () => answer("ben", "DELETE", "/api/notifications/A0000000-0000-0000-0000-000000000001"));
await step("ben reads an id of nothing", () => answer("ben", "PUT", `/api/notifications/${"11111111-1111-1111-1111-111111111111"}/read`));
await step("ben reads by an id in braces", () => answer("ben", "PUT", "/api/notifications/{B0000000-0000-0000-0000-0000000000E2}/read"));
await step("ben reads a name that is no id", () => answer("ben", "PUT", "/api/notifications/nope/read"));
await step("ben posts to read", () => answer("ben", "POST", "/api/notifications/B0000000-0000-0000-0000-0000000000E2/read"));
await step("ben reads all", () => answer("ben", "POST", "/api/notifications/read-all"));
await step("ben reads all again", () => answer("ben", "POST", "/api/notifications/read-all"));
await step("ben's inbox after", () => inbox("ben", "?limit=100"));
await step("ola's inbox", () => inbox("ola"));
await step("ola reads all, with nothing to read", () => answer("ola", "POST", "/api/notifications/read-all"));

writeFileSync(args.out, lines.join("\n") + "\n");
for (const hub of hubs) hub.socket.close(1000);
process.exit(0);
