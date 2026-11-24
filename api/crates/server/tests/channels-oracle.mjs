// What clients see of the channels and sections routes and the two hub calls
// about a text channel, as a transcript: the answers, the events each connected
// person was sent, and the rows after every step. Run it against this server
// and against the first one, on copies of one database (see
// channels-shared-database.sh), and compare the two transcripts.
//
//   node channels-oracle.mjs --base http://127.0.0.1:5000 --server <id> \
//     --visible visible.json (--discover | --out transcript.txt \
//       (--container <docker container> | --database <file>))
//
// Needs Node 26 (the global WebSocket takes request headers, and node:sqlite
// is built in). The accounts it signs in as (ola, ben, cat, dan and eve) and
// the server they are on are made by the shell script. With --discover it
// only writes what each of them sees, as the first server's bootstrap lists it.

import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({
  options: {
    base: { type: "string" },
    server: { type: "string" },
    password: { type: "string" },
    visible: { type: "string" },
    discover: { type: "boolean", default: false },
    out: { type: "string" },
    container: { type: "string" },
    database: { type: "string" },
  },
});
const base = args.base;
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const names = new Map();
const lines = [];
const out = (text) => lines.push(text);

function scrub(value) {
  let text = typeof value === "string" ? value : JSON.stringify(value);
  for (const [id, name] of names) text = text.replaceAll(id, `<${name}>`);
  return text
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, "<id>")
    .replace(/\d{4}-\d\d-\d\d[T ][\d:.]+Z?/g, "<time>")
    .replace(/00-[0-9a-f]{32}-[0-9a-f]{16}-00/g, "<trace>");
}

async function signIn(username) {
  const response = await fetch(`${base}/api/auth/login`, {
    method: "POST",
    headers: clientHeaders,
    body: JSON.stringify({ username, password: args.password }),
  });
  if (response.status !== 200) throw new Error(`signing in ${username}: ${response.status}`);
  const body = await response.json();
  const id = body.id ?? body.user?.id;
  names.set(id, username);
  const cookie = response.headers.getSetCookie().map((line) => line.split(";")[0]).join("; ");
  return { username, cookie, id };
}

async function api(person, method, path, body, raw) {
  const response = await fetch(`${base}${path}`, {
    method,
    headers: { ...clientHeaders, Cookie: person.cookie },
    body: raw ?? (body === undefined ? undefined : JSON.stringify(body)),
  });
  const text = await response.text();
  let parsed = text;
  try {
    parsed = JSON.parse(text);
  } catch {}
  return { response, text, body: parsed };
}

const people = {};
for (const name of ["ola", "ben", "cat", "dan", "eve"]) people[name] = await signIn(name);

if (args.discover) {
  const seen = {};
  for (const [name, person] of Object.entries(people)) {
    const { body } = await api(person, "GET", "/api/bootstrap");
    seen[name] = {
      channels: body.channels.map((x) => ({ id: x.id, name: x.name, kind: x.kind, sectionId: x.sectionId })),
      sections: body.sections.map((x) => ({ id: x.id, name: x.name })),
    };
  }
  writeFileSync(args.visible, JSON.stringify(seen, null, 2));
  process.exit(0);
}

const visible = JSON.parse(readFileSync(args.visible, "utf8"));
const channelIds = (who, kind) => visible[who].channels.filter((x) => !kind || x.kind === kind).map((x) => x.id);
const sectionIds = (who) => visible[who].sections.map((x) => x.id);
const everyChannel = visible.ola.channels;
const byName = (name) => everyChannel.find((x) => x.name === name).id;
const sectionByName = (name) => visible.ola.sections.find((x) => x.name === name).id;
for (const channel of everyChannel) names.set(channel.id, `#${channel.name}`);
for (const section of visible.ola.sections) names.set(section.id, `section ${section.name}`);
names.set(args.server, "server");

