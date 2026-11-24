// The operator panel (`/api/cockpit`) of this server and of the first one,
// asked the same questions on copies of one database file, as transcripts to
// compare.
//
//   node cockpit-oracle.mjs seed        --base URL --setup CODE --state state.json
//   node cockpit-oracle.mjs prepare     --db seeded.db --state state.json
//   node cockpit-oracle.mjs probe       --base URL --state state.json --seeded seeded.db --out transcript.txt
//                                       --public URL --smtp-host HOST --smtp-port PORT --smtp-closed PORT --smtp-refusing PORT
//   node cockpit-oracle.mjs stale-login --base URL --out jars.json
//   node cockpit-oracle.mjs stale-edit  --db copy.db
//   node cockpit-oracle.mjs stale-ask   --base URL --state state.json --seeded seeded.db --cookies jars.json --out transcript.txt --public URL
//   node cockpit-oracle.mjs mails       --first FOLDER --this FOLDER
//   node cockpit-oracle.mjs compare     --first transcript.txt --this transcript.txt
//   node cockpit-oracle.mjs smtp-refuse --port PORT
//
// `seed` builds a history on the first server only, through its own routes:
// twelve accounts, roles of the instance made in the panel, servers of other
// people, content the overview counts, sessions from the desktop app and a
// disabled account. `prepare` adds to the seeded file what no route of either
// server makes without an outside provider: identity links, second-step
// methods, a trusted device and recovery codes. `probe` is run on each
// server, on its own copy of the database, and writes what each request
// answered and what each hub listener heard, with what differs by nature
// replaced. `stale-login`, `stale-edit` and `stale-ask` do the same for
// sessions whose second step is fresh, old or missing, and for the policies
// that ask the panel for one. `mails` compares what the two servers sent to
// the fake mail servers, `compare` two transcripts step by step, and
// `smtp-refuse` is a mail server that refuses everything. Needs Node 26
// (the global WebSocket takes request headers, and `node:sqlite`).
//
// See cockpit-shared-database.sh, which runs it all.

import { readFileSync, writeFileSync, readdirSync } from "node:fs";
import { createHmac } from "node:crypto";
import { createServer } from "node:net";
import { DatabaseSync } from "node:sqlite";
import { parseArgs } from "node:util";

const { values: args, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    base: { type: "string" },
    setup: { type: "string" },
    state: { type: "string" },
    seeded: { type: "string" },
    out: { type: "string" },
    public: { type: "string" },
    cookies: { type: "string" },
    db: { type: "string" },
    first: { type: "string" },
    this: { type: "string" },
    port: { type: "string" },
    "smtp-host": { type: "string" },
    "smtp-port": { type: "string" },
    "smtp-closed": { type: "string" },
    "smtp-refusing": { type: "string" },
  },
});
const mode = positionals[0];
const base = args.base;
const PASSWORD = "Corona!2026-long";
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
const RS = "\u001e";
const NIL = "00000000-0000-0000-0000-000000000000";
const UNKNOWN = "6f9a2f60-0000-4000-8000-000000000000";
const DESKTOP = "Mozilla/5.0 StellarrDesktop/1.4.2 CEF/131.0.6778.86 Package/msi";
// What must never come back from either server once it was sent.
const WRONG_PASSWORD = "Wrong-Pass-COCKPIT-1234";
const SECRET_ONE = "S3cret-SMTP-COCKPIT-4567";
const SECRET_TWO = "Second-S3cret-COCKPIT-89";
const SECRET_PADDED = "  padded-COCKPIT-secret  ";
const RESEND_KEY = "re_COCKPIT_resend_key_0001";
const QUERY_TOKEN = "TOKEN-IN-QUERY-COCKPIT-777";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// Every answer of every server, in order, to look for secrets in afterwards.
const answers = [];

/** A person's browser: a cookie jar, the web client's headers and a user agent. */
class Person {
  constructor(name, agent) {
    this.name = name;
    this.agent = agent;
    this.cookies = new Map();
    this.server = null;
  }

  async send(method, path, { body, raw, type, headers = {}, form, server } = {}) {
    const request = { method, headers: { ...clientHeaders, ...headers }, redirect: "manual" };
    if (this.agent) request.headers["User-Agent"] = this.agent;
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
    const cookies = response.headers.getSetCookie();
    for (const line of this.fixedJar ? [] : cookies) {
      const [pair] = line.split(";");
      const at = pair.indexOf("=");
      const value = pair.slice(at + 1);
      if (value === "") this.cookies.delete(pair.slice(0, at));
      else this.cookies.set(pair.slice(0, at), value);
    }
    const text = await response.text();
    answers.push(text);
    let json = null;
    try {
      json = JSON.parse(text);
    } catch {}
    return { status: response.status, headers: response.headers, cookies, text, json };
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

const PNG = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="), (c) => c.charCodeAt(0));
const pixel = () => { const f = new FormData(); f.append("file", new Blob([PNG], { type: "image/png" }), "pixel.png"); return f; };

// ---------------------------------------------------------------------------
// The history both servers start from.

const PEOPLE = ["owner", "maya", "joe", "ada", "kim", "ilya", "nils", "rue", "lou", "vex", "tom", "pat"];

async function seed() {
  const state = { people: {}, servers: {}, roles: {}, channels: {}, identities: {} };
  const person = Object.fromEntries(PEOPLE.map((name) => [name, new Person(name)]));
  const { owner } = person;
  must(await owner.post("/api/auth/register", {
    username: "owner", displayName: "Owner", email: "owner@example.test", password: PASSWORD, setupCode: args.setup,
  }), 201, "founding");
  await owner.login();
  const instance = must(await owner.get("/api/cockpit/bootstrap"), 200, "the panel").instance.id;
  state.servers.instance = instance;
  owner.server = instance;
  const instanceInvite = async () => must(await owner.post("/api/admin/invites"), 201, "an invite").url.split("/").pop();
  for (const name of PEOPLE.slice(1)) {
    const p = person[name];
    must(await p.post("/api/auth/register", {
      username: name, displayName: name[0].toUpperCase() + name.slice(1), email: `${name}@example.test`,
      password: PASSWORD, inviteCode: await instanceInvite(),
    }), 201, `registering ${name}`);
    await p.login();
  }
  for (const name of PEOPLE) state.people[name] = person[name].id;
  const { maya, joe, ada, kim, ilya, nils, lou, vex } = person;

  // The instance's roles, made in the panel.
  const listed = must(await owner.get("/api/cockpit/users"), 200, "the accounts");
  state.roles.base = listed.roles.find((r) => r.isSystem && !r.isOwner).id;
  state.roles.ownerRole = listed.roles.find((r) => r.isOwner).id;
  state.roles.moderator = must(await owner.post("/api/cockpit/roles", { name: "Moderator", color: "#22aa66", permissions: ["manageMessages", "viewAuditLog"] }), 201, "Moderator").id;
  state.roles.helper = must(await owner.post("/api/cockpit/roles", { name: "Helper", color: "oklch(0.7 0.1 200)", permissions: ["createInvite"] }), 201, "Helper").id;
  must(await owner.patch(`/api/cockpit/users/${ada.id}/role`, { roleId: state.roles.moderator }), 204, "Ada a moderator");
  must(await owner.patch(`/api/cockpit/users/${lou.id}/role`, { roleId: state.roles.helper }), 204, "Lou a helper");

  // What the overview counts: messages, a reaction, an upload and an emoji of the instance server.
  const boot = must(await owner.get("/api/bootstrap"), 200, "bootstrap");
  for (const c of boot.channels ?? []) state.channels[`instance-${c.name}`] = c.id;
  const general = state.channels["instance-general"];
  const upload = must(await owner.post("/api/uploads", undefined, { form: pixel() }), 201, "upload");
  const welcome = must(await owner.post(`/api/channels/${general}/messages`, { content: "Welcome", attachmentIds: [upload.id] }), 201, "welcome");
  must(await joe.post(`/api/channels/${general}/messages`, { content: "Hi all" }, { server: instance }), 201, "Joe writes");
  must(await maya.put(`/api/messages/${welcome.id}/reactions/${encodeURIComponent("👍")}`, undefined, { server: instance }), 204, "Maya reacts");
  const emoji = new FormData();
  emoji.append("shortcode", "pixel");
  emoji.append("file", new Blob([PNG], { type: "image/png" }), "pixel.png");
  must(await owner.post("/api/emoji", undefined, { form: emoji }), 201, "emoji");
  // A channel only moderators see, so that a change of Ada's role shows or hides it.
  const mods = must(await owner.post("/api/admin/channels", { name: "mods", kind: "text" }), 201, "mods");
  state.channels["instance-mods"] = mods.id;
  must(await owner.put(`/api/admin/channels/${mods.id}/permissions/${state.roles.base}`, { allow: [], deny: ["viewChannel"] }), 200, "hide mods");
  must(await owner.put(`/api/admin/channels/${mods.id}/permissions/${state.roles.moderator}`, { allow: ["viewChannel"], deny: [] }), 200, "show mods");

  // Maya owns a server of her own, not the instance.
  const invite = async (p, server) => must(await p.post("/api/admin/invites", undefined, { server }), 201, "invite").url.split("/").pop();
  const join = async (p, code) => must(await p.post(`/api/invites/${code}/join`), 200, `joining ${p.name}`);
  const orbit = must(await maya.post("/api/servers", { name: "Orbit", description: "Maya's place", textChannelName: "general", voiceChannelName: "hall" }), 201, "Orbit");
  state.servers.orbit = orbit.id;
  const orbitCode = await invite(maya, orbit.id);
  for (const p of [joe, ada]) await join(p, orbitCode);
  // Kim's servers: one to be deleted, one to be handed over.
  for (const [key, label] of [["doomed", "Doomed"], ["handover", "Handover"]]) {
    const s = must(await kim.post("/api/servers", { name: label, textChannelName: "talk" }), 201, label);
    state.servers[key] = s.id;
    kim.server = s.id;
    const code = await invite(kim, s.id);
    for (const p of [ilya, joe]) await join(p, code);
    const talk = (must(await kim.get("/api/bootstrap"), 200, "bootstrap").channels ?? []).find((c) => c.name === "talk");
    const message = must(await kim.post(`/api/channels/${talk.id}/messages`, { content: `hello ${key}` }), 201, "message");
    must(await ilya.put(`/api/messages/${message.id}/reactions/${encodeURIComponent("👍")}`, undefined, { server: s.id }), 204, "react");
    // A channel plain members do not see: the owner does, so a hand-over shows it to one and hides it from the other.
    const hidden = must(await kim.post("/api/admin/channels", { name: "hidden", kind: "text" }), 201, "hidden");
    state.channels[`${key}-hidden`] = hidden.id;
    const memberRole = must(await kim.get("/api/admin/roles"), 200, "roles").find((r) => r.isSystem && !r.isOwner);
    must(await kim.put(`/api/admin/channels/${hidden.id}/permissions/${memberRole.id}`, { allow: [], deny: ["viewChannel"] }), 200, "hide it");
  }
  state.servers.open = must(await ada.post("/api/servers", { name: "Open House", isPublic: true }), 201, "Open House").id;

  // More sessions: Joe from the desktop app, Nils twice.
  await new Person("joe", DESKTOP).login();
  await new Person("nils").login();
  await nils.get("/api/servers");
  // Vex is turned away.
  must(await owner.patch(`/api/cockpit/users/${vex.id}/status`, { disabled: true }), 204, "Vex disabled");
  writeFileSync(args.state, JSON.stringify(state, null, 2));
}

// What no route makes without an outside provider, written into the seeded
// file before it is copied: identity links (Pat's is the only way in, Tom
// also has a password), and Rue's second step, trusted devices and recovery
// codes.
const FIXED = {
  tomIdentity: "6A1D0000-0000-4000-8000-0000000000A1",
  patIdentity: "6A1D0000-0000-4000-8000-0000000000A2",
  rueTotp: "6A1D0000-0000-4000-8000-0000000000B1",
  rueMail: "6A1D0000-0000-4000-8000-0000000000B2",
  rueOld: "6A1D0000-0000-4000-8000-0000000000B3",
  rueDevice: "6A1D0000-0000-4000-8000-0000000000C1",
  rueDeviceOld: "6A1D0000-0000-4000-8000-0000000000C2",
};

function edit(db, sql, ...values) {
  const result = db.prepare(sql).run(...values);
  if (result.changes === 0) throw new Error(`no row changed: ${sql}`);
}

function prepare() {
  const state = JSON.parse(readFileSync(args.state, "utf8"));
  const db = new DatabaseSync(args.db);
  const user = (name) => state.people[name].toUpperCase();
  const at = "2026-11-20 10:00:00.1234567";
  edit(db, "INSERT INTO ExternalIdentities (Id, UserId, Issuer, Subject, ProviderId, CreatedAt) VALUES (?, ?, 'https://id.example.test', 'tom-subject', 'example', ?)", FIXED.tomIdentity, user("tom"), at);
  edit(db, "INSERT INTO ExternalIdentities (Id, UserId, Issuer, Subject, ProviderId, CreatedAt) VALUES (?, ?, 'https://id.example.test', 'pat-subject', NULL, '2026-11-20 10:00:01.5')", FIXED.patIdentity, user("pat"));
  edit(db, "UPDATE Users SET PasswordHash = NULL WHERE Id = ?", user("pat"));
  edit(db, "INSERT INTO MfaMethods (Id, UserId, Type, Name, CreatedAt, ConfirmedAt, LastUsedAt) VALUES (?, ?, 'totp', 'Phone', ?, ?, '2026-11-21 08:30:00')", FIXED.rueTotp, user("rue"), at, at);
  edit(db, "INSERT INTO MfaMethods (Id, UserId, Type, Name, CreatedAt, ConfirmedAt, EmailAddress) VALUES (?, ?, 'email', 'Mail', '2026-11-20 11:00:00.25', '2026-11-20 11:00:00.25', 'rue@example.test')", FIXED.rueMail, user("rue"));
  edit(db, "INSERT INTO MfaMethods (Id, UserId, Type, Name, CreatedAt, ConfirmedAt, RevokedAt) VALUES (?, ?, 'totp', 'Old phone', '2026-11-19 09:00:00', '2026-11-19 09:00:00', '2026-11-19 10:00:00')", FIXED.rueOld, user("rue"));
  edit(db, "INSERT INTO TrustedDevices (Id, UserId, TokenHash, Name, Device, IpAddress, CreatedAt, ExpiresAt) VALUES (?, ?, 'hash-one', 'Laptop', 'node', '10.0.0.1', ?, '2099-01-01 00:00:00')", FIXED.rueDevice, user("rue"), at);
  edit(db, "INSERT INTO TrustedDevices (Id, UserId, TokenHash, Name, Device, IpAddress, CreatedAt, ExpiresAt) VALUES (?, ?, 'hash-two', 'Old laptop', 'node', '10.0.0.2', '2026-01-01 00:00:00', '2026-02-01 00:00:00')", FIXED.rueDeviceOld, user("rue"));
  for (let i = 0; i < 4; i++) {
    edit(db, "INSERT INTO RecoveryCodes (Id, UserId, BatchId, CodeHash, CreatedAt, UsedAt) VALUES (?, ?, '6A1D0000-0000-4000-8000-0000000000D0', ?, ?, ?)",
      `6A1D0000-0000-4000-8000-0000000000E${i}`, user("rue"), `code-hash-${i}`, at, i === 0 ? at : null);
  }
  db.close();
  for (const [key, id] of Object.entries(FIXED)) state.identities[key] = id.toLowerCase();
  writeFileSync(args.state, JSON.stringify(state, null, 2));
}

// ---------------------------------------------------------------------------
// What each server answers.

const lines = [];
const out = (text) => lines.push(text);
let state;
const names = new Map();
// Moments the seeded file holds: the same in both copies, so kept as they are.
const seededMoments = new Set();
const momentStats = { fresh: 0, longest: new Map(), endsInZero: 0, zoned: 0, plain: 0 };

function nameIds() {
  for (const [name, id] of Object.entries(state.people)) names.set(id.toLowerCase(), `person:${name}`);
  for (const [name, id] of Object.entries(state.servers)) names.set(id.toLowerCase(), `server:${name}`);
  for (const [name, id] of Object.entries(state.channels)) names.set(id.toLowerCase(), `channel:${name}`);
  for (const [name, id] of Object.entries(state.roles)) names.set(id.toLowerCase(), `role:${name}`);
  for (const [name, id] of Object.entries(state.identities ?? {})) names.set(id.toLowerCase(), `fixed:${name}`);
  names.set(UNKNOWN, "unknown");
  names.set(NIL, "nil");
}

function readSeededMoments() {
  if (!args.seeded) return;
  const db = new DatabaseSync(args.seeded, { readOnly: true });
  const pattern = /^\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d(\.\d+)?$/;
  for (const { name } of db.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").all()) {
    let rows;
    try {
      rows = db.prepare(`SELECT * FROM "${name}"`).all();
    } catch {
      continue;
    }
    for (const row of rows) {
      for (const value of Object.values(row)) {
        if (typeof value === "string" && pattern.test(value)) seededMoments.add(value.replace(" ", "T"));
      }
    }
  }
  db.close();
}

let fresh = 0;
/** Ids, secrets and moments that differ by nature are replaced; the rest is kept as it was sent. */
function scrub(text, { stats = true } = {}) {
  let result = text;
  result = result.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, (id) => {
    const key = id.toLowerCase();
    if (!names.has(key)) names.set(key, `new${++fresh}`);
    const name = names.get(key);
    return id === key ? `<${name}>` : `<${name}:UPPER>`;
  });
  result = result.replace(/(\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d)(\.\d+)?(Z|[+-]\d\d:\d\d)?/g, (whole, seconds, fraction = "", zone = "", at, all) => {
    if (seededMoments.has(`${seconds.replace(" ", "T")}${fraction}`)) return whole;
    const digits = fraction ? fraction.length - 1 : 0;
    if (stats) {
      // How long the fractions of each member get, over the whole run: the
      // first server writes up to seven digits and drops trailing zeros.
      const member = /"(\w+)":"$/.exec(all.slice(Math.max(0, at - 40), at))?.[1] ?? "(text)";
      momentStats.fresh++;
      momentStats.longest.set(member, Math.max(momentStats.longest.get(member) ?? 0, digits));
      if (fraction.endsWith("0")) momentStats.endsInZero++;
      if (zone) momentStats.zoned++;
      else momentStats.plain++;
    }
    // A fresh moment loses trailing zeros by chance, so only whether it has a fraction is kept here.
    return `<time${digits === 0 ? "" : ".f"}${zone}>`;
  });
  result = result.replace(/\w{3}, \d\d \w{3} \d{4} \d\d:\d\d:\d\d GMT/g, "<date>");
  result = result.replace(/"traceId":"[^"]*"/g, '"traceId":"<trace>"');
  result = result.replace(/oklch\(0\.58 0\.14 \d+\)/g, "oklch(<hue>)");
  result = result.replace(/"ipAddress":"[^"]*"/g, '"ipAddress":"<ip>"');
  result = result.replace(/"credential":"[^"]*"/g, '"credential":"<credential>"');
  result = result.replace(/"secret":"[A-Z2-7=]+"/g, '"secret":"<totp secret>"');
  result = result.replace(/"uri":"otpauth:[^"]*"/g, '"uri":"<otpauth>"');
  result = result.replace(/"recoveryCodes":\[[^\]]*\]/g, (codes) => `"recoveryCodes":[${(codes.match(/"[^"]*"/g) ?? []).slice(1).map(() => "<code>").join(",")}]`);
  result = result.replace(/stellarr-logs-\d{8}-\d{6}\.tsv/g, "stellarr-logs-<stamp>.tsv");
  result = result.replace(/ in \d+(\.\d+)? ?ms/g, " in <n> ms");
  result = result.replace(/delivery-test\/[0-9a-f]+@/g, "delivery-test/<hex>@");
  if (args.public) result = result.split(args.public).join("<origin>");
  if (args["smtp-host"]) result = result.split(`"smtpHost":"${args["smtp-host"]}"`).join('"smtpHost":"<smtp host>"');
  for (const port of [args["smtp-port"], args["smtp-closed"], args["smtp-refusing"]].filter(Boolean)) {
    result = result.split(`"smtpPort":${port}`).join(`"smtpPort":<${port === args["smtp-port"] ? "sink" : port === args["smtp-closed"] ? "closed" : "refusing"}>`);
  }
  return result;
}

