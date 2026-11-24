// The servers routes of this server and of the first one, asked the same
// questions on copies of one database file, as transcripts to compare.
//
//   node servers-oracle.mjs seed  --base URL --setup CODE --state state.json
//   node servers-oracle.mjs probe --base URL --state state.json --out transcript.txt --public URL
//
// `seed` builds a history on the first server only, through its own routes:
// eight accounts, servers with roles, sections, a channel hidden from plain
// members, invites, bans, notifications, messages, an emoji and a hundred
// audit lines. `probe` is run on each server, on its own copy of the database
// that `seed` left, and writes what each request answered and what each
// listener heard, with the ids and moments that differ by nature replaced.
// Needs Node 26 (the global WebSocket takes request headers).
//
// See servers-shared-database.sh, which runs both.

import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    base: { type: "string" },
    setup: { type: "string" },
    state: { type: "string" },
    out: { type: "string" },
    public: { type: "string" },
  },
});
const mode = positionals[0];
const base = args.base;
const PASSWORD = "Corona!2026-long";
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** A person's browser: a cookie jar and the web client's headers. */
class Person {
  constructor(name) {
    this.name = name;
    this.cookies = new Map();
    this.server = null;
  }

  async send(method, path, { body, raw, type, headers = {}, form, server } = {}) {
    const request = { method, headers: { ...clientHeaders, ...headers }, redirect: "manual" };
    if (this.cookies.size > 0) request.headers.Cookie = [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
    const active = server === undefined ? this.server : server;
    if (active) request.headers["X-Stellarr-Server"] = active;
    if (form) {
      request.body = form;
    } else if (raw !== undefined) {
      request.headers["Content-Type"] = type ?? "application/json";
      request.body = raw;
    } else if (body !== undefined) {
      request.headers["Content-Type"] = "application/json";
      request.body = JSON.stringify(body);
    }
    const response = await fetch(`${base}${path}`, request);
    for (const line of response.headers.getSetCookie()) {
      const [pair] = line.split(";");
      const at = pair.indexOf("=");
      const value = pair.slice(at + 1);
      if (value === "") this.cookies.delete(pair.slice(0, at));
      else this.cookies.set(pair.slice(0, at), value);
    }
    const text = await response.text();
    let json = null;
    try {
      json = JSON.parse(text);
    } catch {}
    return { status: response.status, headers: response.headers, text, json };
  }

  get(path, options) { return this.send("GET", path, options); }
  post(path, body, options) { return this.send("POST", path, { ...options, body }); }
  put(path, body, options) { return this.send("PUT", path, { ...options, body }); }
  patch(path, body, options) { return this.send("PATCH", path, { ...options, body }); }
  delete(path, options) { return this.send("DELETE", path, options); }

  async login() {
    const response = await this.post("/api/auth/login", { username: this.name, password: PASSWORD });
    if (response.status !== 200) throw new Error(`signing in ${this.name}: ${response.status} ${response.text}`);
    this.id = response.json.id ?? response.json.user?.id;
    return this;
  }
}

function must(response, status, what) {
  if (response.status !== status && !(status < 300 && response.status < 300 && response.status >= 200)) throw new Error(`${what}: ${response.status} ${response.text}`);
  return response.json;
}

// ---------------------------------------------------------------------------

const PEOPLE = ["owner", "maya", "ada", "joe", "nils", "ilya", "kim", "zed"];

async function seed() {
  const state = { people: {}, servers: {}, invites: {}, channels: {}, roles: {} };
  const person = Object.fromEntries(PEOPLE.map((name) => [name, new Person(name)]));
  const owner = person.owner;
  must(await owner.post("/api/auth/register", {
    username: "owner", displayName: "Owner", email: "owner@example.test", password: PASSWORD, setupCode: args.setup,
  }), 201, "founding");
  await owner.login();
  const instanceInvite = async () => {
    const invite = must(await owner.post("/api/admin/invites"), 201, "an invite");
    return invite.url.split("/").pop();
  };
  for (const name of PEOPLE.slice(1)) {
    const p = person[name];
    must(await p.post("/api/auth/register", {
      username: name, displayName: name[0].toUpperCase() + name.slice(1), email: `${name}@example.test`,
      password: PASSWORD, inviteCode: await instanceInvite(),
    }), 201, `registering ${name}`);
    await p.login();
  }
  for (const name of PEOPLE) state.people[name] = person[name].id;
  const { maya, ada, joe, nils, ilya, kim, zed } = person;

  // Orbit: Maya's server, with roles, a section, a hidden channel and a long history.
  const orbit = must(await maya.post("/api/servers", { name: "Orbit", description: "Our place", textChannelName: "general", voiceChannelName: "hall" }), 201, "Orbit");
  state.servers.orbit = orbit.id;
  maya.server = orbit.id;
  const mkRole = async (name, permissions) => must(await maya.post("/api/admin/roles", { name, color: "#3399ff", permissions }), 201, name);
  const auditor = await mkRole("Auditor", ["viewAuditLog"]);
  const manager = await mkRole("Manager", ["manageServer"]);
  const inviter = await mkRole("Inviter", ["createInvite"]);
  const lead = await mkRole("Lead", ["manageServer"]);
  state.roles = { auditor: auditor.id, manager: manager.id, inviter: inviter.id, lead: lead.id };
  const channelsOf = async () => must(await maya.get("/api/bootstrap"), 200, "bootstrap");
  const boot = await channelsOf();
  const orbitChannels = boot.channels ?? boot.server?.channels ?? [];
  for (const c of orbitChannels) state.channels[`orbit-${c.name}`] = c.id;
  const staff = must(await maya.post("/api/admin/channels", { name: "staff", kind: "text" }), 201, "staff");
  state.channels["orbit-staff"] = staff.id;
  const section = must(await maya.post("/api/admin/sections", { name: "Extra" }), 201, "section");
  must(await maya.post("/api/admin/channels", { name: "memes", kind: "text", sectionId: section.id }), 201, "memes");
  const roles = must(await maya.get("/api/admin/roles"), 200, "roles");
  const base = roles.find((r) => r.isSystem && !r.isOwner);
  state.roles.base = base.id;
  // Plain members may not see #staff; the Manager and Inviter roles may.
  must(await maya.put(`/api/admin/channels/${staff.id}/permissions/${base.id}`, { allow: [], deny: ["viewChannel"] }), 200, "hide staff");
  must(await maya.put(`/api/admin/channels/${staff.id}/permissions/${manager.id}`, { allow: ["viewChannel"], deny: [] }), 200, "show staff");

  const invite = async (p, server, body) => {
    const made = must(await p.post("/api/admin/invites", body, { server }), 201, "invite");
    return { id: made.id, code: made.url.split("/").pop() };
  };
  const join = async (p, code) => must(await p.post(`/api/invites/${code}/join`), 200, `joining ${p.name}`);
  const open = await invite(maya, orbit.id, undefined);
  for (const p of [ada, joe, nils, ilya]) await join(p, open.code);
  must(await maya.put(`/api/admin/members/${ada.id}/roles`, { roleIds: [auditor.id] }), 200, "ada");
  must(await maya.put(`/api/admin/members/${joe.id}/roles`, { roleIds: [manager.id] }), 200, "joe");
  must(await maya.put(`/api/admin/members/${nils.id}/roles`, { roleIds: [inviter.id] }), 200, "nils");
  // Ilya manages the server but, unlike the Manager role, hers is no reason to see #staff.
  must(await maya.put(`/api/admin/members/${ilya.id}/roles`, { roleIds: [lead.id] }), 200, "ilya");
  // Zed joined, misbehaved and was banned.
  await join(zed, open.code);
  must(await maya.post(`/api/admin/members/${zed.id}/ban`, { reason: "Spam" }), 201, "ban zed");
  state.invites.open = open;
  state.invites.toStaff = await invite(maya, orbit.id, { channelId: staff.id });
  state.invites.toGeneral = await invite(maya, orbit.id, { channelId: state.channels["orbit-general"] });
  state.invites.revoked = await invite(maya, orbit.id, undefined);
  must(await maya.delete(`/api/admin/invites/${state.invites.revoked.id}`), 204, "revoke");
  state.invites.sent = await invite(maya, orbit.id, undefined);
  must(await maya.post(`/api/admin/invites/${state.invites.sent.id}/members/${kim.id}`), 204, "send to kim");
  // A history longer than a page of the audit log.
  for (let i = 0; i < 30; i++) {
    must(await maya.patch("/api/admin/server", { name: i % 2 ? "Orbit" : "Orbit II", description: `Round ${i}` }), 200, "rename");
    const c = must(await maya.post("/api/admin/channels", { name: `scratch-${i}`, kind: "text" }), 201, "scratch");
    must(await maya.delete(`/api/admin/channels/${c.id}`), 204, "delete scratch");
  }
  must(await maya.patch("/api/admin/server", { name: "Orbit", description: "Our place", isPublic: false }), 200, "reset");

  // Open: a public server of Ada's.
  const pub = must(await ada.post("/api/servers", { name: "Open House", isPublic: true }), 201, "Open House");
  state.servers.open = pub.id;
  const closed = must(await ada.post("/api/servers", { name: "Closed Door", isPublic: false }), 201, "Closed Door");
  state.servers.closed = closed.id;
  must(await ada.post(`/api/admin/members/${zed.id}/ban`, { reason: "No" }, { server: pub.id }), 404, "ban a stranger");

  // Doomed servers: content in every table the deletion has to clear.
  const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="), (c) => c.charCodeAt(0));
  const file = () => { const f = new FormData(); f.append("file", new Blob([png], { type: "image/png" }), "pixel.png"); return f; };
  for (const [key, host, extra] of [["doomed", kim, { name: "Doomed", textChannelName: "talk" }], ["handover", kim, { name: "Handover", textChannelName: "talk" }]]) {
    const s = must(await host.post("/api/servers", extra), 201, key);
    state.servers[key] = s.id;
    host.server = s.id;
    const code = (await invite(host, s.id, undefined)).code;
    for (const p of [ilya, joe]) await join(p, code);
    const channel = (must(await host.get("/api/bootstrap"), 200, "bootstrap").channels ?? []).find((c) => c.name === "talk");
    state.channels[`${key}-talk`] = channel.id;
    const upload = must(await host.post("/api/uploads", undefined, { form: file(), server: s.id }), 201, "upload");
    const message = must(await host.post(`/api/channels/${channel.id}/messages`, { content: `hello ${key}word https://example.invalid/page`, attachmentIds: [upload.id] }), 201, "message");
    must(await ilya.post(`/api/channels/${channel.id}/messages`, { content: `reply ${key}word`, replyToMessageId: message.id }, { server: s.id }), 201, "reply");
    must(await joe.put(`/api/messages/${message.id}/reactions/${encodeURIComponent("👍")}`, undefined, { server: s.id }), 204, "react");
    must(await host.put(`/api/messages/${message.id}/pin`, undefined, { server: s.id }), 200, "pin");
    const emoji = new FormData();
    emoji.append("shortcode", "pixel");
    emoji.append("file", new Blob([png], { type: "image/png" }), "pixel.png");
    must(await host.post("/api/emoji", undefined, { form: emoji, server: s.id }), 201, "emoji");
    must(await host.post("/api/admin/server/icon", undefined, { form: file(), server: s.id }), 201, "icon");
    await host.post(`/api/admin/invites/${(await invite(host, s.id, undefined)).id}/members/${nils.id}`, undefined, { server: s.id });
  }
  writeFileSync(args.state, JSON.stringify(state, null, 2));
}

// ---------------------------------------------------------------------------
// What each server answers.

const lines = [];
const out = (text) => lines.push(text);
let state;
const names = new Map();

function nameIds() {
  for (const [name, id] of Object.entries(state.people)) names.set(id, `person:${name}`);
  for (const [name, id] of Object.entries(state.servers)) names.set(id, `server:${name}`);
  for (const [name, id] of Object.entries(state.channels)) names.set(id, `channel:${name}`);
  for (const [name, id] of Object.entries(state.roles)) names.set(id, `role:${name}`);
  for (const [name, invite] of Object.entries(state.invites)) {
    names.set(invite.id, `invite:${name}`);
    names.set(invite.code, `code:${name}`);
  }
}

let fresh = 0;
/** Ids and moments that differ by nature are replaced; the rest is kept. */
function scrub(text) {
  let result = text;
  // Ids the scenario knew before it ran keep their names; new ones are numbered in order of appearance.
  result = result.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, (id) => {
    const key = id.toLowerCase();
    if (!names.has(key)) names.set(key, `new${++fresh}`);
    return `<${names.get(key)}>`;
  });
  result = result.replace(/\b[0-9a-f]{16}\b/g, (code) => `<${names.get(code) ?? "code"}>`);
  result = result.replace(/(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(\.\d+)?(Z?)/g, (_, _a, _b, z) => `<time${z}>`);
  result = result.replace(/"traceId":"[^"]*"/g, '"traceId":"<trace>"');
  result = result.replace(/oklch\(0\.58 0\.14 \d+\)/g, "oklch(<hue>)");
  if (args.public) result = result.split(args.public).join("<origin>");
  return result;
}

function describe(response) {
  const lines = [String(response.status)];
  for (const name of ["content-type", "cache-control", "location", "allow", "retry-after"]) {
    const value = response.headers.get(name);
    if (value) lines.push(`${name}: ${value}`);
  }
  lines.push(`BODY: ${response.text}`);
  return scrub(lines.join(" | "));
}

/** One hub connection of a person, keeping the events of interest it hears. */
class Listener {
  constructor(person, query = "") {
    this.person = person;
    this.query = query;
    this.heard = [];
    this.buffer = "";
  }