// The rows, read from the database of whichever server this is.
const scratch = mkdtempSync(join(tmpdir(), "channels-oracle-"));
function openDatabase() {
  if (args.container) {
    execFileSync("docker", ["cp", `${args.container}:/tmp/data/.`, scratch], { stdio: "ignore" });
    return new DatabaseSync(join(scratch, "stellarr.db"), { readOnly: true });
  }
  return new DatabaseSync(args.database, { readOnly: true });
}
let auditSeen = 0;
// What the first server marks read when somebody lists their channels is not
// what this transcript is about: only markers that change after the start are.
let baseline = null;
function dump() {
  const db = openDatabase();
  try {
    const upper = (id) => (id ? id.toLowerCase() : id);
    const channelName = new Map(db.prepare('SELECT "Id", "Name" FROM "Channels"').all().map((x) => [x.Id, x.Name]));
    const sectionName = new Map(db.prepare('SELECT "Id", "Name" FROM "Sections"').all().map((x) => [x.Id, x.Name]));
    const userName = new Map(db.prepare('SELECT "Id", "Username" FROM "Users"').all().map((x) => [x.Id, x.Username]));
    out("   channels:");
    for (const x of db.prepare('SELECT * FROM "Channels" ORDER BY "Position", "Id"').all()) {
      out(`     ${x.Position} ${x.Name} kind=${x.Kind} section=${sectionName.get(x.SectionId) ?? "-"} topic=${JSON.stringify(x.Topic)} slow=${x.SlowModeSeconds} hide=${x.HideAfterInactivityHours} limit=${x.UserLimit}`);
    }
    out("   sections:");
    for (const x of db.prepare('SELECT * FROM "Sections" ORDER BY "Position", "Id"').all()) out(`     ${x.Position} ${x.Name}`);
    const reads = db.prepare('SELECT * FROM "ChannelReads" ORDER BY "UserId", "ChannelId"').all();
    baseline ??= new Set(reads.map((x) => `${x.UserId}${x.ChannelId}${x.LastReadAt}`));
    const changed = reads.filter((x) => !baseline.has(`${x.UserId}${x.ChannelId}${x.LastReadAt}`));
    if (changed.length) out(`   reads: ${changed.map((x) => `${userName.get(x.UserId)}:${channelName.get(x.ChannelId)}`).join(" ")}`);
    const audit = db.prepare('SELECT * FROM "ServerAuditEntries" ORDER BY "CreatedAt", "Id"').all();
    // The entries of a step are in the order they were written, which the
    // clock decides; within one step only their content is compared.
    const fresh = audit.slice(auditSeen);
    auditSeen = audit.length;
    for (const x of fresh) {
      out(`   audit: ${x.Action} ${x.TargetType} ${JSON.stringify(x.TargetName)} ${JSON.stringify(x.Summary)} ${scrub(x.ChangesJson ?? "")}`);
    }
    void upper;
  } finally {
    db.close();
  }
}