function headerLines(response) {
  const lines = [];
  for (const name of ["content-type", "cache-control", "pragma", "expires", "location", "content-disposition", "allow", "retry-after", "www-authenticate", "x-content-type-options"]) {
    const value = response.headers.get(name);
    if (value) lines.push(`${name}: ${value}`);
  }
  for (const cookie of response.cookies) {
    const [pair, ...rest] = cookie.split(";");
    const at = pair.indexOf("=");
    lines.push(`set-cookie: ${pair.slice(0, at)}=<${pair.slice(at + 1) ? "value" : "empty"}>;${rest.join(";")}`);
  }
  return lines;
}

function describe(response) {
  return scrub([String(response.status), ...headerLines(response), `BODY: ${response.text}`].join(" | "));
}

/**
 * The shape of a JSON text, read from the text: the members of each object in
 * their order, and for a value its kind. A number is `number` unless it is
 * written with a fraction of zeros (`1.0`), which the first server never
 * wrote; a string that is a moment says whether it ends in Z and how long its
 * fraction is.
 */
function shapeOf(text) {
  let i = 0;
  const space = () => { while (" \t\n\r".includes(text[i])) i++; };
  const string = () => {
    let j = i + 1;
    while (text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
    const value = JSON.parse(text.slice(i, j + 1));
    i = j + 1;
    return value;
  };
  const value = () => {
    space();
    const c = text[i];
    if (c === "{") {
      i++;
      space();
      if (text[i] === "}") { i++; return "{}"; }
      const members = [];
      for (;;) {
        space();
        const key = string();
        space();
        i++;
        members.push(`${key}:${value()}`);
        space();
        if (text[i++] === ",") continue;
        break;
      }
      return `{${members.join(",")}}`;
    }
    if (c === "[") {
      i++;
      space();
      if (text[i] === "]") { i++; return "[]"; }
      const items = [];
      for (;;) {
        items.push(value());
        space();
        if (text[i++] === ",") continue;
        break;
      }
      return `[${[...new Set(items)].join(" | ")}]`;
    }
    if (c === '"') {
      const s = string();
      const m = /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?(Z|[+-]\d\d:\d\d)?$/.exec(s);
      if (!m) return "string";
      const digits = m[1] ? m[1].length - 1 : 0;
      return `moment(${digits > 0 ? "fraction" : "whole seconds"}${m[2] ? `,${m[2]}` : ""})`;
    }
    for (const word of ["true", "false", "null"]) {
      if (text.startsWith(word, i)) { i += word.length; return word === "null" ? "null" : "bool"; }
    }
    const m = /^-?\d+(\.\d+)?([eE][+-]?\d+)?/.exec(text.slice(i, i + 64));
    i += m[0].length;
    return m[1] && /^\.0+$/.test(m[1]) ? "number.0" : "number";
  };
  try {
    return value();
  } catch {
    return "<not JSON>";
  }
}

function describeShape(response) {
  return scrub([String(response.status), ...headerLines(response), `SHAPE: ${shapeOf(response.text)}`].join(" | "));
}

/** One hub connection of a person, keeping every invocation it hears and whether it was closed. */
class Listener {
  constructor(person, query = "") {
    this.person = person;
    this.query = query;
    this.heard = [];
    this.buffer = "";
    this.closed = false;
  }

  async open() {
    const cookie = [...this.person.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
    this.socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app${this.query}`, { headers: { Cookie: cookie, ...clientHeaders } });
    await new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = () => reject(new Error(`the hub refused ${this.person.name}`));
    });
    this.socket.onerror = () => {};
    // When the server closes a connection is a matter of a watcher's timing:
    // it is asked for where it is expected, never heard as an event.
    this.socket.onclose = () => {
      this.closed = true;
    };
    this.socket.onmessage = (event) => {
      this.buffer += String(event.data);
      let end;
      while ((end = this.buffer.indexOf(RS)) >= 0) {
        const text = this.buffer.slice(0, end);
        this.buffer = this.buffer.slice(end + 1);
        if (text === "{}") continue;
        const record = JSON.parse(text);
        if (record.type === 1 && !QUIET.has(record.target)) this.heard.push(`${record.target} ${JSON.stringify(record.arguments)}`);
        if (record.type === 7) this.closing = JSON.stringify(record);
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

// Presence follows the listeners' own comings and goings, in an order neither server fixes.
const QUIET = new Set(["PresenceChanged"]);

let listeners = {};
let unexpected = 0;

/** Waits up to three seconds for the server to close a person's hub connection, says whether it did, and stops listening there. */
async function closedWithin(name) {
  const listener = listeners[name];
  for (let waited = 0; waited < 3000 && !listener.closed; waited += 100) await sleep(100);
  out(`## ${name}'s hub connection closed within 3 s: ${listener.closed}${listener.closing ? `, saying ${scrub(listener.closing)}` : ""}`);
  for (const line of listener.take()) out(`  ${name} <- ${scrub(line)}`);
  delete listeners[name];
}

async function step(label, person, method, path, options = {}) {
  const { as, wait = 150, shape = false, expect, ...request } = options;
  const response = await person.send(method, path, request);
  // A fresh id is named after the step that made it, so one that differs does not rename the rest.
  if (as && typeof response.json?.id === "string") names.set(response.json.id.toLowerCase(), as);
  if (wait > 0) await sleep(wait);
  const heard = Object.entries(listeners).flatMap(([name, listener]) => listener.take().map((event) => `  ${name} <- ${scrub(event)}`)).sort();
  out(`## ${label}`);
  out(shape ? describeShape(response) : describe(response));
  for (const line of heard) out(line);
  if (expect !== undefined && response.status !== expect) {
    unexpected++;
    out(`!! the scenario expected ${expect}`);
  }
  return response;
}

const cockpit = (path) => `/api/cockpit${path}`;

/**
 * Every route of the panel, asked so that it changes nothing: an id nobody
 * has, a body the route refuses, or a read.
 */
function everyRoute(t) {
  const P = t.people;
  return [
    ["GET", "/bootstrap"],
    ["GET", "/activity"],
    ["GET", "/users"],
    ["POST", "/users", { body: { username: "", displayName: "", password: "", roleId: t.roles.base } }],
    ["PATCH", `/users/${UNKNOWN}/role`, { body: { roleId: t.roles.base } }],
    ["PATCH", `/users/${UNKNOWN}/status`, { body: { disabled: false } }],
    ["GET", `/users/${UNKNOWN}/security`],
    ["POST", `/users/${UNKNOWN}/security/recover`, { body: { reason: "Lost the phone" } }],
    ["POST", `/users/${UNKNOWN}/identities/${UNKNOWN}/remove`],
    ["POST", "/roles", { body: { name: "", color: "", permissions: [] } }],
    ["PATCH", `/roles/${UNKNOWN}`, { body: { name: "Nobody" } }],
    ["GET", "/servers"],
    ["GET", `/servers/${UNKNOWN}/members`],
    ["POST", `/servers/${UNKNOWN}/transfer`, { body: { userId: P.joe } }],
    ["DELETE", `/servers/${UNKNOWN}`],
    ["GET", "/sessions"],
    ["DELETE", `/sessions/${UNKNOWN}`],
    ["GET", "/runtime", { shape: true }],
    ["GET", "/logs", { shape: true }],
    ["GET", "/logs/export", { tsv: true }],
    ["GET", "/fresh-second-step"],
    ["GET", "/settings"],
    ["PUT", "/settings", { body: {} }],
    ["GET", "/desktop-release"],
    ["PUT", "/desktop-release", { body: {} }],
    ["POST", "/mail/test", { body: {} }],
    ["POST", "/integrations/giphy/test"],
    ["GET", "/integrations/identity-providers"],
    ["PUT", "/integrations/identity-providers/google", { body: { signUp: true } }],
    ["GET", "/secrets"],
    ["PUT", "/secrets/nope", { body: { value: "x" } }],
    ["DELETE", "/secrets/nope"],
    ["GET", "/nothing-here"],
    ["DELETE", "/users"],
  ];
}

function describeTsv(response) {
  const rows = response.text.split("\n");
  const cells = rows.slice(1).filter((row) => row !== "").map((row) => row.split("\t").length);
  const shape = response.status === 200
    ? `TSV: first row ${JSON.stringify(rows[0])}; ${cells.length > 0 ? "rows" : "no rows"}; every row of five cells: ${cells.every((n) => n === 5)}; ends with a line break: ${response.text.endsWith("\n")}`
    : `BODY: ${response.text}`;
  return scrub([String(response.status), ...headerLines(response), shape].join(" | "), { stats: false });
}

async function gate(callers, t) {
  out("# who passes the gate");
  for (const [method, path, options = {}] of everyRoute(t)) {
    for (const [name, person] of Object.entries(callers)) {
      const { tsv, ...rest } = options;
      if (tsv) {
        const response = await person.send(method, cockpit(path), rest);
        out(`## gate ${method} ${path}: ${name}`);
        out(describeTsv(response));
      } else {
        await step(`gate ${method} ${path}: ${name}`, person, method, cockpit(path), { ...rest, wait: 0 });
      }
    }
  }
}

/**
 * The routes that read a body or an id, each asked with every kind of broken
 * request, so that the order of the refusals shows: the content type, who
 * is calling, the id, the body, the owner, the second step, the route.
 */
function guardedRoutes(t, ownSession) {
  const P = t.people;
  const R = t.roles;
  const S = t.servers;
  const fullSettings = { instanceName: "Stellarr", timeZone: "UTC", mailingEnabled: false, mailFromAddress: "", inviteMailEnabled: true, recoveryMailEnabled: true, alertMailEnabled: false, backupRetentionDays: 14, logRetentionDays: 30, orphanObjectRetentionDays: 1, automaticCleanupEnabled: true, smtpPort: 0 };
  return [
    { label: "create an account", method: "POST", path: () => "/users", good: { username: "", displayName: "", password: "", roleId: R.base }, wrong: { username: 5 } },
    { label: "a role", method: "PATCH", path: (id) => `/users/${id}/role`, id: UNKNOWN, real: P.ada, good: { roleId: R.moderator }, wrong: { roleId: 5 } },
    { label: "a status", method: "PATCH", path: (id) => `/users/${id}/status`, id: UNKNOWN, real: P.joe, good: { disabled: false }, wrong: { disabled: "no" } },
    { label: "recover", method: "POST", path: (id) => `/users/${id}/security/recover`, id: UNKNOWN, real: P.owner, good: { reason: "Lost the phone" }, wrong: { reason: 5 } },
    { label: "unlink", method: "POST", path: (id) => `/users/${P.tom}/identities/${id}/remove`, id: UNKNOWN, real: UNKNOWN },
    { label: "create a role", method: "POST", path: () => "/roles", good: { name: "", color: "", permissions: [] }, wrong: { permissions: "viewChannel" } },
    { label: "change a role", method: "PATCH", path: (id) => `/roles/${id}`, id: UNKNOWN, real: R.ownerRole, good: { name: "Renamed" }, wrong: { name: ["x"] } },
    { label: "transfer", method: "POST", path: (id) => `/servers/${id}/transfer`, id: UNKNOWN, real: S.instance, good: { userId: P.joe }, wrong: { userId: 5 } },
    { label: "delete a server", method: "DELETE", path: (id) => `/servers/${id}`, id: UNKNOWN, real: S.instance },
    { label: "fresh second step", method: "GET", path: () => "/fresh-second-step" },
    { label: "desktop release", method: "PUT", path: () => "/desktop-release", good: { minimumVersion: "", recommendedVersion: "" }, wrong: { minimumVersion: 1 } },
    { label: "provider sign-up", method: "PUT", path: () => "/integrations/identity-providers/google", good: { signUp: true }, wrong: { signUp: "yes" } },
    { label: "set a secret", method: "PUT", path: () => "/secrets/nope", good: { value: "x" }, wrong: { value: 5 } },
    { label: "clear a secret", method: "DELETE", path: () => "/secrets/nope" },
    { label: "settings", method: "PUT", path: () => "/settings", good: fullSettings, wrong: { ...fullSettings, mailingEnabled: "no" } },
    { label: "test mail", method: "POST", path: () => "/mail/test", good: { to: "" }, wrong: { to: 5 } },
    { label: "revoke a session", method: "DELETE", path: (id) => `/sessions/${id}`, id: UNKNOWN, real: ownSession },
    { label: "security", method: "GET", path: (id) => `/users/${id}/security`, id: UNKNOWN, real: P.joe },
    { label: "members", method: "GET", path: (id) => `/servers/${id}/members`, id: UNKNOWN, real: S.orbit },
  ];
}

function shapesOf(route) {
  const shapes = [];
  const path = route.path(route.id);
  if (route.good !== undefined) {
    shapes.push(["good body", path, { body: route.good }]);
    shapes.push(["broken JSON", path, { raw: "{", type: "application/json" }]);
    shapes.push(["empty body", path, { raw: "", type: "application/json" }]);
    shapes.push(["null", path, { raw: "null", type: "application/json" }]);
    shapes.push(["an array", path, { raw: "[]", type: "application/json" }]);
    shapes.push(["text/plain", path, { raw: JSON.stringify(route.good), type: "text/plain" }]);
    shapes.push(["no body, no type", path, {}]);
    shapes.push(["wrong kind of member", path, { body: route.wrong }]);
  } else {
    shapes.push(["as it is", path, {}]);
  }
  const body = route.good !== undefined ? { body: route.good } : {};
  if (route.id) {
    shapes.push(["id x", route.path("x"), body]);
    shapes.push(["id nil", route.path(NIL), body]);
    if (route.real) shapes.push(["real id in upper case", route.path(route.real.toUpperCase()), body]);
    if (route.real) shapes.push(["real id in braces", route.path(`{${route.real}}`), body]);
    if (route.real) shapes.push(["real id without hyphens", route.path(route.real.replaceAll("-", "")), body]);
  }
  shapes.push(["trailing slash", `${path}/`, body]);
  return shapes;
}

async function refusals(callers, t, ownSession) {
  out("# the order of the refusals");
  for (const route of guardedRoutes(t, ownSession)) {
    for (const [shape, path, options] of shapesOf(route)) {
      for (const [name, person] of Object.entries(callers)) {
        // A change that went through is heard on the hub a little later.
        const response = await step(`order ${route.label}, ${shape}: ${name}`, person, route.method, cockpit(path), { ...options, wait: 0 });
        if (response.status < 300 && route.method !== "GET") {
          await sleep(150);
          for (const [who, listener] of Object.entries(listeners)) for (const event of listener.take()) out(`  ${who} <- ${scrub(event)}`);
        }
      }
    }
  }
}

async function probe() {
  state = JSON.parse(readFileSync(args.state, "utf8"));
  nameIds();
  readSeededMoments();
  const P = state.people;
  const R = state.roles;
  const S = state.servers;
  // Vex is disabled, Pat has no password and Rue would be asked for her second step: none of them signs in.
  const who = Object.fromEntries(PEOPLE.filter((name) => !["vex", "pat", "rue"].includes(name)).map((name) => [name, new Person(name)]));
  for (const person of Object.values(who)) await person.login();
  const { owner } = who;
  owner.server = S.instance;
  const anonymous = new Person("anonymous");
  listeners = {};
  for (const [name, server] of [["owner", S.instance], ["ada", S.instance], ["lou", S.instance], ["nils", S.instance], ["kim", S.handover], ["ilya", S.handover], ["joe", S.handover], ["maya", S.orbit]]) {
    listeners[name] = await new Listener(who[name], `?serverId=${server}`).open();
  }
  await sleep(500);
  for (const listener of Object.values(listeners)) listener.take();
  const ownSession = (await owner.get(cockpit("/sessions"))).json.find((s) => s.current).id;
  names.set(ownSession.toLowerCase(), "session:owner-probe");

  const callers = { owner, maya: who.maya, joe: who.joe, anonymous };
  await gate(callers, state);
  await refusals(callers, state, ownSession);

  // ----- accounts
  out("# accounts");
  await step("users: the list", owner, "GET", cockpit("/users"));
  const user = (label, body, expect) => step(`create: ${label}`, owner, "POST", cockpit("/users"), { body, as: `user:${label}`, expect });
  const account = (username, extra = {}) => ({ username, displayName: "Somebody", password: PASSWORD, roleId: R.base, ...extra });
  await user("name of 2", account("ab"), 201);
  await user("name of 1", account("a"), 400);
  await user("name of 32", account("n".repeat(32)), 201);
  await user("name of 33", account("m".repeat(33)), 400);
  await user("a space inside", account("two words"), 400);
  await user("spaces around", account("  padded  "), 201);
  await user("dots, dashes and underscores", account("a_b.c-d"), 201);
  await user("a plus", account("a+b"));
  await user("an at sign", account("a@b"));
  await user("non-ASCII letters", account("zażółć"));
  await user("Cyrillic", account("Жора"));
  await user("Arabic-Indic digits", account("ab٣٤"));
  await user("an astral letter", account("ab𝐀"));
  await user("only astral letters", account("𝐀𝐁"));
  await user("16 astral letters (32 units)", account("𝐀".repeat(16)));
  await user("17 astral letters (34 units)", account("𝐁".repeat(17)));
  await user("an emoji", account("ab😀"));
  await user("a combining mark", account("e\u0301e"));
  await user("a precomposed letter", account("ée"));
  await user("a zero-width joiner", account("a\u200db"));
  await user("a tab inside", account("a\tb"));
  await user("dotless i, which upper-cases to KIM", account("kım"));
  await user("long s, which upper-cases to NILS", account("nilſ"));
  await user("Kelvin sign, which upper-cases to itself", account("\u212Aay"));
  await user("sharp s", account("straße"));
  await user("sharp s spelled out", account("strasse"));
  await user("upper case of an existing name", account("JOE"));
  await user("German capital sharp s", account("ẞig"));
  await user("Turkish dotted capital I", account("İlya"));
  await user("display name of 64", account("dn64", { displayName: "d".repeat(64) }));
  await user("display name of 65", account("dn65", { displayName: "d".repeat(65) }));
  await user("display name of 32 astral letters (64 units)", account("dnastral", { displayName: "𝐀".repeat(32) }));
  await user("display name blank", account("dnblank", { displayName: "   " }));
  await user("display name padded", account("dnpad", { displayName: "  Pad  " }));
  await user("display name with markup", account("dnmarkup", { displayName: "<b>Bold</b> & 'quoted' \"too\"" }));
  await user("display name null", account("dnnull", { displayName: null }));
  await user("password of 7", account("pw7", { password: "p".repeat(7) }));
  await user("password of 8", account("pw8", { password: "p".repeat(8) }));
  await user("password of 256", account("pw256", { password: "p".repeat(256) }));
  await user("password of 257", account("pw257", { password: "p".repeat(257) }));
  await user("password of 4 emoji (8 units)", account("pwemoji", { password: "😀".repeat(4) }));
  await user("password of 7 spaces and a letter", account("pwspace", { password: "       x" }));
  await user("password null", account("pwnull", { password: null }));
  const emails = [
    ["plain", "new@example.test"], ["the same in upper case", "NEW@EXAMPLE.TEST"], ["padded", "  padded@example.test  "], ["empty", ""],
    ["blank", "   "], ["display name form", "Ann <ann@example.test>"], ["angle brackets only", "<bare@example.test>"], ["no at", "no-at"],
    ["no dot", "a@b"], ["two ats", "two@@example.test"], ["dot before at", "dot.@example.test"], ["quoted local part", "\"quoted name\"@example.test"],
    ["address literal", "user@[127.0.0.1]"], ["non-ASCII", "ünï@exämple.test"], ["at inside", "a@b@c.test"], ["long", `${"x".repeat(310)}@example.test`],
    ["a comment", "cmt@example.test (note)"], ["an existing one", "owner@example.test"], ["an existing one in upper case", "MAYA@EXAMPLE.TEST"],
    ["trailing dot", "trail@example.test."], ["two addresses", "one@example.test, two@example.test"], ["IDN in ASCII", "idn@xn--bcher-kva.example"],
    ["unicode upper-casing", "ı@example.test"],
  ];
  let n = 0;
  for (const [label, email] of emails) await user(`email ${label}`, account(`mail${++n}`, { email }));
  await user("email null", account("mailnull", { email: null }));
  await user("email a number", account("mailnum", { email: 5 }));
  await user("role: the owner role", account("roleowner", { roleId: R.ownerRole }), 400);
  await user("role: the base role", account("rolebase", { roleId: R.base }), 201);
  await user("role: moderator", account("rolemod", { roleId: R.moderator }), 201);
  await user("role: unknown", account("roleunknown", { roleId: UNKNOWN }), 400);
  await user("role: nil", account("rolenil", { roleId: NIL }), 400);
  await user("role: left out", { username: "roleout", displayName: "R", password: PASSWORD });
  await user("role: null", account("rolenull", { roleId: null }));
  await user("role: not an id", account("rolex", { roleId: "x" }));
  await user("role: upper case", account("roleupper", { roleId: R.moderator.toUpperCase() }));
  await user("role: braces", account("rolebrace", { roleId: `{${R.moderator}}` }));
  await user("a taken name and a bad role", account("joe", { roleId: UNKNOWN }));
  await user("username a number", account(5));
  await user("extra members", { ...account("extra"), surprise: true, isDisabled: true });
  // The first server matched member names whatever their case (a known difference): Joe's name, taken, shows which.
  await user("property names in another case", { Username: "joe", DisplayName: "Joe", Password: PASSWORD, RoleId: R.base });
  await step("users: the list after", owner, "GET", cockpit("/users"));

  // ----- roles
  out("# roles");
  const role = (label, body, expect) => step(`role create: ${label}`, owner, "POST", cockpit("/roles"), { body, as: `role:${label}`, expect });
  await role("plain", { name: "Mods", color: "#ff0000", permissions: ["manageMessages"] }, 201);
  await role("the same name", { name: "Mods", color: "#00ff00", permissions: [] }, 409);
  await role("the same name in lower case", { name: "mods", color: "#00ff00", permissions: [] });
  await role("the name of the owner role", { name: "Owner", color: "#000", permissions: [] });
  await role("the name of the base role", { name: "Member", color: "#000", permissions: [] });
  await role("name of 64", { name: "r".repeat(64), color: "#000", permissions: [] });
  await role("name of 65", { name: "s".repeat(65), color: "#000", permissions: [] });
  await role("name padded", { name: "  Spaced  ", color: "  #123  ", permissions: [] });
  await role("name blank", { name: "  ", color: "#000", permissions: [] });
  await role("name of astral letters", { name: "𝐑".repeat(32), color: "#000", permissions: [] });
  await role("name of 33 astral letters", { name: "𝐒".repeat(33), color: "#000", permissions: [] });
  await role("color of 48", { name: "Color48", color: "c".repeat(48), permissions: [] });
  await role("color of 49", { name: "Color49", color: "c".repeat(49), permissions: [] });
  await role("color blank", { name: "Colorless", color: " ", permissions: [] });
  await role("color anything", { name: "Anything", color: "not a color", permissions: [] });
  await role("permissions left out", { name: "Bare", color: "#000" });
  await role("permissions null", { name: "Nulls", color: "#000", permissions: null });
  await role("permissions repeated", { name: "Twice", color: "#000", permissions: ["viewChannel", "viewChannel", "administrator"] });
  await role("every permission", { name: "Everything", color: "#000", permissions: ["manageServer", "manageChannels", "manageRoles", "manageMembers", "manageMessages", "moderateMembers", "manageBans", "viewAuditLog", "administrator", "sendMessages", "attachFiles", "addReactions", "connect", "createInvite", "viewChannel"] });
  await role("an unknown permission", { name: "Unknown", color: "#000", permissions: ["flyAround"] });
  await role("a permission in another case", { name: "Cased", color: "#000", permissions: ["ViewChannel"] });
  await role("a permission null", { name: "NullPerm", color: "#000", permissions: [null] });
  await role("name null", { name: null, color: "#000", permissions: [] });
  await role("name a number", { name: 5, color: "#000", permissions: [] });
  await role("markup in the name", { name: "<i>Fancy</i> & co", color: "#000", permissions: [] });
  const update = (label, id, body, expect) => step(`role update: ${label}`, owner, "PATCH", cockpit(`/roles/${id}`), { body, expect });
  await update("rename the moderators", R.moderator, { name: "Moderators" }, 200);
  await update("rename to a taken name", R.moderator, { name: "Helper" }, 409);
  await update("rename to its own name", R.moderator, { name: "Moderators" }, 200);
  await update("nothing", R.moderator, {}, 200);
  await update("nulls", R.moderator, { name: null, color: null, permissions: null }, 200);
  await update("color only", R.helper, { color: "#abcdef" }, 200);
  await update("permissions of the helpers", R.helper, { permissions: ["createInvite", "viewAuditLog"] }, 200);
  await update("no permissions", R.helper, { permissions: [] }, 200);
  await update("an unknown permission", R.helper, { permissions: ["flyAround"] }, 400);
  await update("name blank", R.helper, { name: " " }, 400);
  await update("name of 65", R.helper, { name: "h".repeat(65) }, 400);
  await update("the owner role", R.ownerRole, { name: "Boss" }, 409);
  await update("the owner role, broken name", R.ownerRole, { name: "" });
  await update("the base role", R.base, { color: "#445566" }, 200);
  await update("the base role, its permissions", R.base, { permissions: ["sendMessages", "attachFiles", "addReactions", "connect", "viewChannel", "createInvite"] }, 200);
  await update("the base role renamed", R.base, { name: "Everyone" }, 200);
  await update("unknown", UNKNOWN, { name: "x" }, 404);
  await update("a role of another server", (await who.maya.get("/api/admin/roles", { server: S.orbit })).json[0].id, { name: "Stolen" }, 404);

  // ----- a role and a status for a person
  out("# a person's role and status");
  const setRole = (label, id, roleId, expect) => step(`set role: ${label}`, owner, "PATCH", cockpit(`/users/${id}/role`), { body: { roleId }, expect });
  await setRole("ada to helper", P.ada, R.helper, 204);
  await setRole("ada to the base role", P.ada, R.base, 204);
  await setRole("ada to moderator", P.ada, R.moderator, 204);
  await setRole("ada to moderator again", P.ada, R.moderator, 204);
  await setRole("self", P.owner, R.base, 409);
  await setRole("to the owner role", P.ada, R.ownerRole, 404);
  await setRole("unknown person", UNKNOWN, R.base, 404);
  await setRole("unknown role", P.ada, UNKNOWN, 404);
  await setRole("a role of another server", P.ada, (await who.maya.get("/api/admin/roles", { server: S.orbit })).json[0].id, 404);
  await setRole("a disabled person", P.vex, R.helper, 204);
  await setRole("roleId left out", P.lou, undefined);
  await step("set role: an empty object", owner, "PATCH", cockpit(`/users/${P.lou}/role`), { body: {} });
  const setStatus = (label, id, body, expect) => step(`status: ${label}`, owner, "PATCH", cockpit(`/users/${id}/status`), { body, expect });
  await setStatus("self", P.owner, { disabled: true }, 409);
  await setStatus("unknown", UNKNOWN, { disabled: true }, 404);
  await setStatus("no change", P.joe, { disabled: false }, 204);
  await setStatus("an empty object", P.joe, {}, 204);
  await setStatus("null", P.joe, { disabled: null });
  await setStatus("a string", P.joe, { disabled: "true" });
  await setStatus("a number", P.joe, { disabled: 1 });
  await setStatus("Vex again", P.vex, { disabled: true }, 204);
  // Nils has two sessions and a hub connection.
  const nilsAgain = await new Person("nils").login();
  await step("sessions before Nils is turned away", owner, "GET", cockpit("/sessions"));
  await setStatus("Nils", P.nils, { disabled: true }, 204);
  await closedWithin("nils");
  await step("Nils' first session after", who.nils, "GET", "/api/servers", { server: null, expect: 401 });
  await step("Nils' second session after", nilsAgain, "GET", "/api/servers", { server: null, expect: 401 });
  await step("Nils signs in while turned away", new Person("nils"), "POST", "/api/auth/login", { body: { username: "nils", password: PASSWORD }, expect: 401 });
  await step("sessions after Nils is turned away", owner, "GET", cockpit("/sessions"));
  await step("the list with Nils turned away", owner, "GET", cockpit("/users"));
  await setStatus("Nils back", P.nils, { disabled: false }, 204);
  await step("Nils' old session after coming back", who.nils, "GET", "/api/servers", { server: null });
  await step("Nils signs in again", new Person("nils"), "POST", "/api/auth/login", { body: { username: "nils", password: PASSWORD }, expect: 200 });

  // ----- an account's security
  out("# security");
  const security = (label, id, expect) => step(`security: ${label}`, owner, "GET", cockpit(`/users/${id}/security`), { expect });
  await security("Rue", P.rue, 200);
  await security("Tom", P.tom, 200);
  await security("Pat", P.pat, 200);
  await security("Vex", P.vex, 200);
  await security("the owner", P.owner, 200);
  await security("unknown", UNKNOWN, 404);
  const recover = (label, id, body, expect) => step(`recover: ${label}`, owner, "POST", cockpit(`/users/${id}/security/recover`), { body, expect });
  await recover("self", P.owner, { reason: "My own" }, 409);
  await recover("self, bad reason", P.owner, { reason: "x" }, 409);
  await recover("reason of 2", P.lou, { reason: "ab" }, 400);
  await recover("reason of 2 padded", P.lou, { reason: "  ab  " }, 400);
  await recover("reason of 501", P.lou, { reason: "r".repeat(501) }, 400);
  await recover("reason null", P.lou, { reason: null }, 400);
  await recover("reason left out", P.lou, {}, 400);
  await recover("reason a number", P.lou, { reason: 5 });
  await recover("unknown, good reason", UNKNOWN, { reason: "Lost it" }, 404);
  await recover("unknown, bad reason", UNKNOWN, { reason: "x" }, 400);
  await recover("disabled Vex", P.vex, { reason: "Lost it" }, 409);
  await recover("Tom, reason of 250 astral letters (500 units)", P.tom, { reason: "𝐑".repeat(250) }, 200);
  const first = await recover("Lou, reason of 3", P.lou, { reason: "abc" }, 200);
  await closedWithin("lou");
  const rueFirst = await recover("Rue, reason of 500", P.rue, { reason: "q".repeat(500) }, 200);
  const rueSecond = await recover("Rue again", P.rue, { reason: "  Lost it again  " }, 200);
  await security("Rue after", P.rue, 200);
  await step("Rue signs in with the password", new Person("rue"), "POST", "/api/auth/login", { body: { username: "rue", password: PASSWORD } });
  await step("the audit after the recoveries", owner, "GET", "/api/admin/audit");

  // ----- identity links
  out("# identity links");
  const unlink = (label, id, identity, expect) => step(`unlink: ${label}`, owner, "POST", cockpit(`/users/${id}/identities/${identity}/remove`), { expect });
  await unlink("Pat's only way in", P.pat, state.identities.patIdentity, 409);
  await unlink("Tom's through Pat", P.pat, state.identities.tomIdentity, 404);
  await unlink("Tom's", P.tom, state.identities.tomIdentity, 204);
  await unlink("Tom's again", P.tom, state.identities.tomIdentity, 404);
  await unlink("unknown person", UNKNOWN, state.identities.patIdentity, 404);
  await unlink("Pat's through Tom", P.tom, state.identities.patIdentity, 404);
  await security("Tom after", P.tom, 200);

  // ----- servers
  out("# servers");
  await step("servers: the list", owner, "GET", cockpit("/servers"));
  for (const name of ["instance", "orbit", "handover", "open"]) await step(`members: ${name}`, owner, "GET", cockpit(`/servers/${S[name]}/members`), { expect: 200 });
  await step("members: unknown", owner, "GET", cockpit(`/servers/${UNKNOWN}/members`), { expect: 404 });
  const transfer = (label, id, body, expect) => step(`transfer: ${label}`, owner, "POST", cockpit(`/servers/${id}/transfer`), { body, expect });
  await transfer("the instance", S.instance, { userId: P.joe }, 409);
  await transfer("the instance, broken body", S.instance, undefined);
  await transfer("unknown server", UNKNOWN, { userId: P.joe }, 404);
  await transfer("to the owner herself", S.handover, { userId: P.kim }, 400);
  await transfer("to a stranger", S.handover, { userId: P.maya }, 400);
  await transfer("to nobody", S.handover, {}, 400);
  await transfer("to null", S.handover, { userId: null });
  await transfer("to a word", S.handover, { userId: "ilya" });
  await transfer("to an upper-case id", S.handover, { userId: P.joe.toUpperCase() }, 204);
  await transfer("to Ilya", S.handover, { userId: P.ilya }, 204);
  await transfer("to Ilya again", S.handover, { userId: P.ilya }, 400);
  await step("members of the handed-over server", owner, "GET", cockpit(`/servers/${S.handover}/members`));
  await step("its audit, as Ilya reads it", who.ilya, "GET", "/api/admin/audit", { server: S.handover });
  const remove = (label, id, expect) => step(`delete: ${label}`, owner, "DELETE", cockpit(`/servers/${id}`), { expect });
  await remove("the instance", S.instance, 409);
  await remove("unknown", UNKNOWN, 404);
  await remove("Doomed", S.doomed, 204);
  await remove("Doomed again", S.doomed, 404);
  await remove("Ada's public server", S.open, 204);
  await step("servers: the list after", owner, "GET", cockpit("/servers"));
  await step("Kim's list after", who.kim, "GET", "/api/servers", { server: null });

  // ----- sessions
  out("# sessions");
  const desktop = await new Person("joe", DESKTOP).login();
  const sessions = (await step("sessions: the list", owner, "GET", cockpit("/sessions"))).json ?? [];
  const desktopSession = sessions.find((s) => s.device === DESKTOP && s.lastSeenAt === s.createdAt) ?? sessions.find((s) => s.device === DESKTOP);
  const revoke = (label, id, expect) => step(`revoke: ${label}`, owner, "DELETE", cockpit(`/sessions/${id}`), { expect });
  await revoke("own", ownSession, 409);
  await revoke("own in upper case", ownSession.toUpperCase(), 409);
  await revoke("Joe's desktop", desktopSession?.id, 204);
  await revoke("Joe's desktop again", desktopSession?.id, 404);
  await revoke("unknown", UNKNOWN, 404);
  await step("Joe's desktop session after", desktop, "GET", "/api/servers", { server: null, expect: 401 });
  await step("sessions: the list after", owner, "GET", cockpit("/sessions"));

  // ----- the panel's settings
  out("# settings");
  await step("settings: before anything was saved", owner, "GET", cockpit("/settings"));
  await step("activity: before anything was saved", owner, "GET", cockpit("/activity"));
  const base = { instanceName: "Stellarr", timeZone: "UTC", mailingEnabled: false, mailFromAddress: "", inviteMailEnabled: true, recoveryMailEnabled: true, alertMailEnabled: false, backupRetentionDays: 14, logRetentionDays: 30, orphanObjectRetentionDays: 1, automaticCleanupEnabled: true };
  const put = (label, body, expect) => step(`settings: ${label}`, owner, "PUT", cockpit("/settings"), { body, expect });
  const putRaw = (label, raw) => step(`settings: ${label}`, owner, "PUT", cockpit("/settings"), { raw, type: "application/json" });
  await put("as it is", base, 200);
  await put("an empty object", {}, 400);
  await put("only the name and the zone", { instanceName: "Stellarr", timeZone: "UTC" });
  for (const [label, value] of [["of 80", "i".repeat(80)], ["of 81", "i".repeat(81)], ["blank", "  "], ["padded", "  Home  "], ["of 40 astral letters (80 units)", "𝐈".repeat(40)], ["null", null], ["a number", 5]]) {
    await put(`name ${label}`, { ...base, instanceName: value });
  }
  for (const zone of ["Europe/Warsaw", "Pacific/Kiritimati", "Etc/GMT+12", "America/Argentina/ComodRivadavia", "Nowhere/City", "Central European Standard Time", "europe/warsaw", " UTC ", "", "../../etc/passwd", "UTC\u0000", "EST5EDT", "GMT", "Local", "Z", "+02:00", null]) {
    await put(`zone ${JSON.stringify(zone)}`, { ...base, timeZone: zone });
  }
  for (const field of ["backupRetentionDays", "logRetentionDays", "orphanObjectRetentionDays"]) {
    for (const value of [0, 1, 3650, 3651, -1]) await put(`${field} ${value}`, { ...base, [field]: value });
  }
  for (const [field, values] of [
    ["smtpPort", [0, 1, 65535, 65536, null, -1]],
    ["smtpSecurity", ["TLS", " none ", "starttls", "ssl", "", null]],
    ["smtpHost", ["h".repeat(255), "h".repeat(256), "  padded.example  ", "", null]],
    ["smtpUsername", ["u".repeat(320), "u".repeat(321), " user ", null]],
    ["trustedDeviceDays", [0, 1, 365, 366]],
    ["authChallengeMinutes", [0, 1, 60, 61]],
    ["emailCodeMinutes", [0, 1, 60, 61]],
    ["emailCodeCooldownSeconds", [-1, 0, 600, 601]],
    ["emailCodeMaxAttempts", [0, 1, 10, 11]],
    ["emailCodeSendsPerHour", [0, 1, 60, 61]],
    ["cliRecoveryMinutes", [0, 1, 60, 61, 10]],
    ["mfaRequirement", ["bogus", " OPTIONAL ", "", null]],
    ["resendEnabled", [true, false, null]],
    ["giphyEnabled", [true, false, null]],
  ]) {
    for (const value of values) await put(`${field} ${JSON.stringify(value)}`, { ...base, [field]: value });
  }
  for (const [label, enabled, address] of [
    ["mailing on, no sender", true, ""], ["mailing on, not an address", true, "not an address"], ["mailing off, not an address", false, "not an address"],
    ["mailing on, a display name", true, "Panel <panel@example.test>"], ["mailing on, padded", true, "  panel@example.test  "],
    ["mailing on, 320", true, `${"p".repeat(307)}@example.test`], ["mailing off, 321", false, `${"p".repeat(308)}@example.test`],
    ["mailing on, no dot", true, "panel@localhost"], ["mailing off, null", false, null],
  ]) {
    await put(label, { ...base, mailingEnabled: enabled, mailFromAddress: address });
  }
  await put("several wrong at once", { ...base, instanceName: "", timeZone: "Nowhere", smtpPort: 0, backupRetentionDays: 0, mfaRequirement: "x", cliRecoveryMinutes: 0 }, 400);
  await put("a number as a string", { ...base, backupRetentionDays: "15" });
  await put("a number with a zero fraction", { ...base, backupRetentionDays: 15.0 });
  await putRaw("a number written 15.0", JSON.stringify(base).replace('"backupRetentionDays":14', '"backupRetentionDays":15.0'));
  await put("a number with a fraction", { ...base, backupRetentionDays: 15.5 });
  await putRaw("a number in exponent form", JSON.stringify(base).replace('"backupRetentionDays":14', '"backupRetentionDays":1.5e1'));
  await put("a boolean as a string", { ...base, mailingEnabled: "false" });
  await put("a boolean as a number", { ...base, mailingEnabled: 0 });
  await put("a boolean null", { ...base, mailingEnabled: null });
  await put("a port as a string", { ...base, smtpPort: "25" });
  await put("the port back, after a value one server may have taken", { ...base, smtpPort: 587 }, 200);
  await put("the requirement a number", { ...base, mfaRequirement: 1 });
  await put("the public address", { ...base, publicUrl: "http://elsewhere.example" }, 200);
  await put("an unknown member", { ...base, surprise: { deep: [1] } }, 200);
  await putRaw("a member twice", JSON.stringify(base).replace('"instanceName":"Stellarr"', '"instanceName":"First","instanceName":"Second"'));
  await put("property names in another case", { ...Object.fromEntries(Object.entries(base).map(([k, v]) => [k[0].toUpperCase() + k.slice(1), v])), InstanceName: "Cased" });
  await put("members left out", { instanceName: "Partial", timeZone: "UTC", backupRetentionDays: 7, logRetentionDays: 7, orphanObjectRetentionDays: 7 });
  await step("settings: read back", owner, "GET", cockpit("/settings"));
  await put("a zone far east, and every field", { ...base, instanceName: "Home", timeZone: "Pacific/Kiritimati", inviteMailEnabled: false, recoveryMailEnabled: false, alertMailEnabled: true, backupRetentionDays: 30, logRetentionDays: 60, orphanObjectRetentionDays: 2, automaticCleanupEnabled: false, smtpHost: "mail.example.test", smtpPort: 2525, smtpSecurity: "TLS", smtpUsername: "mailer", resendEnabled: false, giphyEnabled: false, mfaRequirement: "optional", trustedDeviceDays: 30, authChallengeMinutes: 10, emailCodeMinutes: 15, emailCodeCooldownSeconds: 60, emailCodeMaxAttempts: 3, emailCodeSendsPerHour: 5, cliRecoveryMinutes: 15 }, 200);
  await step("settings: read back again", owner, "GET", cockpit("/settings"));
  await step("activity: in that zone", owner, "GET", cockpit("/activity"));
  await step("bootstrap: renamed", owner, "GET", cockpit("/bootstrap"));
  await put("back to UTC", { ...base, smtpSecurity: "starttls", smtpHost: "", smtpUsername: "" }, 200);

  // ----- desktop releases
  out("# desktop releases");
  await step("release: before", owner, "GET", cockpit("/desktop-release"));
  const release = (label, body, expect) => step(`release: ${label}`, owner, "PUT", cockpit("/desktop-release"), { body, expect });
  await release("plain", { minimumVersion: "1.2.3", recommendedVersion: "1.3.0" }, 200);
  await release("the same", { minimumVersion: "1.3.0", recommendedVersion: "1.3.0" }, 200);
  await release("minimum above recommended", { minimumVersion: "2.0.0", recommendedVersion: "1.9.9" }, 400);
  await release("both wrong", { minimumVersion: "1.2", recommendedVersion: "x" }, 400);
  await release("both wrong and minimum above", { minimumVersion: "9.0", recommendedVersion: "1.0.0" }, 400);
  for (const version of [" 1.2.3 ", "01.02.03", "1.2", "1.2.3.4", "v1.2.3", "1.2.3-beta", "999999999.0.0", "1000000000.0.0", "١.٢.٣", "1..3", "1.2.+3", "-1.2.3", "", null]) {
    await release(`version ${JSON.stringify(version)}`, { minimumVersion: "0.1.0", recommendedVersion: version });
  }
  await release("leading zeros compared as numbers", { minimumVersion: "1.10.0", recommendedVersion: "1.9.0" }, 400);
  await release("left out", {}, 400);
  await release("numbers", { minimumVersion: 1, recommendedVersion: 2 });
  await step("release: after", owner, "GET", cockpit("/desktop-release"));

  // ----- mail
  out("# mail");
  const mail = (label, body, expect) => step(`mail test: ${label}`, owner, "POST", cockpit("/mail/test"), { body, expect });
  const recipients = [
    ["plain", "ops@example.test"], ["upper case", "OPS@Example.TEST"], ["padded", "  ops@example.test  "], ["a display name", "Ops <ops@example.test>"],
    ["angle brackets", "<ops@example.test>"], ["quoted", "\"o p\"@example.test"], ["address literal", "ops@[127.0.0.1]"], ["no dot", "ops@localhost"],
    ["non-ASCII", "öps@exämple.test"], ["no at", "ops"], ["empty", ""], ["320", `${"o".repeat(307)}@example.test`], ["321", `${"o".repeat(308)}@example.test`],
    ["two", "a@example.test, b@example.test"], ["a comment", "ops@example.test (x)"], ["null", null],
  ];
  out("## mailing is off");
  for (const [label, to] of recipients) await mail(`off, ${label}`, { to });
  await mail("off, left out", {});
  await mail("off, a number", { to: 5 });
  await put("mail through the sink", { ...base, mailingEnabled: true, mailFromAddress: "panel@example.test", smtpHost: args["smtp-host"], smtpPort: Number(args["smtp-port"]), smtpSecurity: "none" }, 200);
  for (const [label, to] of recipients) await mail(`on, ${label}`, { to }, label === "plain" ? 200 : undefined);
  await put("mail to a closed port", { ...base, mailingEnabled: true, mailFromAddress: "panel@example.test", smtpHost: args["smtp-host"], smtpPort: Number(args["smtp-closed"]), smtpSecurity: "none" }, 200);
  const closed = await mail("a closed port", { to: "ops@example.test" }, 502);
  await put("mail to a server that refuses", { ...base, mailingEnabled: true, mailFromAddress: "panel@example.test", smtpHost: args["smtp-host"], smtpPort: Number(args["smtp-refusing"]), smtpSecurity: "none" }, 200);
  const refused = await mail("a server that refuses", { to: "ops@example.test" }, 502);
  await put("mail with no sender", { ...base, mailingEnabled: true, mailFromAddress: "", smtpHost: args["smtp-host"], smtpPort: Number(args["smtp-port"]), smtpSecurity: "none" }, 400);
  await put("mail through Resend, with no key", { ...base, mailingEnabled: true, mailFromAddress: "panel@example.test", resendEnabled: true }, 200);
  await mail("Resend without a key", { to: "ops@example.test" }, 409);
  await put("mail off again", { ...base, resendEnabled: false, smtpHost: "", smtpPort: 587, smtpSecurity: "starttls" }, 200);
  out(`~~ the closed port's detail: ${closed.json?.detail}`);
  out(`~~ the refusing server's detail: ${refused.json?.detail}`);

  // ----- integrations
  out("# integrations");
  await step("giphy: no key", owner, "POST", cockpit("/integrations/giphy/test"), { expect: 409 });
  await step("giphy: no key, with a body", owner, "POST", cockpit("/integrations/giphy/test"), { body: { key: "x" }, expect: 409 });
  await put("giphy on without a key", { ...base, giphyEnabled: true }, 200);
  await step("giphy: on, no key", owner, "POST", cockpit("/integrations/giphy/test"), { expect: 409 });
  await put("giphy off", { ...base, giphyEnabled: false }, 200);
  await step("providers: the list", owner, "GET", cockpit("/integrations/identity-providers"), { expect: 200 });
  for (const [label, id, body] of [["google", "google", { signUp: true }], ["example", "example", { signUp: false }], ["upper case", "GOOGLE", { signUp: true }], ["no sign-up", "google", {}], ["sign-up null", "google", { signUp: null }], ["sign-up a string", "google", { signUp: "true" }], ["an empty id", "%20", { signUp: true }]]) {
    await step(`providers: ${label}`, owner, "PUT", cockpit(`/integrations/identity-providers/${id}`), { body });
  }

  // ----- secrets
  out("# secrets");
  const secretsFrom = answers.length;
  await step("secrets: the list", owner, "GET", cockpit("/secrets"), { expect: 200 });
  const secret = (label, name, body, expect) => step(`secret: ${label}`, owner, "PUT", cockpit(`/secrets/${name}`), { body, expect });
  await secret("set", "mail.smtp-password", { value: SECRET_ONE }, 200);
  await secret("replaced", "mail.smtp-password", { value: SECRET_TWO }, 200);
  await secret("padded, kept as it is", "mail.smtp-password", { value: SECRET_PADDED }, 200);
  await secret("blank", "mail.smtp-password", { value: "   " }, 400);
  await secret("empty", "mail.smtp-password", { value: "" }, 400);
  await secret("null", "mail.smtp-password", { value: null }, 400);
  await secret("left out", "mail.smtp-password", {}, 400);
  await secret("a number", "mail.smtp-password", { value: 5 });
  await secret("4096", "mail.smtp-password", { value: "s".repeat(4096) }, 200);
  await secret("4097", "mail.smtp-password", { value: "s".repeat(4097) }, 400);
  await secret("2048 astral letters (4096 units)", "mail.smtp-password", { value: "𝐒".repeat(2048) }, 200);
  await secret("2048 astral letters and one more (4097 units)", "mail.smtp-password", { value: `${"𝐒".repeat(2048)}s` }, 400);
  await secret("unknown name", "nope", { value: "x" }, 404);
  await secret("a name in upper case", "MAIL.SMTP-PASSWORD", { value: "x" }, 404);
  await secret("the Resend key, padded", "mail.resend-api-key", { value: `  ${RESEND_KEY}  ` }, 200);
  await step("secrets: the list with two", owner, "GET", cockpit("/secrets"), { expect: 200 });
  await step("settings: with two secrets", owner, "GET", cockpit("/settings"), { expect: 200 });
  await step("secret: clear the Resend key", owner, "DELETE", cockpit("/secrets/mail.resend-api-key"), { expect: 200 });
  await step("secret: clear the Resend key again", owner, "DELETE", cockpit("/secrets/mail.resend-api-key"), { expect: 200 });
  await step("secret: clear an unknown one", owner, "DELETE", cockpit("/secrets/nope"), { expect: 404 });
  await secret("set again", "mail.smtp-password", { value: SECRET_ONE }, 200);
  await step("secrets: the list at the end", owner, "GET", cockpit("/secrets"), { expect: 200 });

  // ----- the log
  out("# the log");
  await step("log: a wrong password", new Person("owner"), "POST", "/api/auth/login", { body: { username: "owner", password: WRONG_PASSWORD } });
  await step("log: a token in a query", owner, "GET", cockpit(`/users?access_token=${QUERY_TOKEN}&code=${QUERY_TOKEN}`), { wait: 0 });
  await step("log: a path holding markup and control characters", anonymous, "GET", "/api/%3Cscript%3Ealert(1)%3C/script%3E%0D%0Aforged%09line", { wait: 0 });
  await step("log: and a null character", anonymous, "GET", "/api/%3Cb%3Enull%3C/b%3E%00end", { wait: 0 });
  await logs(owner);

  // ----- the process
  out("# the process");
  const runtime = await step("runtime", owner, "GET", cockpit("/runtime"), { shape: true, wait: 0 });
  if (runtime.json) {
    for (const service of runtime.json.services ?? []) out(`  service ${service.id}: ${service.name} | ${service.state} | ${service.id === "api" ? "<the platform>" : scrub(service.detail)}`);
    out(`  realtime connections ${runtime.json.realtimeConnections}, voice rooms ${runtime.json.voiceRooms}, sample seconds ${runtime.json.sampleSeconds}`);
  }

  // ----- limits
  out("# limits");
  for (const [label, method, path, body] of [["the list of accounts", "GET", "/users"], ["a secret nobody has", "PUT", "/secrets/nope", { value: "x" }], ["the log", "GET", "/logs?limit=1"]]) {
    const statuses = await Promise.all(Array.from({ length: 60 }, () => owner.send(method, cockpit(path), { body }).then((r) => r.status)));
    const counts = {};
    for (const status of statuses) counts[status] = (counts[status] ?? 0) + 1;
    out(`## 60 quick requests, ${label}: ${JSON.stringify(counts)}`);
  }

  // ----- what never comes back
  out("# secrets in later answers");
  const later = answers.slice(secretsFrom).join("\n");
  for (const [label, value] of [["the first SMTP password", SECRET_ONE], ["the second", SECRET_TWO], ["the padded one", SECRET_PADDED.trim()], ["the Resend key", RESEND_KEY], ["the wrong password", WRONG_PASSWORD], ["the token in the query", QUERY_TOKEN]]) {
    out(`## ${label} in an answer: ${later.includes(value)}`);
  }
  const heardText = Object.values(listeners).map((l) => l.heard.join("\n")).join("\n");
  out(`## a secret on the hub: ${[SECRET_ONE, SECRET_TWO, RESEND_KEY].some((v) => heardText.includes(v))}`);

  // ----- what Rue does with her credential, last but one, since it signs her in and adds to every count
  out("# the recovered account signs in");
  // What Rue does with the credential: redeem it, enrol an authenticator, and is signed in.
  const redeem = (label, person, body) => step(`redeem: ${label}`, person, "POST", "/api/auth/recovery/redeem", { body });
  const rue = new Person("rue");
  await redeem("the first credential, replaced", rue, { credential: rueFirst.json?.credential });
  await redeem("too short", rue, { credential: "short" });
  await redeem("Lou's, with keepSignedIn false", new Person("lou"), { credential: first.json?.credential, keepSignedIn: false });
  const redeemed = await redeem("the second credential", rue, { credential: ` ${rueSecond.json?.credential}\n` });
  await redeem("the second credential again", new Person("rue"), { credential: rueSecond.json?.credential });
  const challengeId = redeemed.json?.challengeId ?? redeemed.json?.challenge?.id;
  const started = await step("enrol an authenticator", rue, "POST", "/api/auth/mfa/enroll/totp/start", { body: { challengeId, name: "New phone" } });
  await step("confirm a wrong code", rue, "POST", "/api/auth/mfa/enroll/totp/confirm", { body: { challengeId, methodId: started.json?.methodId, code: "000000" } });
  await step("confirm the code", rue, "POST", "/api/auth/mfa/enroll/totp/confirm", { body: { challengeId, methodId: started.json?.methodId, code: totp(started.json?.secret ?? "") }, expect: 200 });
  await step("Rue is signed in", rue, "GET", "/api/session", { server: null });
  await security("Rue signed in again", P.rue, 200);

  // ----- last: the owner, who has no second step, asks the panel for one
  out("# the owner turns on the panel's second step");
  await put("require it for the panel", { ...base, mfaRequirement: "cockpit" }, 200);
  await step("bootstrap after", owner, "GET", cockpit("/bootstrap"), { expect: 403 });
  await step("fresh second step after", owner, "GET", cockpit("/fresh-second-step"), { expect: 403 });
  await put("turn it off again", { ...base, mfaRequirement: "optional" }, 403);
  await step("a broken body after", owner, "PUT", cockpit("/settings"), { raw: "{", type: "application/json" });
  await step("Maya after", who.maya, "GET", cockpit("/bootstrap"), { expect: 403 });
  await step("the owner's own server list after", owner, "GET", "/api/servers", { server: null, expect: 200 });

  out("# moments");
  out(`## fresh moments: ${momentStats.fresh > 0 ? "some" : "none"}; with a Z: ${momentStats.zoned > 0}; without: ${momentStats.plain > 0}; a fraction ending in 0: ${momentStats.endsInZero > 0}`);
  out(`## the longest fraction of each member: ${[...momentStats.longest].sort().map(([k, v]) => `${k} ${v}`).join(", ")}`);
  out(`## steps the scenario did not expect: ${unexpected}`);
  writeFileSync(args.out, lines.join("\n") + "\n");
  for (const listener of Object.values(listeners)) listener.socket.close();
  process.exit(0);
}

/** The log of each server is its own: what is compared is the shape of an answer and what the filters do. */
async function logs(owner) {
  const ask = (query) => owner.get(cockpit(`/logs${query}`));
  const total = (json) => json.levels.info + json.levels.warn + json.levels.error;
  const report = (label, text) => { out(`## log ${label}`); out(`  ${scrub(text)}`); };
  const all = await ask("?limit=500");
  report("the shape", `${all.status} ${shapeOf(all.text)}`);
  report("capacity", String(all.json.capacity));
  for (const [label, limit, clamped] of [["0", "0", 1], ["1", "1", 1], ["2", "2", 2], ["500", "500", 500], ["501", "501", 500], ["-1", "-1", 1], ["empty", "", 100], ["left out", null, 100], ["abc", "abc"], ["1.5", "1.5"], ["2^31", "2147483648"], ["twice", "1&limit=2"]]) {
    const r = await ask(limit === null ? "" : `?limit=${limit}`);
    if (r.status !== 200 || clamped === undefined) {
      report(`limit ${label}`, `${r.status} ${r.status === 200 ? `${r.json.entries.length} entries` : r.text}`);
      continue;
    }
    const n = r.json.entries.length;
    const expected = Math.min(clamped, total(r.json));
    report(`limit ${label}`, `entries ${n === expected ? "= min(limit, lines)" : `${n}, not ${expected}`}; nextBefore ${r.json.nextBefore === null ? "null" : "set"} (${(r.json.nextBefore !== null) === total(r.json) > clamped ? "as it should" : "wrongly"})`);
  }
  const page = await ask("?limit=5");
  const last = page.json.entries.at(-1).sequence;
  const next = await ask(`?limit=5&before=${page.json.nextBefore}`);
  report("before the first page's cursor", `${next.status}; nextBefore is the last sequence: ${page.json.nextBefore === last}; continues below it: ${next.json.entries.every((e) => e.sequence < last)}; newest first: ${next.json.entries.every((e, i, a) => i === 0 || a[i - 1].sequence > e.sequence)}`);
  for (const [label, before] of [["0", "0"], ["1", "1"], ["-5", "-5"], ["abc", "abc"], ["huge", "9223372036854775807"], ["beyond 64 bits", "9223372036854775808"], ["empty", ""]]) {
    const r = await ask(`?before=${before}`);
    report(`before ${label}`, `${r.status} ${r.status === 200 ? `${r.json.entries.length > 0 ? "entries" : "no entries"}` : r.text}`);
  }
  for (const level of ["info", "warn", "error", "info,warn", "INFO", " warn , error ", "bogus", "info,bogus", ",,", ""]) {
    const unfiltered = (await ask("")).json.levels;
    const r = await ask(`?limit=500&level=${encodeURIComponent(level)}`);
    const wanted = level.split(",").map((x) => x.trim().toLowerCase()).filter((x) => ["info", "warn", "error"].includes(x));
    const every = r.json.entries.every((e) => wanted.length === 0 || wanted.includes(e.level));
    const kept = r.json.entries.length === Math.min(500, wanted.length === 0 ? total(r.json) : wanted.reduce((sum, k) => sum + r.json.levels[k], 0));
    // One more request line between the two answers.
    const ignored = r.json.levels.warn === unfiltered.warn && r.json.levels.error === unfiltered.error && r.json.levels.info - unfiltered.info <= 1;
    report(`level ${JSON.stringify(level)}`, `${r.status}; every entry of the level: ${every}; entries = the level's count: ${kept}; level counts ignore the level: ${ignored}`);
  }
  const source = all.json.sources[0].name;
  for (const [label, value] of [["the busiest", source], ["padded", `  ${source}  `], ["in upper case", source.toUpperCase()], ["unknown", "Nobody.Here"]]) {
    const r = await ask(`?limit=500&source=${encodeURIComponent(value)}`);
    report(`source ${label}`, `${r.status}; entries ${r.json.entries.length > 0 ? "some" : "none"}; every one from it: ${r.json.entries.every((e) => e.source === source)}; levels count only it: ${total(r.json) === r.json.entries.length || r.json.entries.length === 500}; the sources list is not filtered: ${r.json.sources.length > 1}`);
  }
  report("sources", `${all.json.sources.length <= 50 ? "at most 50" : "more than 50"}; busiest first: ${all.json.sources.every((s, i, a) => i === 0 || a[i - 1].count >= s.count)}`);
  const markup = await ask(`?limit=500&search=${encodeURIComponent("forged")}`);
  const upper = await ask(`?limit=500&search=${encodeURIComponent("  FORGED ")}`);
  report("search for the markup path", `${markup.status}; found: ${markup.json.entries.length}; the same in upper case and padded: ${upper.json.entries.length === markup.json.entries.length}`);
  for (const entry of markup.json.entries) {
    const raw = markup.text.slice(markup.text.indexOf('"message":', markup.text.indexOf(`"sequence":${entry.sequence}`)));
    report("  the line as sent", `${entry.level} | ${raw.slice(0, raw.indexOf('","traceId"') + 1).replace(/^"message":/, "")}`);
  }
  const blank = await ask("?limit=500&search=%20%20");
  report("search blank", `${blank.status}; the same as no search: ${blank.json.entries.length === Math.min(500, total(blank.json))}`);
  const everything = (await ask("?limit=500")).text;
  const exported = await owner.get(cockpit("/logs/export"));
  out("## log export");
  out(describeTsv(exported));
  const rows = exported.text.split("\n").slice(1).filter(Boolean);
  const moments = rows.map((row) => row.split("\t")[0]);
  report("export order", `oldest first: ${moments.every((m, i) => i === 0 || moments[i - 1] <= m)}; moments ${[...new Set(moments.map((m) => shapeOf(JSON.stringify(m))))].join(" ")}; levels ${[...new Set(rows.map((r) => r.split("\t")[1]))].sort().join(",")}`);
  // The export writes moments in a fixed width (the round-trip format): seven digits and a Z, every one.
  report("export moments", `every one of seven digits and a Z: ${moments.every((m) => /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{7}Z$/.test(m))}; example shape ${moments[0]?.replace(/\d/g, "9")}`);
  const markupRows = rows.filter((row) => row.includes("forged"));
  report("export of the markup path", markupRows.map((row) => row.split("\t").slice(1, 2).concat(row.split("\t").slice(4)).join(" | ")).join(" / "));
  for (const [label, query] of [["errors", "?level=error"], ["warnings and errors", "?level=warn,error"], ["a search", "?search=forged"], ["an unknown source", "?source=Nobody"], ["a limit, ignored", "?limit=1"]]) {
    const r = await owner.get(cockpit(`/logs/export${query}`));
    const kept = r.text.split("\n").slice(1).filter(Boolean);
    report(`export ${label}`, `${r.status}; rows ${kept.length > 1 ? "several" : kept.length}; levels ${[...new Set(kept.map((row) => row.split("\t")[1]))].sort().join(",")}`);
  }
  for (const [label, value] of [["the wrong password", WRONG_PASSWORD], ["the first SMTP password", SECRET_ONE], ["the second", SECRET_TWO], ["the Resend key", RESEND_KEY], ["the token in the query", QUERY_TOKEN]]) {
    const searched = await ask(`?limit=500&search=${encodeURIComponent(value)}`);
    report(`${label} in the log`, `search finds ${searched.json.entries.length}; in a page: ${everything.includes(value)}; in the export: ${exported.text.includes(value)}`);
  }
}

// ---------------------------------------------------------------------------
// The second step: sessions made before the database is edited, asked afterwards.

const STALE = [["owner-fresh", "owner"], ["owner-old", "owner"], ["owner-stale", "owner"], ["maya", "maya"], ["joe", "joe"]];

async function staleLogin() {
  const jars = {};
  for (const [key, name] of STALE) {
    const person = await new Person(name, `cockpit-${key}`).login();
    jars[key] = { name, id: person.id, agent: person.agent, cookies: [...person.cookies] };
  }
  writeFileSync(args.out, JSON.stringify(jars));
}

/**
 * The owner, Maya and Joe have an authenticator. One of the owner's sessions
 * passed the step now, one ten minutes ago and one never; Joe's now, Maya's
 * never.
 */
function staleEdit() {
  const db = new DatabaseSync(args.db);
  const method = db.prepare("INSERT INTO MfaMethods (Id, UserId, Type, Name, CreatedAt, ConfirmedAt) SELECT upper(hex(randomblob(4))) || '-0000-4000-8000-' || upper(hex(randomblob(6))), Id, 'totp', 'Phone', '2026-11-20 10:00:00', '2026-11-20 10:00:00' FROM Users WHERE Username IN ('owner', 'maya', 'joe')").run();
  if (method.changes !== 3) throw new Error("the methods were not added");
  const at = (offset) => db.prepare(`SELECT strftime('%Y-%m-%d %H:%M:%f', 'now', '${offset}') AS at`).get().at;
  edit(db, "UPDATE UserSessions SET MfaCompletedAt = ? WHERE Device = 'cockpit-owner-fresh'", at("+0 seconds"));
  edit(db, "UPDATE UserSessions SET MfaCompletedAt = ? WHERE Device = 'cockpit-owner-old'", at("-600 seconds"));
  edit(db, "UPDATE UserSessions SET MfaCompletedAt = NULL WHERE Device = 'cockpit-owner-stale'");
  edit(db, "UPDATE UserSessions SET MfaCompletedAt = ? WHERE Device = 'cockpit-joe'", at("+0 seconds"));
  edit(db, "UPDATE UserSessions SET MfaCompletedAt = NULL WHERE Device = 'cockpit-maya'");
  db.close();
}

async function staleAsk() {
  state = JSON.parse(readFileSync(args.state, "utf8"));
  nameIds();
  readSeededMoments();
  const jars = JSON.parse(readFileSync(args.cookies, "utf8"));
  const who = {};
  for (const [key, jar] of Object.entries(jars)) {
    who[key] = new Person(jar.name, jar.agent);
    who[key].id = jar.id;
    who[key].cookies = new Map(jar.cookies);
    who[key].server = state.servers.instance;
    // A session is asked as it was made, whatever an answer says to the cookie.
    who[key].fixedJar = true;
  }
  who.anonymous = new Person("anonymous");
  const fresh = who["owner-fresh"];
  const P = state.people;
  const S = state.servers;
  const ask = (label, person, method, path, options = {}) => step(label, who[person], method, path, { ...options, wait: 0 });
  await ask("fresh at the start", "owner-fresh", "GET", cockpit("/fresh-second-step"), { expect: 204 });
  const sessions = (await fresh.get(cockpit("/sessions"))).json ?? [];
  for (const s of sessions) if (s.device.startsWith("cockpit-")) names.set(s.id.toLowerCase(), `session:${s.device}`);
  // Revoking a session needs no fresh step: a real id here would let the
  // stale sessions end the fresh one, so the session of the matrix is nobody's.
  const ownSession = UNKNOWN;

  // Every guarded route, every shape, every kind of session.
  const callers = Object.fromEntries(["owner-fresh", "owner-old", "owner-stale", "maya", "joe", "anonymous"].map((key) => [key, who[key]]));
  await refusals(callers, state, ownSession);
  // Routes that need no fresh step, with a session that has none.
  for (const [method, path, options = {}] of everyRoute(state)) {
    if (["/runtime", "/logs", "/logs/export"].includes(path)) continue;
    await ask(`stale owner ${method} ${path}`, "owner-stale", method, cockpit(path), options);
  }

  // The settings page: only who may skip the step, and where mail goes, need a fresh one.
  out("# settings with a stale second step");
  const base = { instanceName: "Stellarr", timeZone: "UTC", mailingEnabled: false, mailFromAddress: "", inviteMailEnabled: true, recoveryMailEnabled: true, alertMailEnabled: false, backupRetentionDays: 14, logRetentionDays: 30, orphanObjectRetentionDays: 1, automaticCleanupEnabled: true };
  const put = (label, person, body) => ask(`settings ${label}: ${person}`, person, "PUT", cockpit("/settings"), { body });
  for (const person of ["owner-stale", "owner-old"]) {
    await put("plain fields", person, { ...base, instanceName: "Stale Home" });
    await put("the requirement, the same value", person, { ...base, mfaRequirement: "optional" });
    for (const field of ["trustedDeviceDays", "authChallengeMinutes", "emailCodeMinutes", "emailCodeCooldownSeconds", "emailCodeMaxAttempts", "emailCodeSendsPerHour", "cliRecoveryMinutes"]) {
      await put(`${field}, the same value`, person, { ...base, [field]: { trustedDeviceDays: 14, authChallengeMinutes: 5, emailCodeMinutes: 8, emailCodeCooldownSeconds: 45, emailCodeMaxAttempts: 5, emailCodeSendsPerHour: 10, cliRecoveryMinutes: 10 }[field] });
    }
    await put("the SMTP host, unchanged", person, { ...base, smtpHost: "" });
    await put("the SMTP host, padded but the same", person, { ...base, smtpHost: "  " });
    await put("the SMTP host, changed", person, { ...base, smtpHost: "evil.example" });
    await put("the SMTP port, unchanged", person, { ...base, smtpPort: 587 });
    await put("the SMTP port, changed", person, { ...base, smtpPort: 25 });
    await put("the SMTP security, the same in upper case", person, { ...base, smtpSecurity: " STARTTLS " });
    await put("the SMTP security, changed", person, { ...base, smtpSecurity: "none" });
    await put("the SMTP user, unchanged", person, { ...base, smtpUsername: "" });
    await put("the SMTP user, changed", person, { ...base, smtpUsername: "evil" });
    await put("Resend, unchanged", person, { ...base, resendEnabled: false });
    await put("Resend, changed", person, { ...base, resendEnabled: true });
    await put("GIPHY, changed", person, { ...base, giphyEnabled: true });
    await put("a security field and a plain one", person, { ...base, instanceName: "Should Not Stick", mfaRequirement: "optional" });
    await put("a security field that is wrong", person, { ...base, mfaRequirement: "bogus" });
    await put("a plain field that is wrong and a security field", person, { ...base, instanceName: "", trustedDeviceDays: 3 });
    await ask(`settings read back: ${person}`, person, "GET", cockpit("/settings"));
  }
  await put("the requirement", "joe", { ...base, mfaRequirement: "everyone" });

  // The policy that asks the panel itself for a second step, set by the owner whose step is fresh.
  // "everyone" last: the first server signs a session without the step out under it.
  for (const requirement of ["cockpit", "admins", "everyone"]) {
    out(`# the requirement ${requirement}`);
    await put(`require ${requirement}`, "owner-fresh", { ...base, mfaRequirement: requirement });
    for (const [method, path, options = {}] of [["GET", "/bootstrap"], ["GET", "/users"], ["GET", "/fresh-second-step"], ["PUT", "/settings", { body: base }], ["PUT", "/settings", { raw: "{", type: "application/json" }], ["PUT", "/settings", { raw: "{}", type: "text/plain" }], ["PATCH", `/users/${UNKNOWN}/status`, { body: { disabled: true } }], ["PATCH", "/users/x/status", { body: { disabled: true } }], ["DELETE", `/sessions/${UNKNOWN}`], ["POST", "/integrations/giphy/test"]]) {
      for (const person of ["owner-fresh", "owner-old", "owner-stale", "maya", "joe", "anonymous"]) {
        await ask(`${requirement}: ${method} ${path}${options.raw !== undefined ? ` ${options.type} ${options.raw}` : ""}: ${person}`, person, method, cockpit(path), options);
      }
    }
    await ask(`${requirement}: the owner's server list, stale`, "owner-stale", "GET", "/api/servers", { server: null });
    await ask(`${requirement}: Joe's server list`, "joe", "GET", "/api/servers", { server: null });
  }
  await put("back to optional", "owner-fresh", { ...base, mfaRequirement: "optional" });
  await ask("fresh at the end", "owner-fresh", "GET", cockpit("/fresh-second-step"), { expect: 204 });
  out(`## steps the scenario did not expect: ${unexpected}`);
  writeFileSync(args.out, lines.join("\n") + "\n");
}

// ---------------------------------------------------------------------------
// The second step of an authenticator app, as RFC 6238 computes it.

function totp(secret, at = Date.now()) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let bits = "";
  for (const c of secret.replace(/=+$/, "").toUpperCase()) bits += alphabet.indexOf(c).toString(2).padStart(5, "0");
  const key = Buffer.from((bits.match(/.{8}/g) ?? []).map((b) => parseInt(b, 2)));
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(at / 30000)));
  const mac = createHmac("sha1", key).update(counter).digest();
  const offset = mac[mac.length - 1] & 15;
  return String((mac.readUInt32BE(offset) & 0x7fffffff) % 1000000).padStart(6, "0");
}

