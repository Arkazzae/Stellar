// What a client of the direct conversations observes, as a transcript, from one
// server. Run it against this server and against the first one on copies of one
// database (see dms-oracle.sh) and compare the transcripts.
//
//   node dms-oracle.mjs prepare --base URL --ids F   turns encryption on, by the first server's own routes
//   node dms-oracle.mjs story   --base URL --out F --ids F  the story, with a hub open for four people
//   node dms-oracle.mjs pages   --base URL --out F --cursors F   history paged to its end, cursors kept
//   node dms-oracle.mjs feed    --base URL --out F --cursors F   another server's cursors, asked of this one
//
// Needs Node 26. The accounts (ola, ben, cat, dan, gus), the uploads and the
// settings are made by the shell script. Ids are replaced by names and by the
// order they were first seen in.

import { createHash, randomBytes } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const mode = process.argv[2];
const { values: args } = parseArgs({
  args: process.argv.slice(3),
  options: {
    base: { type: "string" }, out: { type: "string" }, cursors: { type: "string" }, ids: { type: "string" }, password: { type: "string", default: "Corona!2026-long" },
    eve: { type: "string" }, fay: { type: "string" },
  },
});
const base = args.base;
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// The same bytes whichever server is asked, so that what comes back compares.
const b64 = (n) => Buffer.alloc(n, (n * 7 + 3) % 251).toString("base64");
const rnd64 = (n) => randomBytes(n).toString("base64");

const names = new Map();
const seen = new Map();
const lines = [];
const out = (text) => lines.push(text);

function scrub(value) {
  let text = typeof value === "string" ? value : JSON.stringify(value);
  for (const [id, name] of names) text = text.replaceAll(id, `<${name}>`);
  text = text.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, (id) => {
    const key = id.toLowerCase();
    if (!seen.has(key)) seen.set(key, `id${seen.size + 1}`);
    return `<${seen.get(key)}>`;
  });
  text = text.replace(/"(nextCursor)":"[A-Za-z0-9_-]+"/g, '"$1":"<cursor>"');
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
  const keep = ["content-type", "location", "allow", "retry-after"];
  const headers = keep.filter((h) => response.headers.has(h)).map((h) => `${h}: ${response.headers.get(h)}`).join("; ");
  return { status: response.status, text, headers, json: () => JSON.parse(text) };
}

async function signIn(username) {
  const response = await fetch(base + "/api/auth/login", { method: "POST", headers: clientHeaders, body: JSON.stringify({ username, password: args.password }) });
  if (response.status !== 200) throw new Error(`signing in ${username}: ${response.status}`);
  const body = await response.json();
  const id = body.id ?? body.user?.id;
  names.set(id, username);
  return { username, id, cookie: response.headers.getSetCookie().map((line) => line.split(";")[0]).join("; ") };
}