class Client {
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
      // Who is online is not what this transcript is about.
      if (record.type === 1 && record.target === "PresenceChanged") continue;
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

const clients = {};
for (const [name, person] of Object.entries(people)) clients[name] = await new Client(name, person).open();
await sleep(800);
for (const client of Object.values(clients)) client.take();

let created = 0;
const made = {};
async function step(title, action) {
  out(`## ${title}`);
  try {
    const result = await action?.();
    if (result !== undefined) out(`   ${scrub(result)}`);
  } catch (error) {
    out(`   error: ${error.message}`);
  }
  await sleep(500);
  for (const client of Object.values(clients)) {
    for (const record of client.take()) {
      out(`   event to ${client.label}: ${scrub(record.target ? `${record.target} ${JSON.stringify(record.arguments)}` : record)}`);
    }
  }
  dump();
}

// One request, as what a client could tell: status, the headers it reads, and
// the body. Whatever a request made is given a name for the rest of the run.
async function ask(who, method, path, body, raw) {
  const { response, text, body: parsed } = await api(people[who], method, path, body, raw);
  if (response.status === 201 && parsed?.id) {
    names.set(parsed.id, `new ${++created}`);
    made[parsed.name] = parsed.id;
  }
  const kept = ["content-type", "location"].map((name) => [name, response.headers.get(name)]).filter(([, value]) => value);
  return `${who} ${method} ${scrub(path)} -> ${response.status} ${scrub(JSON.stringify(Object.fromEntries(kept)))} ${scrub(typeof parsed === "string" ? parsed : JSON.stringify(parsed))}`;
}

async function hub(who, target, ...argumentsList) {
  const completion = await clients[who].invoke(target, ...argumentsList);
  return `${who} ${target}(${scrub(argumentsList.map((a) => JSON.stringify(a)).join(", "))}) -> ${scrub(completion)}`;
}

// The same shuffle on both servers.
function shuffled(list, seed) {
  let state = seed >>> 0;
  const next = () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const copy = [...list];
  for (let i = copy.length - 1; i > 0; i--) {
    const j = Math.floor(next() * (i + 1));
    [copy[i], copy[j]] = [copy[j], copy[i]];
  }
  return copy;
}

const A = "/api/admin/channels";
const S = "/api/admin/sections";
const order = (who, ids, move) => ask(who, "PUT", `${A}/order`, move === undefined ? { channelIds: ids } : { channelIds: ids, move });

out(`# what each person sees: ${JSON.stringify(Object.fromEntries(Object.keys(visible).map((who) => [who, visible[who].channels.map((x) => x.name)])))}`);
await step("start", () => undefined);

// Orders.
await step("cat may not reorder", () => order("cat", channelIds("cat").reverse()));
await step("dan may not reorder", () => order("dan", channelIds("dan").reverse()));
await step("an order that leaves a channel out", () => order("ben", channelIds("ben").slice(1)));
await step("an order that names a channel ben does not see", () => order("ben", [...channelIds("ben"), byName("dev")]));
await step("an order with a channel twice", () => order("ben", [...channelIds("ben"), channelIds("ben")[0]]));
await step("an empty order", () => order("ben", []));
await step("ben reverses what he sees", () => order("ben", [...channelIds("ben")].reverse()));
await step("eve shuffles what she sees", () => order("eve", shuffled(channelIds("eve"), 1)));
await step("ola shuffles everything", () => order("ola", shuffled(channelIds("ola"), 2)));
for (let round = 3; round < 11; round++) {
  const who = ["ben", "eve", "ola"][round % 3];
  await step(`${who} shuffles what they see, round ${round}`, () => order(who, shuffled(channelIds(who), round)));
}

// Moves.
const benSees = channelIds("ben");
await step("ben files a channel under Alpha, which he sees", () =>
  order("ben", benSees, { channelId: byName("news"), sectionId: sectionByName("Alpha") }));
await step("ben files one under Bravo, which he does not see", () =>
  order("ben", benSees, { channelId: byName("general"), sectionId: sectionByName("Bravo") }));
await step("ben moves a channel he does not see", () =>
  order("ben", benSees, { channelId: byName("dev"), sectionId: sectionByName("Alpha") }));
await step("a move to a section that is not there", () =>
  order("ola", channelIds("ola"), { channelId: byName("news"), sectionId: "11111111-1111-1111-1111-111111111111" }));
await step("a move of a channel that is not there", () =>
  order("ola", channelIds("ola"), { channelId: "11111111-1111-1111-1111-111111111111", sectionId: null }));
await step("a move without a channel", () => order("ola", channelIds("ola"), { sectionId: null }));
await step("ola files general under Bravo, which hides it from most", () =>
  order("ola", channelIds("ola"), { channelId: byName("general"), sectionId: sectionByName("Bravo") }));
await step("ola takes secret out of Bravo, which shows it to most", () =>
  order("ola", channelIds("ola"), { channelId: byName("secret"), sectionId: null }));
await step("ola moves news back out", () =>
  order("ola", channelIds("ola"), { channelId: byName("news"), sectionId: null }));
await step("a move that changes nothing", () =>
  order("ola", channelIds("ola"), { channelId: byName("news"), sectionId: null }));

// Creating.
await step("cat may not create", () => ask("cat", "POST", A, { name: "cats", kind: "text" }));
await step("dan may not create outside his section", () => ask("dan", "POST", A, { name: "dans", kind: "text" }));
await step("dan creates in Bravo", () => ask("dan", "POST", A, { name: "Dans-Room", kind: "text", sectionId: sectionByName("Bravo") }));
await step("a name a hidden channel has", () => ask("ben", "POST", A, { name: "DEV", kind: "text" }));
await step("a name taken, asked by somebody who may not create", () => ask("cat", "POST", A, { name: "dev", kind: "text" }));
await step("ben files a channel under a section he does not see", () =>
  ask("ben", "POST", A, { name: "benzo", kind: "voice", sectionId: sectionByName("Bravo") }));
await step("a section that is not there", () =>
  ask("ola", "POST", A, { name: "nowhere", kind: "voice", sectionId: "11111111-1111-1111-1111-111111111111" }));
await step("ola creates a voice channel", () => ask("ola", "POST", A, { name: "  Olas  ", kind: "voice" }));
for (const [name, kind] of [["a b", "text"], ["", "text"], ["x".repeat(61), "text"], ["ok", "Text"], ["ok", "video"], [null, "text"], ["żółć-9", "text"], ["𝐀", "text"]]) {
  await step(`a channel named ${JSON.stringify(name)} of kind ${JSON.stringify(kind)}`, () => ask("ola", "POST", A, { name, kind }));
}
await step("a body of another shape", () => ask("ola", "POST", A, { kind: "text" }));

// Changing.
await step("ben changes a channel he does not see", () => ask("ben", "PATCH", `${A}/${byName("dev")}`, { topic: "hidden" }));
await step("cat changes a channel she does not see", () => ask("cat", "PATCH", `${A}/${byName("news")}`, { topic: "hidden" }));
await step("cat may not change what she sees", () => ask("cat", "PATCH", `${A}/${byName("general")}`, { topic: "no" }));
await step("ben sets a topic and slow mode", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { topic: "  Hello  ", slowModeSeconds: 30 }));
await step("nothing to change", () => ask("ben", "PATCH", `${A}/${byName("general")}`, {}));
await step("a voice setting on a text channel", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { userLimit: 5 }));
await step("a text setting on a voice channel", () => ask("ben", "PATCH", `${A}/${byName("voice")}`, { topic: "x" }));
await step("a limit on a voice channel", () => ask("ben", "PATCH", `${A}/${byName("voice")}`, { userLimit: 5, hideAfterInactivityHours: 24 }));
for (const [field, value] of [["slowModeSeconds", 21601], ["slowModeSeconds", -1], ["hideAfterInactivityHours", 8761], ["userLimit", 100], ["topic", "t".repeat(1025)]]) {
  await step(`${field} of ${String(value).length > 8 ? "too many" : value}`, () => ask("ola", "PATCH", `${A}/${byName("general")}`, { [field]: value }));
}
await step("a topic as long as it may be", () => ask("ola", "PATCH", `${A}/${byName("general")}`, { topic: "t".repeat(1024) }));
await step("a name a hidden channel has", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { name: "Dev" }));
await step("a new name", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { name: "  Lobby " }));
await step("an invalid name", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { name: "a b" }));
await step("a channel that is not there", () => ask("ola", "PATCH", `${A}/11111111-1111-1111-1111-111111111111`, {}));
await step("an id that is not one", () => ask("ola", "PATCH", `${A}/nope`, {}));