  async open() {
    const cookie = [...this.person.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
    this.socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app${this.query}`, { headers: { Cookie: cookie, ...clientHeaders } });
    await new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = () => reject(new Error(`the hub refused ${this.person.name}`));
    });
    this.socket.onmessage = (event) => {
      this.buffer += String(event.data);
      let end;
      while ((end = this.buffer.indexOf(RS)) >= 0) {
        const text = this.buffer.slice(0, end);
        this.buffer = this.buffer.slice(end + 1);
        if (text === "{}") continue;
        const record = JSON.parse(text);
        if (record.type === 1 && WANTED.has(record.target)) this.heard.push(`${record.target} ${JSON.stringify(record.arguments[0])}`);
      }
    };
    this.socket.send(`{"protocol":"json","version":1}${RS}`);
    return this;
  }

  take() {
    const heard = this.heard;
    this.heard = [];
    return heard;
  }
}

const WANTED = new Set(["ServerUpdated", "NotificationInboxChanged", "ChannelAccessChanged", "MessageCreated"]);

async function probe() {
  state = JSON.parse(readFileSync(args.state, "utf8"));
  nameIds();
  const who = Object.fromEntries(PEOPLE.map((name) => [name, new Person(name)]));
  for (const person of Object.values(who)) await person.login();
  const anonymous = new Person("anonymous");
  const listeners = {};
  for (const name of ["owner", "maya", "ada", "joe", "nils", "ilya", "kim", "zed"]) {
    listeners[name] = await new Listener(who[name], `?serverId=${state.servers.orbit}`).open();
  }
  await sleep(500);
  for (const listener of Object.values(listeners)) listener.take();
  const s = state.servers;
  const c = state.channels;
  const code = (name) => state.invites[name].code;
  const iid = (name) => state.invites[name].id;
  const P = (name) => state.people[name];
  const NIL = "00000000-0000-0000-0000-000000000000";

  async function step(label, person, method, path, options = {}) {
    const response = await person.send(method, path, options);
    // A listener hears what a request caused a little after the answer.
    await sleep(120);
    const heard = Object.entries(listeners).flatMap(([name, listener]) => listener.take().map((event) => `  ${name} <- ${scrub(event)}`)).sort();
    out(`## ${label}`);
    out(describe(response));
    for (const line of heard) out(line);
    return response;
  }

  // ----- the lists
  await step("list: maya", who.maya, "GET", "/api/servers", { server: null });
  await step("list: ilya", who.ilya, "GET", "/api/servers", { server: null });
  await step("list: anonymous", anonymous, "GET", "/api/servers");
  await step("discover: ilya", who.ilya, "GET", "/api/servers/discover");
  await step("discover: ada", who.ada, "GET", "/api/servers/discover");
  await step("discover: anonymous", anonymous, "GET", "/api/servers/discover");

  // ----- making a server
  const make = (label, person, body, options = {}) => step(`create: ${label}`, person, "POST", "/api/servers", { body, server: null, ...options });
  await make("minimal", who.zed, { name: "Zed's" });
  await make("everything", who.zed, { name: "  Zed Two  ", description: " Hello ", isPublic: true, textChannelName: " Lobby ", voiceChannelName: "Room-1", mentionsOnly: false, voiceJoinSound: false });
  await make("no voice", who.zed, { name: "Zed Three", voiceChannelName: "" });
  await make("voice name only", who.zed, { name: "Zed Four", voiceChannelName: "hall" });
  await make("nameless", who.zed, { name: "   " });
  await make("no name", who.zed, {});
  await make("long name", who.zed, { name: "n".repeat(81) });
  await make("longest name", who.zed, { name: "n".repeat(80) });
  await make("long description", who.zed, { name: "x", description: "d".repeat(281) });
  await make("clashing channels", who.zed, { name: "x", textChannelName: "same", voiceChannelName: "SAME" });
  await make("bad text channel", who.zed, { name: "x", textChannelName: "two words" });
  await make("bad voice channel", who.zed, { name: "x", voiceChannelName: "two words" });
  await make("long channel name", who.zed, { name: "x", textChannelName: "c".repeat(61) });
  await make("unicode channel names", who.zed, { name: "Zażółć", textChannelName: "ÓGŁÓWNY", voiceChannelName: "GŁOS-1" });
  await make("nulls", who.zed, { name: "Nulls", description: null, textChannelName: null, voiceChannelName: null, mentionsOnly: null, voiceJoinSound: null });
  await make("is public null", who.zed, { name: "x", isPublic: null });
  await make("name number", who.zed, { name: 5 });
  await make("extra members", who.zed, { name: "Extra", surprise: true });
  for (const [label, raw, type] of [["empty body", "", "application/json"], ["broken json", "{", "application/json"], ["null body", "null", "application/json"], ["array body", "[]", "application/json"], ["text body", "{}", "text/plain"], ["form body", "name=x", "application/x-www-form-urlencoded"]]) {
    await step(`create: ${label}`, who.zed, "POST", "/api/servers", { raw, type, server: null });
  }
  await step("create: no body or type", who.zed, "POST", "/api/servers", { server: null });
  await step("create: anonymous", anonymous, "POST", "/api/servers", { body: { name: "x" } });
  await step("discover: after creating", who.ilya, "GET", "/api/servers/discover");

  // ----- joining a public server
  await step("join public: ilya", who.ilya, "POST", `/api/servers/${s.open}/join`);
  await step("join public: ilya again", who.ilya, "POST", `/api/servers/${s.open}/join`);
  await step("join public: closed server", who.ilya, "POST", `/api/servers/${s.closed}/join`);
  await step("join public: unknown", who.ilya, "POST", `/api/servers/${NIL}/join`);
  await step("join public: not an id", who.ilya, "POST", "/api/servers/not-an-id/join");
  await step("join public: upper case id", who.joe, "POST", `/api/servers/${s.open.toUpperCase()}/join`);
  await step("join public: braced id", who.nils, "POST", `/api/servers/{${s.open}}/join`);
  await step("join public: unhyphenated id", who.kim, "POST", `/api/servers/${s.open.replaceAll("-", "")}/join`);
  await step("join public: anonymous", anonymous, "POST", `/api/servers/${s.open}/join`);
  await step("discover: after joining", who.ilya, "GET", "/api/servers/discover");
  await step("list: ilya after joining", who.ilya, "GET", "/api/servers", { server: null });

  // ----- the page an invite link opens
  for (const name of ["open", "toStaff", "toGeneral", "revoked", "sent"]) {
    await step(`preview ${name}: anonymous`, anonymous, "GET", `/api/invites/${code(name)}`);
  }
  for (const [name, person] of [["toStaff", "maya"], ["toStaff", "kim"], ["toStaff", "ilya"], ["toGeneral", "kim"], ["open", "zed"]]) {
    await step(`preview ${name}: ${person}`, who[person], "GET", `/api/invites/${code(name)}`);
  }
  await step("preview: unknown", anonymous, "GET", "/api/invites/0123456789abcdef");
  await step("preview: upper case", anonymous, "GET", `/api/invites/${code("open").toUpperCase()}`);
  await step("preview: spaces", anonymous, "GET", `/api/invites/%20${code("open")}%20`);
  await step("preview: long code", anonymous, "GET", `/api/invites/${"a".repeat(300)}`);
  await step("preview: odd characters", anonymous, "GET", "/api/invites/%C4%B0%C4%B1%E1%BA%9E");

  // ----- joining through an invite
  const join = (label, person, name) => step(`join invite ${label}`, person, "POST", `/api/invites/${name}/join`);
  await join("staff: kim", who.kim, code("toStaff"));
  await join("staff: kim again", who.kim, code("toStaff"));
  await join("general: owner", who.owner, code("toGeneral"));
  await join("open: zed (banned)", who.zed, code("open"));
  await join("revoked: ilya", who.ilya, code("revoked"));
  await join("unknown: ilya", who.ilya, "no-such-code");
  await join("upper case: owner", who.owner, code("open").toUpperCase());
  await join("anonymous", anonymous, code("open"));
  await step("join invite: with a body", who.owner, "POST", `/api/invites/${code("open")}/join`, { body: { x: 1 } });
  await step("list: kim after joining", who.kim, "GET", "/api/servers", { server: null });

  // ----- the invites of a server
  const orbit = s.orbit;
  for (const name of ["maya", "joe", "ilya", "ada", "nils", "kim", "zed"]) {
    await step(`invites list: ${name}`, who[name], "GET", "/api/admin/invites", { server: orbit });
  }
  const create = (label, person, body, options = {}) => step(`invite create: ${label}`, person, "POST", "/api/admin/invites", { body, server: orbit, ...options });
  await create("maya, no body", who.maya, undefined);
  await create("maya, empty object", who.maya, {});
  await create("maya, to general", who.maya, { channelId: c["orbit-general"] });
  await create("maya, to staff", who.maya, { channelId: c["orbit-staff"] });
  await create("nils, server-wide", who.nils, {});
  await create("nils, to general", who.nils, { channelId: c["orbit-general"] });
  await create("nils, to staff (cannot see it)", who.nils, { channelId: c["orbit-staff"] });
  await create("joe, to staff", who.joe, { channelId: c["orbit-staff"] });
  await create("ada, server-wide", who.ada, {});
  await create("ilya (lead)", who.ilya, {});
  await create("maya, other server's channel", who.maya, { channelId: c["doomed-talk"] });
  await create("maya, unknown channel", who.maya, { channelId: NIL });
  await create("maya, channelId null", who.maya, { channelId: null });
  await create("maya, channelId a number", who.maya, { channelId: 5 });
  await create("maya, channelId text", who.maya, { channelId: "x" });
  await create("maya, upper case channelId", who.maya, { channelId: c["orbit-general"].toUpperCase() });
  await create("maya, extra members", who.maya, { channelId: null, more: 1 });
  for (const [label, raw, type] of [["null body", "null", "application/json"], ["empty json body", "", "application/json"], ["broken json", "{", "application/json"], ["array", "[]", "application/json"], ["text body", "{}", "text/plain"], ["whitespace", " ", "application/json"]]) {
    await step(`invite create: ${label}`, who.maya, "POST", "/api/admin/invites", { raw, type, server: orbit });
  }
  await step("invite create: not a member", who.kim, "POST", "/api/admin/invites", { server: s.closed });
  await step("invite create: no such server header", who.maya, "POST", "/api/admin/invites", { server: "not-an-id" });
  await step("invite create: unknown server", who.maya, "POST", "/api/admin/invites", { server: NIL });
  await step("invite create: server in query", who.maya, "POST", `/api/admin/invites?serverId=${orbit}`, { server: null });
  await step("invite create: empty server header", who.maya, "POST", "/api/admin/invites", { server: "" });
  await step("invites list: maya after", who.maya, "GET", "/api/admin/invites", { server: orbit });
  await step("invites list: nils after", who.nils, "GET", "/api/admin/invites", { server: orbit });

  // ----- sending and revoking
  const send = (label, person, invite, user, options = {}) => step(`invite send: ${label}`, person, "POST", `/api/admin/invites/${invite}/members/${user}`, { server: orbit, ...options });
  await send("to ilya (a member)", who.maya, iid("open"), P("ilya"));
  await send("to zed (banned)", who.maya, iid("open"), P("zed"));
  await send("to an unknown person", who.maya, iid("open"), NIL);
  await send("with an unknown invite", who.maya, NIL, P("kim"));
  await send("to kim", who.maya, iid("open"), P("kim"));
  await send("to kim again", who.maya, iid("open"), P("kim"));
  await send("to owner", who.joe, iid("toGeneral"), P("owner"));
  await send("by ada (no permission)", who.ada, iid("open"), P("owner"));
  await send("revoked invite", who.maya, iid("revoked"), P("owner"));
  await send("not ids", who.maya, "x", "y");
  await send("upper case ids", who.maya, iid("toStaff").toUpperCase(), P("zed").toUpperCase());
  await step("revoke: unknown", who.maya, "DELETE", `/api/admin/invites/${NIL}`, { server: orbit });
  await step("revoke: not an id", who.maya, "DELETE", "/api/admin/invites/x", { server: orbit });
  await step("revoke: ada", who.ada, "DELETE", `/api/admin/invites/${iid("open")}`, { server: orbit });
  await step("revoke: joe", who.joe, "DELETE", `/api/admin/invites/${iid("toGeneral")}`, { server: orbit });
  await step("revoke: invite of another server", who.maya, "DELETE", `/api/admin/invites/${iid("open")}`, { server: s.closed });
  await step("revoke: open", who.maya, "DELETE", `/api/admin/invites/${iid("open")}`, { server: orbit });
  await step("revoke: open again", who.maya, "DELETE", `/api/admin/invites/${iid("open")}`, { server: orbit });
  await step("preview revoked", anonymous, "GET", `/api/invites/${code("open")}`);

  // ----- the audit log
  for (const name of ["maya", "ada", "joe", "nils", "ilya", "kim"]) {
    await step(`audit: ${name}`, who[name], "GET", "/api/admin/audit", { server: orbit });
  }
  const first = await who.maya.get("/api/admin/audit", { server: orbit });
  const entries = first.json;
  const at = entries[entries.length - 1].at;
  out(`## the oldest of the first page is at <${scrub(at)}>`);
  const beforeSpellings = [
    ["as given", at], ["with Z", at.endsWith("Z") ? at : `${at}Z`], ["without Z", at.replace(/Z$/, "")], ["date only", at.slice(0, 10)],
    ["offset", at.replace(/Z$/, "") + "+02:00"], ["negative offset", at.replace(/Z$/, "") + "-0100"], ["space", at.replace("T", " ")], ["empty", ""], ["garbage", "yesterday"],
    ["month name", "Nov 24 2026"], ["slashes", "11/24/2026"], ["long fraction", "2026-11-24T10:00:00.123456789Z"], ["seven digits", "2026-11-24T10:00:00.1234567Z"], ["far future", "2999-01-01T00:00:00Z"], ["far past", "1999-01-01T00:00:00Z"], ["epoch", "0"], ["only time", "10:00:00"],
  ];
  for (const [label, value] of beforeSpellings) {
    await step(`audit before ${label}`, who.maya, "GET", `/api/admin/audit?before=${encodeURIComponent(value)}`, { server: orbit });
  }
  await step("audit before twice", who.maya, "GET", `/api/admin/audit?before=${encodeURIComponent(at)}&before=1999-01-01`, { server: orbit });
  await step("audit other query", who.maya, "GET", "/api/admin/audit?limit=2", { server: orbit });
  await step("audit: ada, other server", who.ada, "GET", "/api/admin/audit", { server: s.open });

  // ----- notification defaults
  const note = (label, person, body) => step(`notifications patch: ${label}`, person, "PATCH", "/api/admin/notifications", { body, server: orbit });
  for (const name of ["maya", "joe", "ilya", "ada", "nils", "kim"]) {
    await step(`notifications get: ${name}`, who[name], "GET", "/api/admin/notifications", { server: orbit });
  }
  await note("everything", who.maya, { allMessages: true, mentionsOnly: false, voiceJoinSound: false, serverMuted: true, mutedChannelIds: [c["orbit-general"], c["orbit-general"], c["orbit-staff"]] });
  await step("notifications get: maya after", who.maya, "GET", "/api/admin/notifications", { server: orbit });
  await step("notifications get: ilya (cannot see staff)", who.ilya, "GET", "/api/admin/notifications", { server: orbit });
  await step("notifications get: joe after", who.joe, "GET", "/api/admin/notifications", { server: orbit });
  await note("ilya names general", who.ilya, { mutedChannelIds: [c["orbit-hall"]] });
  await step("notifications get: maya after ilya", who.maya, "GET", "/api/admin/notifications", { server: orbit });
  await note("ilya names staff", who.ilya, { mutedChannelIds: [c["orbit-staff"]] });
  await note("ilya names none", who.ilya, { mutedChannelIds: [] });
  await step("notifications get: maya after ilya's none", who.maya, "GET", "/api/admin/notifications", { server: orbit });
  await note("empty object", who.maya, {});
  await note("foreign channel", who.maya, { mutedChannelIds: [c["doomed-talk"]] });
  await note("unknown channel", who.maya, { mutedChannelIds: [NIL] });
  await note("not an id", who.maya, { mutedChannelIds: ["x"] });
  await note("upper case ids", who.maya, { mutedChannelIds: [c["orbit-hall"].toUpperCase()] });
  await note("null list", who.maya, { mutedChannelIds: null });
  await note("booleans null", who.maya, { allMessages: null, serverMuted: null });
  await note("wrong type", who.maya, { allMessages: "yes" });
  await note("ada", who.ada, { allMessages: true });
  await note("clear", who.maya, { mutedChannelIds: [], allMessages: false, mentionsOnly: true, voiceJoinSound: true, serverMuted: false });
  await step("notifications: a server with none", who.kim, "GET", "/api/admin/notifications", { server: s.doomed });

  // ----- the profile
  const profile = (label, person, body, options = {}) => step(`profile: ${label}`, person, "PATCH", "/api/admin/server", { body, server: orbit, ...options });
  await profile("name and description", who.maya, { name: "  Orbit 3  ", description: " Hi " });
  await profile("nothing", who.maya, {});
  await profile("same values", who.maya, { name: "Orbit 3", description: "Hi" });
  await profile("make public", who.joe, { isPublic: true });
  await profile("make public again", who.joe, { isPublic: true });
  await profile("invite only", who.joe, { isPublic: false });
  await profile("empty name", who.maya, { name: "" });
  await profile("blank name", who.maya, { name: "   " });
  await profile("long name", who.maya, { name: "n".repeat(81) });
  await profile("long description", who.maya, { description: "d".repeat(281) });
  await profile("empty description", who.maya, { description: "" });
  await profile("null members", who.maya, { name: null, description: null, isPublic: null });
  await profile("wrong type", who.maya, { isPublic: "yes" });
  await profile("ada", who.ada, { name: "Mine" });
  await profile("nils", who.nils, { name: "Mine" });
  await profile("kim (no member)", who.kim, { name: "Mine" });
  await profile("anonymous", anonymous, { name: "Mine" });
  await profile("bad server header", who.maya, { name: "x" }, { server: "x" });
  await profile("unknown server", who.maya, { name: "x" }, { server: NIL });
  await profile("empty body", who.maya, undefined, { raw: "", type: "application/json" });
  await profile("broken body", who.maya, undefined, { raw: "{", type: "application/json" });
  await profile("text body", who.maya, undefined, { raw: "{}", type: "text/plain" });
  await profile("put back", who.maya, { name: "Orbit", description: "Our place", isPublic: false });

  // ----- handing a server over
  const transfer = (label, person, body, options = {}) => step(`transfer: ${label}`, person, "POST", "/api/admin/server/transfer", { body, server: orbit, ...options });
  await transfer("by joe (not the owner)", who.joe, { userId: P("joe") });
  await transfer("by ada", who.ada, { userId: P("ada") });
  await transfer("by kim (no member)", who.kim, { userId: P("kim") });
  await transfer("to herself", who.maya, { userId: P("maya") });
  await transfer("to a stranger who was banned", who.maya, { userId: P("zed") });
  await transfer("to nobody", who.maya, {});
  await transfer("to the empty id", who.maya, { userId: NIL });
  await transfer("to null", who.maya, { userId: null });
  await transfer("to an unknown id", who.maya, { userId: "6f9a2f60-0000-4000-8000-000000000000" });
  await transfer("to a number", who.maya, { userId: 5 });
  await transfer("to a word", who.maya, { userId: "joe" });
  await transfer("extra members", who.maya, { userId: NIL, more: 1 });
  for (const [label, raw, type] of [["empty body", "", "application/json"], ["broken json", "{", "application/json"], ["null body", "null", "application/json"], ["array", "[]", "application/json"], ["text body", "{}", "text/plain"]]) {
    await step(`transfer: ${label}`, who.maya, "POST", "/api/admin/server/transfer", { raw, type, server: orbit });
  }
  await transfer("anonymous", anonymous, { userId: P("joe") });
  await transfer("bad server header", who.maya, { userId: P("joe") }, { server: "x" });
  await step("transfer: by a member of another server (header)", who.ilya, "POST", "/api/admin/server/transfer", { body: { userId: P("kim") }, server: s.handover });
  await transfer("to joe (the Manager)", who.maya, { userId: P("joe") });
  await step("maya may not delete any more", who.maya, "DELETE", "/api/admin/server", { server: orbit });
  await step("maya may not hand over again", who.maya, "POST", "/api/admin/server/transfer", { body: { userId: P("maya") }, server: orbit });
  await step("audit: after the hand-over", who.joe, "GET", "/api/admin/audit", { server: orbit });
  await step("hand over a server to its first member", who.kim, "POST", "/api/admin/server/transfer", { body: { userId: P("ilya") }, server: s.handover });
  await step("notifications get: new owner", who.ilya, "GET", "/api/admin/notifications", { server: s.handover });

  // ----- deleting
  const remove = (label, person, server) => step(`delete: ${label}`, person, "DELETE", "/api/admin/server", { server });
  await remove("by ilya (not the owner)", who.ilya, s.doomed);
  await remove("by joe (not the owner)", who.joe, s.doomed);
  await remove("by kim (the former owner of the handed over server)", who.kim, s.handover);
  await remove("the instance server by its owner", who.owner, undefined);
  await remove("the instance server by its owner, named", who.owner, (await who.owner.get("/api/servers", { server: null })).json[0].id);
  await remove("anonymous", anonymous, s.doomed);
  await remove("bad server header", who.kim, "x");
  await remove("the doomed server", who.kim, s.doomed);
  await remove("the doomed server again", who.kim, s.doomed);
  await remove("the handed over server by its new owner", who.ilya, s.handover);
  await step("preview of an invite to a deleted server", anonymous, "GET", `/api/invites/${code("sent")}`);
  await step("list: kim after deleting", who.kim, "GET", "/api/servers", { server: null });
  await step("join public: a deleted server", who.joe, "POST", `/api/servers/${s.doomed}/join`);

  writeFileSync(args.out, lines.join("\n") + "\n");
  for (const listener of Object.values(listeners)) listener.socket.close();
  process.exit(0);
}

if (mode === "seed") await seed();
if (mode === "probe") await probe();