// ---------------------------------------------------------------------------
// The mail the two servers sent, side by side, and the two transcripts.

/**
 * A message as a reader gets it: its headers unfolded, in the order sent and
 * then as a set, and each text part decoded from its transfer encoding.
 */
function readMail(text) {
  const raw = text.replace(/\r\n/g, "\n");
  const split = (entity) => {
    const at = entity.indexOf("\n\n");
    return at < 0 ? [entity, ""] : [entity.slice(0, at), entity.slice(at + 2)];
  };
  const [head] = split(raw);
  const headers = head.replace(/\n[ \t]+/g, " ").split("\n");
  // Every leaf of the message, nested multiparts opened, in order.
  const leaves = (entity) => {
    const [partHead, partBody] = split(entity);
    const lines = partHead.replace(/\n[ \t]+/g, " ").split("\n");
    const type = (lines.find((l) => /^content-type:/i.test(l)) ?? "Content-Type: text/plain").replace(/\s+/g, " ");
    const boundary = /boundary="?([^";\s]+)"?/i.exec(type)?.[1];
    if (/multipart\//i.test(type) && boundary) {
      const inner = partBody.split(`--${boundary}`).slice(1, -1).map((p) => p.replace(/^\n/, ""));
      return [{ type: type.replace(/;?\s*boundary="?[^";\s]+"?/i, "").replace(/; type="[^"]*"/i, (m) => m), encoding: "", content: `(${inner.length} parts)` }, ...inner.flatMap(leaves)];
    }
    let content = partBody;
    const encoding = lines.find((l) => /^content-transfer-encoding:/i.test(l)) ?? "(no transfer encoding)";
    if (/quoted-printable/i.test(encoding)) {
      content = Buffer.from(content.replace(/=\n/g, "").replace(/=([0-9A-F]{2})/gi, (_, h) => String.fromCharCode(parseInt(h, 16))), "latin1").toString("utf8");
    } else if (/base64/i.test(encoding)) {
      content = Buffer.from(content.replace(/\s+/g, ""), "base64").toString("utf8");
    }
    return [{ type, encoding, content: content.replace(/\n+$/, "") }];
  };
  const clean = (line) => line.replace(/^(Message-I[Dd]|Date):.*$/i, "$1: <by nature>").replace(/;?\s*boundary="?[^";\s]+"?/i, "");
  return { headers: headers.map(clean), names: headers.map((h) => h.split(":")[0].toLowerCase()), parts: leaves(raw).slice(1) };
}