// Removing.
await step("ben removes a channel he does not see", () => ask("ben", "DELETE", `${A}/${byName("dev")}`));
await step("cat may not remove", () => ask("cat", "DELETE", `${A}/${byName("general")}`));
await step("ola removes a voice channel", () => ask("ola", "DELETE", `${A}/${byName("lounge")}`));
await step("ola removes a second", () => ask("ola", "DELETE", `${A}/${byName("ops")}`));
await step("ola removes a third", () => ask("ola", "DELETE", `${A}/${byName("voice")}`));
await step("the last voice channel stays", () => ask("ola", "DELETE", `${A}/${made.olas}`));
await step("a channel that is not there", () => ask("ola", "DELETE", `${A}/11111111-1111-1111-1111-111111111111`));

// Sections.
await step("cat may not make a section", () => ask("cat", "POST", S, { name: "Cats" }));
await step("ben makes a section", () => ask("ben", "POST", S, { name: "  Charlie  " }));
await step("a section with no name", () => ask("ben", "POST", S, { name: "   " }));
await step("a section named too long", () => ask("ben", "POST", S, { name: "x".repeat(61) }));
await step("a section named as long as it may be", () => ask("ben", "POST", S, { name: "𝐀".repeat(30) }));
await step("ben renames Bravo, which he does not see", () => ask("ben", "PATCH", `${S}/${sectionByName("Bravo")}`, { name: "No" }));
await step("dan renames Bravo, which he manages", () => ask("dan", "PATCH", `${S}/${sectionByName("Bravo")}`, { name: " Brav " }));
await step("dan renames it to the same", () => ask("dan", "PATCH", `${S}/${sectionByName("Bravo")}`, { name: "Brav" }));
await step("cat may not rename Alpha", () => ask("cat", "PATCH", `${S}/${sectionByName("Alpha")}`, { name: "No" }));
await step("a rename without a name", () => ask("ola", "PATCH", `${S}/${sectionByName("Alpha")}`, {}));
await step("a rename to nothing", () => ask("ola", "PATCH", `${S}/${sectionByName("Alpha")}`, { name: "" }));
await step("sections ben sees, reversed", () => ask("ben", "PUT", `${S}/order`, { sectionIds: [...sectionIds("ben")].reverse() }));
await step("a section order that names one ben does not see", () => ask("ben", "PUT", `${S}/order`, { sectionIds: sectionIds("ola") }));
await step("a section order with a section left out", () => ask("ola", "PUT", `${S}/order`, { sectionIds: sectionIds("ola").slice(1) }));
await step("cat may not order sections", () => ask("cat", "PUT", `${S}/order`, { sectionIds: sectionIds("cat") }));
await step("ola orders every section", () => ask("ola", "PUT", `${S}/order`, { sectionIds: shuffled(sectionIds("ola"), 5) }));
await step("a section with channels stays", () => ask("ola", "DELETE", `${S}/${sectionByName("Alpha")}`));
await step("cat may not remove a section", () => ask("cat", "DELETE", `${S}/${sectionByName("Alpha")}`));
await step("ben removes a section he does not see", () => ask("ben", "DELETE", `${S}/${sectionByName("Bravo")}`));
await step("ola removes an empty section", () => ask("ola", "DELETE", `${S}/${made.Charlie}`));
await step("a section that is not there", () =>
  ask("ola", "POST", A, { name: "nowhere", kind: "voice", sectionId: "11111111-1111-1111-1111-111111111111" }));
