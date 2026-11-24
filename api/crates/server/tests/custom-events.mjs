// What the people connected to the hub are told when emoji, the soundboard,
// the own profile and the preferences change, as a transcript. Run it against
// the first server and against this one, on fresh instances, and compare.
//
//   node custom-events.mjs --base http://127.0.0.1:5224 --setup CODE [--database file] --out transcript.txt
//
// Needs Node 26 (the global WebSocket takes request headers).

import { writeFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { DatabaseSync } from "node:sqlite";
import { randomUUID } from "node:crypto";

const { values: args } = parseArgs({
  options: { base: { type: "string" }, setup: { type: "string" }, out: { type: "string" }, database: { type: "string" }, password: { type: "string", default: "Corona!2026-long" } },
});
const base = args.base;
const headers = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const names = new Map();
const lines = [];
const out = (text) => lines.push(text);
const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3, 4]);
const WAVE = new TextEncoder().encode("RIFF\0\0\0\0WAVE\0\0\0\0");

function scrub(value) {
  let text = typeof value === "string" ? value : JSON.stringify(value);
  for (const [id, name] of names) text = text.replaceAll(id, `<${name}>`);
  return text
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, "<id>")
    .replace(/00-[0-9a-f]{32}-[0-9a-f]{16}-00/g, "<trace>")
    .replace(/\d{4}-\d\d-\d\dT[\d:.]+Z?/g, "<time>")
    .replace(/oklch\([^)]*\)/g, "<colour>");
}

async function call(person, method, path, body) {
  const init = { method, headers: { ...headers, Cookie: person.cookie } };
  if (body instanceof FormData) init.body = body;
  else if (body !== undefined) {
    init.headers["Content-Type"] = "application/json";
    init.body = JSON.stringify(body);
  }
  const response = await fetch(base + path, init);
  const text = await response.text();
  return { status: response.status, body: text ? JSON.parse(text) : null };
}

async function register(username, extra) {
  const response = await fetch(`${base}/api/auth/register`, {
    method: "POST",
    headers: { ...headers, "Content-Type": "application/json" },
    body: JSON.stringify({ username, displayName: username, email: `${username}@example.test`, password: args.password, ...extra }),
  });
  if (response.status !== 201 && response.status !== 200) throw new Error(`registering ${username}: ${response.status} ${await response.text()}`);
  const login = await fetch(`${base}/api/auth/login`, {
    method: "POST",
    headers: { ...headers, "Content-Type": "application/json" },
    body: JSON.stringify({ username, password: args.password }),
  });
  const body = await login.json();
  names.set(body.id, username);
  return { username, id: body.id, cookie: login.headers.getSetCookie().map((line) => line.split(";")[0]).join("; ") };
}

function invite() {
  if (!args.database) return null;
  const db = new DatabaseSync(args.database);
  const row = db.prepare('SELECT "ServerId", "UserId" FROM "ServerMembers" WHERE "OwnerSlot" = 1 LIMIT 1').get();
  const code = "invite" + randomUUID().replaceAll("-", "").slice(0, 10);
  db.prepare('INSERT INTO "ServerInvites" ("Id", "ServerId", "CreatedById", "ChannelId", "Code", "Uses", "MaxUses", "ExpiresAt", "CreatedAt") VALUES (?, ?, ?, NULL, ?, 0, NULL, NULL, ?)')
    .run(randomUUID().toUpperCase(), row.ServerId, row.UserId, code, new Date().toISOString().replace("T", " ").replace("Z", "0"));
  db.close();
  return code;
}