function mails() {
  const read = (folder) => readdirSync(folder).filter((f) => f.endsWith(".eml")).sort().map((f) => readMail(readFileSync(`${folder}/${f}`, "utf8")));
  const first = read(args.first);
  const second = read(args.this);
  console.log(`${first.length} messages from the first server, ${second.length} from this one`);
  const port = (text) => text.replace(/host\.docker\.internal:\d+|127\.0\.0\.1:\d+/g, "<smtp>");
  for (let i = 0; i < Math.max(first.length, second.length); i++) {
    const a = first[i];
    const b = second[i];
    if (!a || !b) { console.log(`message ${i + 1}: only on ${a ? "the first server" : "this one"}: ${(a ?? b).headers.find((h) => /^to:/i.test(h))}`); continue; }
    const notes = [];
    const setA = [...a.headers].map(port).sort();
    const setB = [...b.headers].map(port).sort();
    for (const h of setA) if (!setB.some((x) => x.toLowerCase() === h.toLowerCase())) notes.push(`  header only on the first: ${JSON.stringify(h)}`);
    for (const h of setB) if (!setA.some((x) => x.toLowerCase() === h.toLowerCase())) notes.push(`  header only on this one : ${JSON.stringify(h)}`);
    if (a.names.join(",") !== b.names.join(",")) notes.push(`  header order: first ${a.names.join(",")} | this ${b.names.join(",")}`);
    for (let j = 0; j < Math.max(a.parts.length, b.parts.length); j++) {
      const x = a.parts[j];
      const y = b.parts[j];
      if (!x || !y) { notes.push(`  part ${j + 1} only on ${x ? "the first" : "this one"}`); continue; }
      if (x.type.toLowerCase() !== y.type.toLowerCase()) notes.push(`  part ${j + 1} type: first ${x.type} | this ${y.type}`);
      if (x.encoding.toLowerCase() !== y.encoding.toLowerCase()) notes.push(`  part ${j + 1} transfer encoding: first ${x.encoding} | this ${y.encoding}`);
      if (port(x.content) !== port(y.content)) {
        const p = port(x.content);
        const q = port(y.content);
        let k = 0;
        while (k < p.length && p[k] === q[k]) k++;
        notes.push(`  part ${j + 1} text differs at ${k}: first ${JSON.stringify(p.slice(Math.max(0, k - 60), k + 100))} | this ${JSON.stringify(q.slice(Math.max(0, k - 60), k + 100))}`);
      }
    }
    console.log(notes.length === 0 ? `message ${i + 1}: the same` : `message ${i + 1}:\n${notes.join("\n")}`);
  }
}