await step("ola creates a voice channel", () => ask("ola", "POST", A, { name: "  Olas  ", kind: "voice" }));
for (const [name, kind] of [["a b", "text"], ["", "text"], ["x".repeat(61), "text"], ["ok", "Text"], ["ok", "video"], [null, "text"], ["żółć-9", "text"], ["𝐀", "text"]]) {
  await step(`a channel named ${JSON.stringify(name)} of kind ${JSON.stringify(kind)}`, () => ask("ola", "POST", A, { name, kind }));
}
await step("a body of another shape", () => ask("ola", "POST", A, { kind: "text" }));

// Changing.
await step("ben changes a channel he does not see", () => ask("ben", "PATCH", `${A}/${byName("dev")}`, { topic: "hidden" }));
await step("cat changes a channel she does not see", () => ask("cat", "PATCH", `${A}/${byName("news")}`, { topic: "hidden" }));
await step("cat may not change what she sees", () => ask("cat", "PATCH", `${A}/${byName("general")}`, { topic: "no" }));
await step("ben sets a topic and slow mode", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { topic: "  Hello  ", slowModeSeconds: 30 }));
await step("nothing to change", () => ask("ben", "PATCH", `${A}/${byName("general")}`, {}));
await step("a voice setting on a text channel", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { userLimit: 5 }));
await step("a text setting on a voice channel", () => ask("ben", "PATCH", `${A}/${byName("voice")}`, { topic: "x" }));
await step("a limit on a voice channel", () => ask("ben", "PATCH", `${A}/${byName("voice")}`, { userLimit: 5, hideAfterInactivityHours: 24 }));
for (const [field, value] of [["slowModeSeconds", 21601], ["slowModeSeconds", -1], ["hideAfterInactivityHours", 8761], ["userLimit", 100], ["topic", "t".repeat(1025)]]) {
  await step(`${field} of ${String(value).length > 8 ? "too many" : value}`, () => ask("ola", "PATCH", `${A}/${byName("general")}`, { [field]: value }));
}
await step("a topic as long as it may be", () => ask("ola", "PATCH", `${A}/${byName("general")}`, { topic: "t".repeat(1024) }));
await step("a name a hidden channel has", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { name: "Dev" }));
await step("a new name", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { name: "  Lobby " }));
await step("an invalid name", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { name: "a b" }));
await step("a channel that is not there", () => ask("ola", "PATCH", `${A}/11111111-1111-1111-1111-111111111111`, {}));
await step("an id that is not one", () => ask("ola", "PATCH", `${A}/nope`, {}));