class Hub {
  constructor(label, person) { this.label = label; this.person = person; this.records = []; this.waiting = new Map(); this.next = 0; this.buffer = ""; }
  async open() {
    this.socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app`, { headers: { Cookie: this.person.cookie } });
    await new Promise((resolve, reject) => { this.socket.onopen = resolve; this.socket.onerror = () => reject(new Error(`the upgrade for ${this.label} failed`)); });
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
      this.records.push(record);
    }
  }
  take() { const taken = this.records; this.records = []; return taken; }
}

const hubs = [];
async function connect(label, person) {
  const hub = await new Hub(label, person).open();
  hubs.push(hub);
  // The server joins the connection to its groups after the handshake; the
  // next connection must not start before, or it would be told less.
  await sleep(500);
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
  await sleep(350);
  for (const hub of hubs) {
    const heard = hub.take().map((record) => `   ${hub.label} hears: ${scrub(record)}`);
    if (title.includes("connects")) heard.sort();
    for (const line of heard) out(line);
  }
}

const P = {};
// A body is compared as the value it writes: the first server escapes every
// character that is not ASCII, this one writes it as it is.
const normal = (text) => { try { return JSON.stringify(JSON.parse(text)); } catch { return text; } };
const show = (r) => `${r.status} [${r.headers}] ${normal(r.text)}`;
const answer = async (who, method, path, body, extra) => `${who} ${method} ${path.replace(/\?.*/, (q) => q)} -> ${show(await http(P[who], method, path, body, extra))}`;

// ---------------------------------------------------------------- prepare
if (mode === "prepare") {
  for (const name of ["ola", "ben", "cat"]) P[name] = await signIn(name);
  const must = async (who, method, path, body, want) => {
    const r = await http(P[who], method, path, body);
    if (!want.includes(r.status)) throw new Error(`${who} ${method} ${path}: ${r.status} ${r.text}`);
    return r.status === 204 || r.text === "" ? null : r.json();
  };
  const setUp = async (who) => {
    const authKey = randomBytes(32);
    await must(who, "POST", "/api/me/encryption/account-key", {
      algorithm: "x25519", publicKey: rnd64(32),
      backup: { salt: rnd64(16), iterations: 600000, verifier: createHash("sha256").update(authKey).digest("base64"), iv: rnd64(12), ciphertext: rnd64(64) },
    }, [201]);
    return await must(who, "POST", "/api/me/encryption/devices", { algorithm: "x25519", publicKey: rnd64(32), label: "Firefox, Linux" }, [201]);
  };
  for (const name of ["ola", "ben", "cat"]) await setUp(name);
  const open = async (who, other) => (await must(who, "POST", "/api/social/conversations", { userId: P[other].id }, [200])).id;
  const epoch = async (who, conversation) => {
    const keys = await must(who, "GET", `/api/social/conversations/${conversation}/encryption/keys`, undefined, [200]);
    const epochId = crypto.randomUUID();
    await must(who, "POST", `/api/social/conversations/${conversation}/encryption/epochs`, {
      epochId, previousEpochId: keys.encryption.currentEpochId,
      envelopes: keys.recipients.map((r) => ({ recipientKeyId: r.keyId, ephemeralPublicKey: rnd64(32), iv: rnd64(12), ciphertext: rnd64(48) })),
    }, [201]);
    return epochId;
  };
  const enable = async (a, b, conversation) => {
    await must(a, "POST", `/api/social/conversations/${conversation}/encryption/request`, undefined, [200]);
    await must(b, "POST", `/api/social/conversations/${conversation}/encryption/confirm`, undefined, [200]);
  };
  const benCat = await open("ben", "cat");
  await enable("ben", "cat", benCat);
  await epoch("ben", benCat);
  // A second browser of cat's after the epoch of ben's conversation: that epoch
  // no longer reaches every key. Ola's conversation is turned on after it, so
  // its epochs do.
  const again = await signIn("cat");
  const device = await fetch(base + "/api/me/encryption/devices", { method: "POST", headers: { ...clientHeaders, Cookie: again.cookie }, body: JSON.stringify({ algorithm: "x25519", publicKey: rnd64(32), label: "Chrome" }) });
  if (device.status !== 201) throw new Error("second device " + device.status);
  const olaCat = await open("ola", "cat");
  await enable("ola", "cat", olaCat);
  const first = await epoch("ola", olaCat);
  await epoch("cat", olaCat);
  writeFileSync(args.ids, JSON.stringify({ old: first }));
  process.exit(0);
}

// ---------------------------------------------------------------- the common ground
for (const name of ["ola", "ben", "cat", "dan", "gus"]) P[name] = await signIn(name);
names.set(args.eve, "eve");
names.set(args.fay, "fay");
const uuid = (n, prefix = "C0") => `${prefix}000000-0000-0000-0000-${String(n).padStart(12, "0")}`;
const C = {}; // conversation ids by the peer's name, from ola's point of view unless said

const social = async (who) => (await http(P[who], "GET", "/api/social")).json();
const conversationWith = async (who, peer) => (await social(who)).conversations.find((c) => c.peer.username === peer)?.id;
const open = (who, other) => answer(who, "POST", "/api/social/conversations", { userId: typeof other === "string" && P[other] ? P[other].id : other });
const send = (who, conversation, body) => answer(who, "POST", `/api/social/conversations/${conversation}/messages`, body);
const sendRaw = (who, conversation, text, type = "application/json") => answer(who, "POST", `/api/social/conversations/${conversation}/messages`, text, { "Content-Type": type });
const read = (who, conversation) => answer(who, "POST", `/api/social/conversations/${conversation}/read`);
const page = (who, conversation, query = "") => answer(who, "GET", `/api/social/conversations/${conversation}/messages${query}`);
const block = (who, other) => answer(who, "POST", `/api/social/blocks/${P[other].id}`);
const unblock = (who, other) => answer(who, "DELETE", `/api/social/blocks/${P[other].id}`);
const ask = (who, name) => answer(who, "POST", "/api/social/requests", { username: name });
const accept = async (who) => answer(who, "POST", `/api/social/requests/${(await social(who)).requests[0].id}/accept`);

async function views(...who) {
  const parts = [];
  for (const name of who) {
    const s = await http(P[name], "GET", "/api/social");
    parts.push(`   ${name} social: ${scrub(s.text)}`);
  }
  return parts.join("\n");
}
const checkpoint = (title, ...who) => step(`${title}: what each sees`, async () => `\n${await views(...who)}`);

// ---------------------------------------------------------------- pages / feed
if (mode === "pages" || mode === "feed") {
  const found = {};
  for (const peer of ["ben", "cat", "dan"]) found[peer] = await conversationWith("ola", peer);
  const kept = {};
  if (mode === "feed") Object.assign(kept, JSON.parse(readFileSync(args.cursors, "utf8")));
  const record = {};
  for (const peer of ["ben", "cat", "dan"]) {
    const conversation = found[peer];
    if (!conversation) continue;
    names.set(conversation, `conversation-${peer}`);
    for (const size of [1, 2, 3, 5, 100]) {
      if (mode === "pages") {
        let cursor = null;
        for (let n = 0; n < 200; n++) {
          const r = await http(P.ola, "GET", `/api/social/conversations/${conversation}/messages?limit=${size}${cursor ? `&cursor=${cursor}` : ""}`);
          const body = r.json();
          out(`${peer} size ${size} page ${n}: ${r.status} ${body.messages.map((m) => m.system ? `[${m.system.kind}]` : m.encrypted ? "(sealed)" : m.content.slice(0, 20)).join("|")} next=${body.nextCursor ? "yes" : "no"}`);
          (record[`${peer}:${size}`] ??= []).push(body.nextCursor);
          cursor = body.nextCursor;
          if (!cursor) break;
        }
      } else {
        for (const [n, cursor] of (kept[`${peer}:${size}`] ?? []).entries()) {
          if (!cursor) continue;
          const r = await http(P.ola, "GET", `/api/social/conversations/${conversation}/messages?limit=${size}&cursor=${cursor}`);
          const body = r.status === 200 ? r.json() : null;
          out(`${peer} size ${size} after page ${n}: ${r.status} ${body ? body.messages.map((m) => m.system ? `[${m.system.kind}]` : m.encrypted ? "(sealed)" : m.content.slice(0, 20)).join("|") : scrub(r.text)} next=${body?.nextCursor ? "yes" : "no"}`);
        }
        // The same cursor for another conversation, and one cut short.
        const wrong = (kept[`ben:1`] ?? [])[0];
        if (wrong && peer !== "ben") {
          const r = await http(P.ola, "GET", `/api/social/conversations/${conversation}/messages?limit=1&cursor=${wrong}`);
          out(`${peer} with the cursor of another conversation: ${r.status} ${scrub(r.text)}`);
        }
      }
    }
  }
  if (mode === "pages") writeFileSync(args.cursors, JSON.stringify(record));
  writeFileSync(args.out, lines.join("\n") + "\n");
  process.exit(0);
}

// ---------------------------------------------------------------- the story
const hola = await connect("ola", P.ola);
const hben = await connect("ben", P.ben);
const hcat = await connect("cat", P.cat);
const hgus = await connect("gus", P.gus);
await step("everybody connects");

// Opening.
await step("ola opens a conversation with ben", () => open("ola", "ben"));
C.ben = await conversationWith("ola", "ben");
names.set(C.ben, "conversation-ben");
await step("ola opens it again", () => open("ola", "ben"));
await step("ben opens it from his side", () => open("ben", "ola"));
await step("ola and dan open one at the same moment", async () => {
  const [one, two] = await Promise.all([open("ola", "dan"), open("dan", "ola")]);
  return `${one.replace(/^ola /, "")} | ${two.replace(/^dan /, "")}`;
});
C.dan = await conversationWith("ola", "dan");
names.set(C.dan, "conversation-dan");
await step("ola opens a conversation with herself", () => open("ola", "ola"));
await step("ola opens one with nobody", () => open("ola", "11111111-1111-1111-1111-111111111111"));
await step("ola opens one with an empty body", () => answer("ola", "POST", "/api/social/conversations", {}));
await step("ola opens one with a null id", () => answer("ola", "POST", "/api/social/conversations", { userId: null }));
await step("ola opens one with an id without hyphens", () => open("ola", P.ben.id.replaceAll("-", "")));
for (const body of ["null", "[]", "not json", '{"userId":5}', '{"USERID":"11111111-1111-1111-1111-111111111111"}']) {
  await step(`ola opens one with ${body}`, () => answer("ola", "POST", "/api/social/conversations", body));
}
await step("ola opens one as text", () => answer("ola", "POST", "/api/social/conversations", '{}', { "Content-Type": "text/plain" }));
await step("ola opens one with a person who stands in for somebody", () => open("ola", args.eve));
await step("ola opens one with a disabled person", () => open("ola", args.fay));
C.fay = await conversationWith("ola", "fay");
names.set(C.fay, "conversation-fay");
await step("ola posts to the disabled person", () => send("ola", C.fay, { content: "anybody there?" }));
await step("anonymous opens one", () => http(null, "POST", "/api/social/conversations", { userId: P.ben.id }).then(show));
await checkpoint("opened", "ola", "ben", "gus");

// Posting.
await step("ola posts", () => send("ola", C.ben, { content: "  hello ben  ", clientMessageId: uuid(1) }));
await step("ola retries the same send", () => send("ola", C.ben, { content: "hello ben", clientMessageId: uuid(1) }));
await step("ola retries with other text", () => send("ola", C.ben, { content: "hello again", clientMessageId: uuid(1) }));
await step("ben sends under ola's client id", () => send("ben", C.ben, { content: "hello ben", clientMessageId: uuid(1) }));
await step("ola sends under that id elsewhere", () => send("ola", C.dan, { content: "hello ben", clientMessageId: uuid(1) }));
await step("ola posts without a client id", () => send("ola", C.ben, { content: "no id" }));
await step("ola posts with the key in capitals", () => sendRaw("ola", C.ben, '{"CONTENT":"capitals"}'));
await step("ola posts with a zero client id", () => send("ola", C.ben, { content: "x", clientMessageId: "00000000-0000-0000-0000-000000000000" }));
await step("ola posts with a client id without hyphens", () => send("ola", C.ben, { content: "x", clientMessageId: "11111111111111111111111111111111" }));
await step("ola posts nothing", () => send("ola", C.ben, { content: "   " }));
await step("ola posts nothing at all", () => send("ola", C.ben, {}));
await step("ola posts 20000 characters", () => send("ola", C.ben, { content: "x".repeat(20000), clientMessageId: uuid(2) }).then((s) => s.slice(0, 120)));
await step("ola posts 20001 characters", () => send("ola", C.ben, { content: "x".repeat(20001) }));
await step("ola posts 10000 astral characters", () => send("ola", C.ben, { content: "😀".repeat(10000), clientMessageId: uuid(3) }).then((s) => s.slice(0, 120)));
await step("ola posts 10001 astral characters", () => send("ola", C.ben, { content: "😀".repeat(10001) }));
await step("ola posts with spaces around 20000 characters", () => send("ola", C.ben, { content: ` ${"y".repeat(20000)} `, clientMessageId: uuid(4) }).then((s) => s.slice(0, 120)));
for (const body of ["null", "[]", "not json", '{"content":5}', '{"attachmentIds":[null]}', '{"attachmentIds":"x"}', '{"clientMessageId":5}', '{"encrypted":5}', '{"content":"a","encrypted":null}']) {
  await step(`ola posts ${body}`, () => sendRaw("ola", C.ben, body));
}
await step("ola posts as text", () => sendRaw("ola", C.ben, '{"content":"x"}', "text/plain"));

// Attachments.
const plain = (n) => uuid(n, "A1");
const sealed = (n) => uuid(n, "A2");
await step("ola posts one attachment", () => send("ola", C.ben, { attachmentIds: [plain(1)], clientMessageId: uuid(10) }));
await step("ola posts text with attachments", () => send("ola", C.ben, { content: "look", attachmentIds: [plain(3), plain(2)], clientMessageId: uuid(11) }));
await step("ola posts an attachment already used", () => send("ola", C.ben, { attachmentIds: [plain(1)], clientMessageId: uuid(12) }));
await step("ola posts an attachment twice in one message", () => send("ola", C.ben, { attachmentIds: [plain(4), plain(4)], clientMessageId: uuid(13) }));
await step("ola posts ten attachments", () => send("ola", C.ben, { attachmentIds: Array.from({ length: 10 }, (_, i) => plain(5 + i)), clientMessageId: uuid(14) }).then((s) => s.slice(0, 160)));
await step("ola posts eleven attachments", () => send("ola", C.ben, { attachmentIds: Array.from({ length: 11 }, (_, i) => plain(1 + i)) }));
await step("ola posts the attachment of somebody else", () => send("ola", C.ben, { attachmentIds: [uuid(1, "B1")] }));
await step("ola posts an attachment of nobody", () => send("ola", C.ben, { attachmentIds: ["11111111-1111-1111-1111-111111111111"] }));
await step("ola posts an encrypted upload in a plain message", () => send("ola", C.ben, { content: "x", attachmentIds: [sealed(1)] }));
await step("ola retries an attachment send with other uploads", () => send("ola", C.ben, { attachmentIds: [plain(2)], clientMessageId: uuid(10) }));
await step("ola retries an attachment send", () => send("ola", C.ben, { attachmentIds: [plain(1)], clientMessageId: uuid(10) }));
await step("ola posts the attachment of a voice message", () => send("ola", C.ben, { attachmentIds: [uuid(1, "A3")], clientMessageId: uuid(15) }));

// Reading.
await step("ben reads the history", () => page("ben", C.ben, "?limit=3"));
await step("ben reads the history with the limit in another case", () => page("ben", C.ben, "?LIMIT=+2"));
await step("ben reads with a bad limit", () => page("ben", C.ben, "?limit=0"));
await step("ben reads with a limit that is no number", () => page("ben", C.ben, "?limit=abc"));
await step("ben reads with an empty limit", () => page("ben", C.ben, "?limit="));
await step("ben reads with a limit of 101", () => page("ben", C.ben, "?limit=101"));
await step("ben reads with a cursor that is nothing", () => page("ben", C.ben, "?cursor="));
await step("ben reads with a cursor that is rubbish", () => page("ben", C.ben, "?cursor=abc"));
await step("ben reads with a cursor of 4 bytes", () => page("ben", C.ben, "?cursor=AAAAAA"));
await step("ben reads with a cursor cut off", () => page("ben", C.ben, `?cursor=${Buffer.alloc(49, 1).toString("base64url")}`));
await step("ben reads with a cursor of another conversation", async () => {
  const first = (await http(P.ola, "GET", `/api/social/conversations/${C.dan}/messages?limit=1`)).json();
  return first.nextCursor ? page("ben", C.ben, `?cursor=${first.nextCursor}`) : "no cursor";
});
await step("gus reads ben's conversation", () => page("gus", C.ben));
await step("gus posts in it", () => send("gus", C.ben, { content: "hi" }));
await step("gus marks it read", () => read("gus", C.ben));
await step("ola reads a conversation that is nobody's", () => page("ola", "11111111-1111-1111-1111-111111111111"));
await step("ola reads a name that is no id", () => page("ola", "nope"));
await checkpoint("posted", "ola", "ben");
await step("ben marks it read", () => read("ben", C.ben));
await step("ben marks it read again", () => read("ben", C.ben));
await step("ben marks it read with a body", () => answer("ben", "POST", `/api/social/conversations/${C.ben}/read`, { a: 1 }));
await checkpoint("read", "ola", "ben");
await step("ben answers", () => send("ben", C.ben, { content: "hi ola", clientMessageId: uuid(20) }));
await checkpoint("answered", "ola", "ben");

// What a person's settings and a block change.
await step("ola posts to dan, who takes messages from friends only", () => send("ola", C.dan, { content: "hello dan", clientMessageId: uuid(30) }));
await step("dan posts to ola", () => send("dan", C.dan, { content: "hello ola", clientMessageId: uuid(31) }));
await step("ola retries her refused send", () => send("ola", C.dan, { content: "hello dan", clientMessageId: uuid(30) }));
await step("ola asks dan to be friends", () => ask("ola", "dan"));
await step("dan accepts", () => accept("dan"));
await step("ola posts to dan, now a friend", () => send("ola", C.dan, { content: "hello dan", clientMessageId: uuid(30) }));
await step("ben blocks ola", () => block("ben", "ola"));
await step("ola posts to ben", () => send("ola", C.ben, { content: "after the block" }));
await step("ben posts to ola", () => send("ben", C.ben, { content: "I blocked you" }));
await step("ola retries an old send while blocked", () => send("ola", C.ben, { content: "hello ben", clientMessageId: uuid(1) }));
await step("ola reads the history while blocked", () => page("ola", C.ben));
await step("ben reads the history while blocked", () => page("ben", C.ben));
await step("ola marks it read while blocked", () => read("ola", C.ben));
await step("ola opens it while blocked", () => open("ola", "ben"));
await step("ben opens it while blocked", () => open("ben", "ola"));
await checkpoint("blocked", "ola", "ben");
await step("ben unblocks ola", () => unblock("ben", "ola"));
await step("ola posts to ben again", () => send("ola", C.ben, { content: "welcome back", clientMessageId: uuid(40) }));
await step("ola reads the history again", () => page("ola", C.ben, "?limit=4"));
await checkpoint("after the block", "ola", "ben", "gus");

// Encrypted conversations, turned on through the first server's routes.
C.cat = await conversationWith("ola", "cat");
names.set(C.cat, "conversation-cat");
C.bencat = await conversationWith("ben", "cat");
names.set(C.bencat, "conversation-bencat");
const current = (await social("ola")).conversations.find((c) => c.id === C.cat)?.encryption.currentEpochId;
const foreign = (await social("ben")).conversations.find((c) => c.id === C.bencat)?.encryption.currentEpochId;
const old = JSON.parse(readFileSync(args.ids, "utf8")).old;
const sealedBody = (epoch, id, iv = 12, cipher = 40, extra = {}) => ({ clientMessageId: id, encrypted: { epochId: epoch, iv: b64(iv), ciphertext: b64(cipher) }, ...extra });
await checkpoint("encrypted", "ola", "cat");
await step("ola posts plain text to an encrypted conversation", () => send("ola", C.cat, { content: "secret" }));
await step("ola posts plain text with an upload to it", () => send("ola", C.cat, { attachmentIds: [plain(11)] }));
await step("ola posts ciphertext", () => send("ola", C.cat, sealedBody(current, uuid(50))));
await step("ola retries the ciphertext", () => send("ola", C.cat, sealedBody(current, uuid(50), 12, 90)));
await step("ola retries it as plain text", () => send("ola", C.cat, { content: "x", clientMessageId: uuid(50) }));
await step("cat reads it", () => page("cat", C.cat));
for (const [iv, cipher] of [[12, 17], [12, 16], [12, 1], [11, 40], [13, 40], [12, 262144], [12, 262145]]) {
  await step(`ola posts an iv of ${iv} bytes and a ciphertext of ${cipher}`, () => send("ola", C.cat, sealedBody(current, uuid(100 + iv + cipher % 1000), iv, cipher)).then((s) => s.slice(0, 260)));
}
await step("ola posts ciphertext without a client id", () => send("ola", C.cat, { encrypted: { epochId: current, iv: b64(12), ciphertext: b64(40) } }));
await step("ola posts ciphertext with plain text beside it", () => send("ola", C.cat, sealedBody(current, uuid(51), 12, 40, { content: "x" })));
await step("ola posts ciphertext with empty text beside it", () => send("ola", C.cat, sealedBody(current, uuid(52), 12, 40, { content: "" })));
await step("ola posts ciphertext without an epoch", () => send("ola", C.cat, { clientMessageId: uuid(53), encrypted: { iv: b64(12), ciphertext: b64(40) } }));
await step("ola posts ciphertext without an iv", () => send("ola", C.cat, { clientMessageId: uuid(54), encrypted: { epochId: current, ciphertext: b64(40) } }));
await step("ola posts ciphertext without ciphertext", () => send("ola", C.cat, { clientMessageId: uuid(55), encrypted: { epochId: current, iv: b64(12) } }));
await step("ola posts ciphertext that is no Base64", () => sendRaw("ola", C.cat, `{"clientMessageId":"${uuid(56)}","encrypted":{"epochId":"${current}","iv":"!!","ciphertext":"x"}}`));
await step("ola posts Base64 with spaces in it", () => sendRaw("ola", C.cat, `{"clientMessageId":"${uuid(57)}","encrypted":{"epochId":"${current}","iv":"AAAA AAAA\\nAAAA","ciphertext":"${b64(40)}"}}`));
await step("ola posts an empty encrypted object", () => send("ola", C.cat, { clientMessageId: uuid(58), encrypted: {} }));
await step("ola posts an epoch that is nobody's", () => send("ola", C.cat, sealedBody("11111111-1111-1111-1111-111111111111", uuid(59))));
await step("ola posts an epoch of the conversation that is no longer current", () => send("ola", C.cat, sealedBody(old ?? "11111111-1111-1111-1111-111111111112", uuid(60))));
await step("ola posts an epoch of another conversation", () => send("ola", C.cat, sealedBody(foreign, uuid(61))));
await step("ola posts twenty encrypted uploads", () => send("ola", C.cat, sealedBody(current, uuid(62), 12, 40, { attachmentIds: Array.from({ length: 20 }, (_, i) => sealed(1 + i)) })).then((s) => s.slice(0, 200)));
await step("ola posts twenty-one encrypted uploads", () => send("ola", C.cat, sealedBody(current, uuid(63), 12, 40, { attachmentIds: Array.from({ length: 21 }, (_, i) => sealed(1 + i)) })));
await step("ola posts a plain upload in an encrypted message", () => send("ola", C.cat, sealedBody(current, uuid(64), 12, 40, { attachmentIds: [plain(12)] })));
await step("ola posts the same ciphertext with other uploads", () => send("ola", C.cat, sealedBody(current, uuid(62), 12, 40, { attachmentIds: [sealed(1)] })));
await step("ola posts to ben and cat's conversation", () => send("ola", C.bencat, sealedBody(foreign, uuid(65))));
await step("ben posts to a conversation whose epoch lacks a key", () => send("ben", C.bencat, sealedBody(foreign, uuid(66))));
await step("ben posts plain text to it", () => send("ben", C.bencat, { content: "plain" }));
await step("ola reads the encrypted history", () => page("ola", C.cat, "?limit=100"));
await checkpoint("end of the story", "ola", "ben", "cat", "gus");

writeFileSync(args.out, lines.join("\n") + "\n");
for (const hub of hubs) hub.socket.close(1000);
process.exit(0);