/**
 * Two transcripts step by step: the label of each step that differs, and for
 * each line of it that differs, the stretch around the first difference.
 */
function compare() {
  const stepsOf = (path) => {
    const steps = [];
    for (const line of readFileSync(path, "utf8").split("\n")) {
      if (line.startsWith("#")) steps.push([line]);
      else if (steps.length > 0 && line !== "") steps.at(-1).push(line);
    }
    return steps;
  };
  const a = stepsOf(args.first);
  const b = stepsOf(args.this);
  const span = (x, y) => {
    let i = 0;
    while (i < x.length && x[i] === y[i]) i++;
    let j = 0;
    while (j < Math.min(x.length, y.length) - i && x[x.length - 1 - j] === y[y.length - 1 - j]) j++;
    const from = Math.max(0, i - 120);
    return [x.slice(from, x.length - j + 30).slice(0, 900), y.slice(from, y.length - j + 30).slice(0, 900)];
  };
  // Differences that are known, by kind, and why: the script's exit status counts the others.
  const KNOWN = [
    ["decided: a cached answer is refused for the credential and the log export (cache-control: no-store)", (x, y) => y.some((l) => /cache-control: no-store/.test(l)) && !x.some((l) => /cache-control: no-store/.test(l)) && x.join("\n") === strip(y)],
    ["decided: any change to the mail settings asks for a fresh second step", (x, y) => /^## (settings|stale)/.test(x[0]) && /^\s*200/.test(x[1] ?? "") && /fresh-mfa-required/.test(y[1] ?? "")],
    ["accepted: an id that is none is a 404 before the caller is asked who they are, here a 401 after", (x, y) => /^\s*404/.test(x[1] ?? "") && /^\s*401/.test(y[1] ?? "")],
    ["accepted: characters outside the basic plane are written whole, the first server wrote surrogate pairs", (x) => /\\uD[89AB][0-9A-F]{2}\\uD[C-F][0-9A-F]{2}/.test(x.join("\n"))],
    ["accepted: member names of a body in another letter case", (x) => /another case/.test(x[0])],
    ["accepted: a Windows time zone name", (x) => /Central European/.test(x[0])],
    ["accepted: a test mail to an address the SMTP client cannot write, and the wording of mail failures", (x) => /^## (mail test|settings: mail off again)/.test(x[0])],
  ];
  const strip = (lines) => lines.filter((l) => !/cache-control: no-store/.test(l)).join("\n");
  const kinds = new Map();
  let unknown = 0;
  let differing = 0;
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? ["<missing>"];
    const y = b[i] ?? ["<missing>"];
    if (x.join("\n") === y.join("\n")) continue;
    differing++;
    const kind = KNOWN.find(([, applies]) => applies(x, y));
    if (kind) {
      kinds.set(kind[0], (kinds.get(kind[0]) ?? 0) + 1);
    } else {
      unknown++;
    }
    console.log(x[0]);
    if (x[0] !== y[0]) console.log(`  (this one: ${y[0]})`);
    for (let k = 0; k < Math.max(x.length, y.length); k++) {
      if (k === 0 || x[k] === y[k]) continue;
      const [p, q] = span(x[k] ?? "<none>", y[k] ?? "<none>");
      console.log(`  first: ${p}`);
      console.log(`  this : ${q}`);
    }
  }
  console.log(differing === 0 ? "the transcripts are the same" : `${differing} of ${a.length} steps differ`);
  for (const [kind, count] of kinds) console.log(`  known (${count}): ${kind}`);
  console.log(`  unknown: ${unknown}`);
  if (unknown > 0) process.exitCode = 1;
}

/** A mail server that answers every command with a refusal. */
function smtpRefuse() {
  createServer((socket) => {
    socket.on("error", () => {});
    socket.write("220 refusing ready\r\n");
    socket.on("data", (data) => {
      for (const line of String(data).split("\r\n").filter(Boolean)) {
        const command = line.slice(0, 4).toUpperCase();
        if (command === "EHLO" || command === "HELO") socket.write("250 refusing\r\n");
        else if (command === "QUIT") socket.end("221 bye\r\n");
        else socket.write("554 5.7.1 Refused for the comparison\r\n");
      }
    });
  }).listen(Number(args.port), "0.0.0.0");
}

if (mode === "seed") await seed();
if (mode === "prepare") prepare();
if (mode === "probe") await probe();
if (mode === "stale-login") await staleLogin();
if (mode === "stale-edit") staleEdit();
if (mode === "stale-ask") await staleAsk();
if (mode === "mails") mails();
if (mode === "compare") compare();
if (mode === "smtp-refuse") smtpRefuse();