// Removing.
await step("ben removes a channel he does not see", () => ask("ben", "DELETE", `${A}/${byName("dev")}`));
await step("cat may not remove", () => ask("cat", "DELETE", `${A}/${byName("general")}`));
await step("ola removes a voice channel", () => ask("ola", "DELETE", `${A}/${byName("lounge")}`));
await step("ola removes a second", () => ask("ola", "DELETE", `${A}/${byName("ops")}`));
await step("ola removes a third", () => ask("ola", "DELETE", `${A}/${byName("voice")}`));
await step("the last voice channel stays", () => ask("ola", "DELETE", `${A}/${made.olas}`));
await step("a channel that is not there", () => ask("ola", "DELETE", `${A}/11111111-1111-1111-1111-111111111111`));

// Sections.
await step("cat may not make a section", () => ask("cat", "POST", S, { name: "Cats" }));
await step("ben makes a section", () => ask("ben", "POST", S, { name: "  Charlie  " }));
await step("a section with no name", () => ask("ben", "POST", S, { name: "   " }));
await step("a section named too long", () => ask("ben", "POST", S, { name: "x".repeat(61) }));
await step("a section named as long as it may be", () => ask("ben", "POST", S, { name: "𝐀".repeat(30) }));
await step("ben renames Bravo, which he does not see", () => ask("ben", "PATCH", `${S}/${sectionByName("Bravo")}`, { name: "No" }));
await step("dan renames Bravo, which he manages", () => ask("dan", "PATCH", `${S}/${sectionByName("Bravo")}`, { name: " Brav " }));
await step("dan renames it to the same", () => ask("dan", "PATCH", `${S}/${sectionByName("Bravo")}`, { name: "Brav" }));
await step("cat may not rename Alpha", () => ask("cat", "PATCH", `${S}/${sectionByName("Alpha")}`, { name: "No" }));
await step("a rename without a name", () => ask("ola", "PATCH", `${S}/${sectionByName("Alpha")}`, {}));
await step("a rename to nothing", () => ask("ola", "PATCH", `${S}/${sectionByName("Alpha")}`, { name: "" }));
await step("sections ben sees, reversed", () => ask("ben", "PUT", `${S}/order`, { sectionIds: [...sectionIds("ben")].reverse() }));
await step("a section order that names one ben does not see", () => ask("ben", "PUT", `${S}/order`, { sectionIds: sectionIds("ola") }));
await step("a section order with a section left out", () => ask("ola", "PUT", `${S}/order`, { sectionIds: sectionIds("ola").slice(1) }));
await step("cat may not order sections", () => ask("cat", "PUT", `${S}/order`, { sectionIds: sectionIds("cat") }));
await step("ola orders every section", () => ask("ola", "PUT", `${S}/order`, { sectionIds: shuffled(sectionIds("ola"), 5) }));
await step("a section with channels stays", () => ask("ola", "DELETE", `${S}/${sectionByName("Alpha")}`));
await step("cat may not remove a section", () => ask("cat", "DELETE", `${S}/${sectionByName("Alpha")}`));
await step("ben removes a section he does not see", () => ask("ben", "DELETE", `${S}/${sectionByName("Bravo")}`));
await step("ola removes an empty section", async () => {
  const sections = (await api(people.ola, "GET", "/api/bootstrap")).body.sections;
  return ask("ola", "DELETE", `${S}/${sections.find((x) => x.name === "Charlie").id}`);
});
await step("a section that is not there", () =>
  ask("ola", "POST", A, { name: "nowhere", kind: "voice", sectionId: "11111111-1111-1111-1111-111111111111" }));
await step("ola creates a voice channel", () => ask("ola", "POST", A, { name: "  Olas  ", kind: "voice" }));
for (const [name, kind] of [["a b", "text"], ["", "text"], ["x".repeat(61), "text"], ["ok", "Text"], ["ok", "video"], [null, "text"], ["żółć-9", "text"], ["𝐀", "text"]]) {
  await step(`a channel named ${JSON.stringify(name)} of kind ${JSON.stringify(kind)}`, () => ask("ola", "POST", A, { name, kind }));
}
await step("a body of another shape", () => ask("ola", "POST", A, { kind: "text" }));