class Listener {
  constructor(label, person) {
    this.label = label;
    this.person = person;
    this.records = [];
    this.buffer = "";
  }
  async open() {
    this.socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app`, { headers: { Cookie: this.person.cookie } });
    await new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = () => reject(new Error(`the upgrade for ${this.label} failed`));
    });
    this.socket.onmessage = (event) => {
      this.buffer += String(event.data);
      let end;
      while ((end = this.buffer.indexOf(RS)) >= 0) {
        const record = JSON.parse(this.buffer.slice(0, end));
        this.buffer = this.buffer.slice(end + 1);
        if (record.type === 1) this.records.push(`${record.target} ${scrub(record.arguments)}`);
      }
    };
    this.socket.send(`{"protocol":"json","version":1}${RS}`);
    await sleep(300);
    this.records = [];
    return this;
  }
  take() {
    const taken = this.records;
    this.records = [];
    return taken;
  }
}

const boss = await register("boss", { setupCode: args.setup });
const code = invite() ?? (await call(boss, "POST", "/api/admin/invites", {})).body.url.split("/").pop();
const maja = await register("maja", { inviteCode: code });
const joe = await register("joe", { inviteCode: invite() ?? (await call(boss, "POST", "/api/admin/invites", {})).body.url.split("/").pop() });
const listeners = [await new Listener("boss", boss).open(), await new Listener("maja", maja).open(), await new Listener("joe", joe).open()];

async function step(title, action) {
  out(`## ${title}`);
  let result;
  try {
    result = await action();
  } catch (error) {
    result = `threw ${error.message}`;
  }
  if (result !== undefined) out(`   answer: ${scrub(result)}`);
  await sleep(400);
  for (const listener of listeners) {
    const taken = listener.take();
    taken.sort();
    for (const record of taken) out(`   ${listener.label} heard ${record}`);
  }
}

const brief = ({ status, body }) => `${status} ${body === null ? "" : JSON.stringify(body)}`.trim();
const upload = (code, bytes = PNG, type = "image/png") => {
  const form = new FormData();
  form.append("shortcode", code);
  form.append("file", new Blob([bytes], { type }), "e.png");
  return form;
};
const wave = async (person, duration) => {
  const form = new FormData();
  form.append("file", new Blob([WAVE], { type: "audio/wav" }), "s.wav");
  form.append("durationSeconds", String(duration));
  return (await call(person, "POST", "/api/uploads", form)).body.id;
};

let emoji;
await step("a member uploads an emoji", async () => brief(await call(maja, "POST", "/api/emoji", upload("nope"))));
await step("the owner uploads an emoji", async () => {
  const answer = await call(boss, "POST", "/api/emoji", upload("yes"));
  emoji = answer.body.id;
  names.set(emoji, "emoji");
  return brief(answer);
});
await step("the same name again", async () => brief(await call(boss, "POST", "/api/emoji", upload("yes"))));
await step("a bad name", async () => brief(await call(boss, "POST", "/api/emoji", upload("!"))));
await step("a member deletes it", async () => brief(await call(maja, "DELETE", `/api/emoji/${emoji}`)));
await step("the owner deletes it", async () => brief(await call(boss, "DELETE", `/api/emoji/${emoji}`)));
await step("the owner deletes it again", async () => brief(await call(boss, "DELETE", `/api/emoji/${emoji}`)));

const first = await wave(boss, 8);
const second = await wave(boss, 5);
let one, two;
await step("a sound is made", async () => {
  const answer = await call(boss, "POST", "/api/admin/soundboard/sounds", { attachmentId: first, name: "One", glyph: "1", trimStartSeconds: 0, trimEndSeconds: 2 });
  one = answer.body.id;
  names.set(one, "one");
  return brief(answer);
});
await step("a second sound is made", async () => {
  const answer = await call(boss, "POST", "/api/admin/soundboard/sounds", { attachmentId: second, name: "Two", glyph: "2", trimStartSeconds: 1, trimEndSeconds: 3 });
  two = answer.body.id;
  names.set(two, "two");
  return brief(answer);
});
await step("a member makes a sound", async () => brief(await call(maja, "POST", "/api/admin/soundboard/sounds", { attachmentId: await wave(maja, 4), name: "No", glyph: "x", trimStartSeconds: 0, trimEndSeconds: 1 })));
await step("a sound is changed", async () => brief(await call(boss, "PATCH", `/api/admin/soundboard/sounds/${one}`, { name: "Uno" })));
await step("a sound is changed to nothing", async () => brief(await call(boss, "PATCH", `/api/admin/soundboard/sounds/${one}`, {})));
await step("a sound is changed to nonsense", async () => brief(await call(boss, "PATCH", `/api/admin/soundboard/sounds/${one}`, { name: "" })));
await step("the order", async () => brief(await call(boss, "PUT", "/api/admin/soundboard/order", { soundIds: [two, one] })));
await step("the order, incomplete", async () => brief(await call(boss, "PUT", "/api/admin/soundboard/order", { soundIds: [two] })));
await step("the settings", async () => brief(await call(boss, "PATCH", "/api/admin/soundboard/settings", { everyoneCanPlay: false })));
await step("the settings again", async () => brief(await call(boss, "PATCH", "/api/admin/soundboard/settings", { everyoneCanPlay: false })));
await step("the settings by a member", async () => brief(await call(maja, "PATCH", "/api/admin/soundboard/settings", { everyoneCanPlay: true })));
await step("a join sound is chosen", async () => brief(await call(maja, "PATCH", "/api/me/preferences", { joinSoundId: one })));
await step("a sound is deleted", async () => brief(await call(boss, "DELETE", `/api/admin/soundboard/sounds/${one}`)));
await step("a sound that is not there is deleted", async () => brief(await call(boss, "DELETE", `/api/admin/soundboard/sounds/${one}`)));

await step("the profile changes", async () => brief(await call(maja, "PATCH", "/api/me", { displayName: "Maja N", bio: "hi" })));
await step("the profile changes to nothing", async () => brief(await call(maja, "PATCH", "/api/me", {})));
await step("the profile is refused", async () => brief(await call(maja, "PATCH", "/api/me", { displayName: "" })));
await step("presence is hidden", async () => brief(await call(maja, "PATCH", "/api/me/preferences", { showPresence: false })));
await step("presence is hidden again", async () => brief(await call(maja, "PATCH", "/api/me/preferences", { showPresence: false })));
await step("presence is shown", async () => brief(await call(maja, "PATCH", "/api/me/preferences", { showPresence: true })));
await step("another preference", async () => brief(await call(maja, "PATCH", "/api/me/preferences", { developerMode: true })));
await step("a client setting", async () => brief(await call(maja, "PUT", "/api/me/settings/voice", { a: 1 })));

for (const listener of listeners) listener.socket.close(1000);
writeFileSync(args.out, lines.join("\n") + "\n");
process.exit(0);