// Changing.
await step("ben changes a channel he does not see", () => ask("ben", "PATCH", `${A}/${byName("dev")}`, { topic: "hidden" }));
await step("cat changes a channel she does not see", () => ask("cat", "PATCH", `${A}/${byName("news")}`, { topic: "hidden" }));
await step("cat may not change what she sees", () => ask("cat", "PATCH", `${A}/${byName("general")}`, { topic: "no" }));
await step("ben sets a topic and slow mode", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { topic: "  Hello  ", slowModeSeconds: 30 }));
await step("nothing to change", () => ask("ben", "PATCH", `${A}/${byName("general")}`, {}));
await step("a voice setting on a text channel", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { userLimit: 5 }));
await step("a text setting on a voice channel", () => ask("ben", "PATCH", `${A}/${byName("voice")}`, { topic: "x" }));
await step("a limit on a voice channel", () => ask("ben", "PATCH", `${A}/${byName("voice")}`, { userLimit: 5, hideAfterInactivityHours: 24 }));
for (const [field, value] of [["slowModeSeconds", 21601], ["slowModeSeconds", -1], ["hideAfterInactivityHours", 8761], ["userLimit", 100], ["topic", "t".repeat(1025)]]) {
  await step(`${field} of ${String(value).length > 8 ? "too many" : value}`, () => ask("ola", "PATCH", `${A}/${byName("general")}`, { [field]: value }));
}
await step("a topic as long as it may be", () => ask("ola", "PATCH", `${A}/${byName("general")}`, { topic: "t".repeat(1024) }));
await step("a name a hidden channel has", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { name: "Dev" }));
await step("a new name", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { name: "  Lobby " }));
await step("an invalid name", () => ask("ben", "PATCH", `${A}/${byName("general")}`, { name: "a b" }));
await step("a channel that is not there", () => ask("ola", "PATCH", `${A}/11111111-1111-1111-1111-111111111111`, {}));
await step("an id that is not one", () => ask("ola", "PATCH", `${A}/nope`, {}));

// Removing.
await step("ben removes a channel he does not see", () => ask("ben", "DELETE", `${A}/${byName("dev")}`));
await step("cat may not remove", () => ask("cat", "DELETE", `${A}/${byName("general")}`));
await step("ola removes a voice channel", () => ask("ola", "DELETE", `${A}/${byName("lounge")}`));
await step("ola removes a second", () => ask("ola", "DELETE", `${A}/${byName("ops")}`));
await step("ola removes a third", () => ask("ola", "DELETE", `${A}/${byName("voice")}`));
await step("the last voice channel stays", async () => {
  const voice = (await api(people.ola, "GET", "/api/bootstrap")).body.channels.filter((x) => x.kind === "voice");
  out(`   voice channels left: ${voice.map((x) => x.name).join(", ")}`);
  return ask("ola", "DELETE", `${A}/${voice[0].id}`);
});
await step("a channel that is not there", () => ask("ola", "DELETE", `${A}/11111111-1111-1111-1111-111111111111`));

// Sections.
await step("cat may not make a section", () => ask("cat", "POST", S, { name: "Cats" }));
await step("ben makes a section", () => ask("ben", "POST", S, { name: "  Charlie  " }));
await step("a section with no name", () => ask("ben", "POST", S, { name: "   " }));
await step("a section named too long", () => ask("ben", "POST", S, { name: "x".repeat(61) }));
await step("a section named as long as it may be", () => ask("ben", "POST", S, { name: "𝐀".repeat(30) }));
await step("ben renames Bravo, which he does not see", () => ask("ben", "PATCH", `${S}/${sectionByName("Bravo")}`, { name: "No" }));
await step("dan renames Bravo, which he manages", () => ask("dan", "PATCH", `${S}/${sectionByName("Bravo")}`, { name: " Brav " }));
await step("dan renames it to the same", () => ask("dan", "PATCH", `${S}/${sectionByName("Bravo")}`, { name: "Brav" }));
await step("cat may not rename Alpha", () => ask("cat", "PATCH", `${S}/${sectionByName("Alpha")}`, { name: "No" }));
await step("a rename without a name", () => ask("ola", "PATCH", `${S}/${sectionByName("Alpha")}`, {}));
await step("a rename to nothing", () => ask("ola", "PATCH", `${S}/${sectionByName("Alpha")}`, { name: "" }));
await step("sections ben sees, reversed", () => ask("ben", "PUT", `${S}/order`, { sectionIds: [...sectionIds("ben")].reverse() }));
await step("a section order that names one ben does not see", () => ask("ben", "PUT", `${S}/order`, { sectionIds: sectionIds("ola") }));
await step("a section order with a section left out", () => ask("ola", "PUT", `${S}/order`, { sectionIds: sectionIds("ola").slice(1) }));
await step("cat may not order sections", () => ask("cat", "PUT", `${S}/order`, { sectionIds: sectionIds("cat") }));
await step("ola orders every section", () => ask("ola", "PUT", `${S}/order`, { sectionIds: shuffled(sectionIds("ola"), 5) }));
await step("a section with channels stays", () => ask("ola", "DELETE", `${S}/${sectionByName("Alpha")}`));
await step("cat may not remove a section", () => ask("cat", "DELETE", `${S}/${sectionByName("Alpha")}`));
await step("ben removes a section he does not see", () => ask("ben", "DELETE", `${S}/${sectionByName("Bravo")}`));
const empty = () => [...names.entries()].find(([, name]) => name === "new 7")?.[0];
await step("ola removes an empty section", async () => {
  const sections = (await api(people.ola, "GET", "/api/bootstrap")).body.sections;
  const charlie = sections.find((x) => x.name === "Charlie");
  return ask("ola", "DELETE", `${S}/${charlie?.id ?? empty() ?? "11111111-1111-1111-1111-111111111111"}`);
});
await step("a section that is not there", () => ask("ola", "DELETE", `${S}/11111111-1111-1111-1111-111111111111`));

// Reading.
await step("ola reads a channel", () => ask("ola", "PUT", `/api/channels/${byName("news")}/read`));
await step("ola reads it again", () => ask("ola", "PUT", `/api/channels/${byName("news")}/read`));
await step("cat reads a channel she does not see", () => ask("cat", "PUT", `/api/channels/${byName("news")}/read`));
await step("ben reads a channel he sees", () => ask("ben", "PUT", `/api/channels/${byName("design")}/read`));
await step("ben reads a channel he does not see", () => ask("ben", "PUT", `/api/channels/${byName("dev")}/read`));
await step("reading a voice channel", () => ask("ola", "PUT", `/api/channels/${byName("ops")}/read`));
await step("reading a channel that is not there", () => ask("ola", "PUT", "/api/channels/11111111-1111-1111-1111-111111111111/read"));

// The hub.
await step("ben joins a channel he sees", () => hub("ben", "JoinChannel", byName("design")));
await step("ben joins one he does not see", () => hub("ben", "JoinChannel", byName("dev")));
await step("ben joins a voice channel", () => hub("ben", "JoinChannel", byName("ops")));
await step("joining what is not an id", () => hub("ben", "JoinChannel", "nope"));
await step("joining with nothing", () => hub("ben", "JoinChannel"));
await step("joining a channel that is not there", () => hub("ben", "JoinChannel", "11111111-1111-1111-1111-111111111111"));
await step("ola types in dev, which ben does not see", () => hub("ola", "SendTyping", byName("dev")));
await step("ola types in design", () => hub("ola", "SendTyping", byName("design")));
await step("ben types in design", () => hub("ben", "SendTyping", byName("design")));
await step("cat types where she does not see", () => hub("cat", "SendTyping", byName("secret")));
await step("dan types where he may not send", () => hub("dan", "SendTyping", byName("news")));
await step("typing in a voice channel", () => hub("ola", "SendTyping", byName("ops")));
await step("typing in what is not an id", () => hub("ola", "SendTyping", "nope"));
await step("typing too fast", async () => {
  const answers = [];
  for (let i = 0; i < 14; i++) answers.push((await clients.eve.invoke("SendTyping", byName("design"))).error ?? "ok");
  return answers.join(" | ");
});

writeFileSync(args.out, lines.join("\n") + "\n");
for (const client of Object.values(clients)) client.socket.close(1000);
process.exit(0);
