// The panel's backup routes of this server and of the first one, asked the
// same questions on copies of one database file, as transcripts to compare.
//
//   node backups-oracle.mjs seed  --base URL --setup CODE --state state.json
//   node backups-oracle.mjs login --base URL --state state.json --out jar.json
//   node backups-oracle.mjs ask   --base URL --state state.json --cookies jar.json --phase P --out transcript.txt
//
// Phases: plain (an owner with no second step: every question), stale (an
// owner and a member who have a second step this session did not pass),
// fresh (the owner passed one a minute ago), cockpit-open and cockpit-passed
// (the policy asks a second step of the panel, not passed / passed) and
// disabled (the owner's account is disabled). See backups-shared-database.sh.
// Needs Node 26.

import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { parseArgs } from "node:util";

const { values: args, positionals } = parseArgs({
  allowPositionals: true,
  options: { base: { type: "string" }, setup: { type: "string" }, state: { type: "string" }, out: { type: "string" }, cookies: { type: "string" }, phase: { type: "string" }, secrets: { type: "string" } },
});
const mode = positionals[0];
const base = args.base;
const PASSWORD = "Corona!2026-long";
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
const PEOPLE = ["owner", "maya", "kim", "ada"];
const BINARY = /zip|sqlite|octet|multipart/;

class Person {
  constructor(name) { this.name = name; this.cookies = new Map(); }
  async send(method, path, { body, raw, type, headers = {}, server } = {}) {
    const request = { method, headers: { ...clientHeaders, ...headers }, redirect: "manual" };
    if (this.cookies.size > 0) request.headers.Cookie = [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
    if (server) request.headers["X-Stellarr-Server"] = server;
    if (raw !== undefined) {
      if (type) request.headers["Content-Type"] = type;
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
      if (value === "") this.cookies.delete(pair.slice(0, at)); else this.cookies.set(pair.slice(0, at), value);
    }
    const bytes = Buffer.from(await response.arrayBuffer());
    return { status: response.status, headers: response.headers, bytes, text: bytes.toString("latin1") };
  }
  get(path, o) { return this.send("GET", path, o); }
  post(path, body, o) { return this.send("POST", path, { ...o, body }); }
  put(path, body, o) { return this.send("PUT", path, { ...o, body }); }
  delete(path, o) { return this.send("DELETE", path, o); }
  async login() {
    const r = await this.post("/api/auth/login", { username: this.name, password: PASSWORD });
    if (r.status !== 200) throw new Error(`signing in ${this.name}: ${r.status} ${r.text}`);
    this.id = JSON.parse(r.text).id ?? JSON.parse(r.text).user?.id;
    return this;
  }
}

function must(r, status, what) {
  if (r.status !== status) throw new Error(`${what}: ${r.status} ${r.text}`);
  return JSON.parse(r.text || "null");
}

async function seed() {
  const people = Object.fromEntries(PEOPLE.map((n) => [n, new Person(n)]));
  must(await people.owner.post("/api/auth/register", { username: "owner", displayName: "Owner", email: "owner@example.test", password: PASSWORD, setupCode: args.setup }), 201, "founding");
  await people.owner.login();
  for (const name of PEOPLE.slice(1)) {
    const invite = must(await people.owner.post("/api/admin/invites"), 201, "invite");
    must(await people[name].post("/api/auth/register", { username: name, displayName: name, email: `${name}@example.test`, password: PASSWORD, inviteCode: invite.url.split("/").pop() }), 201, name);
    await people[name].login();
  }
  must(await people.ada.post("/api/servers", { name: "Ada's", isPublic: false }), 201, "second server");
  writeFileSync(args.state, JSON.stringify({ people: Object.fromEntries(PEOPLE.map((n) => [n, people[n].id])) }));
}

async function login() {
  const jars = {};
  for (const name of PEOPLE) {
    const p = await new Person(name).login();
    jars[name] = { id: p.id, cookies: [...p.cookies] };
  }
  writeFileSync(args.out, JSON.stringify(jars));
}

// --- transcript ------------------------------------------------------------
const lines = [];
const names = new Map();
let fresh = 0;
const NOW = Date.now();
function scrub(text) {
  let r = text;
  r = r.replace(/backup-(\d{8})-(\d{6})-([0-9a-f]{32})/g, (m, d, t, tok) => (new Set(tok).size === 1 ? m : "backup-<stamp>-<token>"));
  r = r.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, (id) => {
    const key = id.toLowerCase();
    if (!names.has(key)) names.set(key, `id${++fresh}`);
    return `<${names.get(key)}>`;
  });
  // Moments are told apart only by age: fixtures are old, what the run makes is recent.
  r = r.replace(/("id":"backup-<stamp>-<token>\.\w+","sizeBytes":)\d+/g, "$1<size>");
  r = r.replace(/(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(\.\d+)?Z/g, (m) => (Math.abs(NOW - Date.parse(m)) < 3_600_000 ? "<recent>" : `<old ${m.slice(0, 10)}>`));
  r = r.replace(/"traceId":"[^"]*"/g, '"traceId":"<trace>"');
  return r;
}
/** The list of backups: files with the same moment come in the order the folder gives them, so the old ones are sorted. */
function orderTies(text) {
  const match = /^(\{"backups":\[)(.*?)(\],"retentionDays".*)$/s.exec(text);
  if (!match) return text;
  const items = match[2] === "" ? [] : match[2].split("},{").map((x, i, all) => (i > 0 ? "{" : "") + x + (i < all.length - 1 ? "}" : ""));
  const recent = (x) => Math.abs(NOW - Date.parse(/"createdAt":"([^"]*)"/.exec(x)[1])) < 3_600_000;
  const fresh = items.filter(recent);
  const old = items.filter((x) => !recent(x)).sort();
  return match[1] + [...fresh, ...old].join(",") + match[3];
}
/** The first server wrote every character outside ASCII (and a few inside) as \\uXXXX; this one writes them as they are. Both are read as the characters. */
function unescaped(text) {
  return text.replace(/\\u([0-9a-fA-F]{4})/g, (m, hex) => (["0022", "005C", "005c"].includes(hex) ? m : String.fromCharCode(parseInt(hex, 16))));
}
function describe(r) {
  const out = [String(r.status)];
  const binary = BINARY.test(r.headers.get("content-type") ?? "");
  // The first server sent a body of unknown length chunked; this one states it (a known difference).
  for (const n of ["content-type", "content-disposition", ...(binary ? ["content-length"] : []), "accept-ranges", "content-range", "location", "etag", "cache-control", "allow"]) {
    const v = r.headers.get(n);
    if (v) out.push(`${n}: ${v}`);
  }
  if (r.headers.get("last-modified")) out.push("last-modified: <date>");
  const type = r.headers.get("content-type") ?? "";
  if (BINARY.test(type) && r.bytes.length > 0) {
    let shown = r.text;
    if (type.includes("multipart")) shown = r.text.replace(/--[A-Za-z0-9_-]{8,}/g, "--<boundary>").replace(/boundary=[^\s;]+/g, "boundary=<b>");
    out.push(type.includes("multipart") ? `BODY: ${shown.length} bytes ${createHash("sha256").update(shown).digest("hex").slice(0, 12)}` : `BODY: ${r.bytes.length} bytes sha ${createHash("sha256").update(r.bytes).digest("hex").slice(0, 12)}`);
  } else out.push(`BODY: ${orderTies(unescaped(r.bytes.toString("utf8")))}`);
  return scrub(out.join(" | "));
}

async function ask() {
  const jars = JSON.parse(readFileSync(args.cookies, "utf8"));
  const who = {};
  for (const [name, jar] of Object.entries(jars)) { who[name] = new Person(name); who[name].id = jar.id; who[name].cookies = new Map(jar.cookies); }
  const anon = new Person("anon");
  who.anon = anon;
  const secrets = (args.secrets ?? "").split(",").filter(Boolean);
  const seen = [];
  const step = async (label, person, method, path, options) => {
    const r = await who[person].send(method, path, options);
    const text = describe(r);
    for (const s of secrets) if (r.text.includes(s)) seen.push(`${label}: a secret is in the answer`);
    lines.push(`## ${label}`, text);
    return r;
  };
  const B = "/api/cockpit/backups";
  const R2 = "/api/cockpit/integrations/cloudflare/r2";
  const SET = `${B}/settings`;
  const json = (type = "application/json") => type;
  const settle = async () => { for (let i = 0; i < 100; i++) { const r = await who.owner.get(B); if (r.status !== 200 || JSON.parse(r.text).job === null) return; await new Promise((x) => setTimeout(x, 50)); } };
  const FIX = "backup-20200101-000000-";
  const id = (c) => `${FIX}${c.repeat(32)}`;
  const GOOD = `${id("a")}.zip`;

  const gate = async (label, path, method = "GET", options) => {
    for (const p of ["anon", "maya", "ada", "kim", "owner"]) await step(`${label} [${p}]`, p, method, path, options);
  };

  if (args.phase === "plain") {
    await gate("gate: list", B);
    await gate("gate: settings", SET);
    await gate("gate: r2", R2);
    await gate("gate: verify", `${B}/${GOOD}/verify`, "POST");
    await gate("gate: download", `${B}/${GOOD}/download`);
    await gate("gate: create", B, "POST");

    await step("list: fixtures", "owner", "GET", B);
    await step("settings: before anything", "owner", "GET", SET);
    await step("r2: before anything", "owner", "GET", R2);

    // verify
    for (const [label, name] of [["good zip", GOOD], ["good db", `${id("b")}.db`], ["corrupt zip", `${id("c")}.zip`], ["truncated zip", `${id("d")}.zip`], ["lying manifest", `${id("e")}.zip`], ["bad database in zip", `${id("f")}.zip`], ["not a database", `${id("0")}.db`], ["no such", `${id("1")}.zip`], ["partial name", `.backup-x.zip.partial`], ["wrong extension", "backup-note.txt"], ["escape", "backup-%2e%2e%2fx.zip"], ["slash", "backup-x%2Fy.zip"], ["backslash", "backup-x%5Cy.zip"], ["unicode", "backup-%C5%82.zip"], ["not a zip", `${id("2")}.zip`], ["no manifest", `${id("3")}.zip`], ["missing database", `${id("4")}.zip`], ["key count lies", `${id("5")}.zip`], ["instance count lies", `${id("6")}.zip`]])
      await step(`verify: ${label}`, "owner", "POST", `${B}/${name}/verify`);
    await step("list: after verifying", "owner", "GET", B);

    // download
    for (const [label, name] of [["zip", GOOD], ["db", `${id("b")}.db`], ["no such", `${id("1")}.zip`], ["escape", "backup-%2e%2e%2fx.zip"]]) await step(`download: ${label}`, "owner", "GET", `${B}/${name}/download`);
    for (const [label, headers, method] of [
      ["head", {}, "HEAD"], ["first bytes", { Range: "bytes=0-9" }], ["suffix", { Range: "bytes=-5" }], ["open end", { Range: "bytes=100-" }],
      ["past the end", { Range: "bytes=99999999-" }], ["two ranges", { Range: "bytes=0-1,5-6" }], ["bad range", { Range: "bytes=x" }], ["backwards", { Range: "bytes=9-1" }],
      ["if-range wrong", { Range: "bytes=0-9", "If-Range": '"nope"' }], ["if-none-match", { "If-None-Match": '"x"' }], ["if-modified-since future", { "If-Modified-Since": "Wed, 01 Jan 2200 00:00:00 GMT" }],
      ["if-modified-since past", { "If-Modified-Since": "Wed, 01 Jan 2001 00:00:00 GMT" }], ["if-unmodified-since past", { "If-Unmodified-Since": "Wed, 01 Jan 2001 00:00:00 GMT" }],
    ]) await step(`download: ${label}`, "owner", method ?? "GET", `${B}/${GOOD}/download`, { headers });

    // create
    await step("create: no body", "owner", "POST", B);
    await step("create: empty json body", "owner", "POST", B, { raw: "", type: "application/json" });
    await step("create: null", "owner", "POST", B, { raw: "null", type: "application/json" });
    await step("create: {}", "owner", "POST", B, { body: {} });
    await step("create: array", "owner", "POST", B, { raw: "[]", type: "application/json" });
    await step("create: broken", "owner", "POST", B, { raw: "{", type: "application/json" });
    await step("create: text/plain with a valid body", "owner", "POST", B, { raw: '{"database":true,"keys":true}', type: "text/plain" });
    await step("create: no content type, valid body", "owner", "POST", B, { raw: '{"keys":true}' });
    await step("create: strings for booleans", "owner", "POST", B, { raw: '{"database":"true"}', type: "application/json" });
    await settle();
    await step("create: database only", "owner", "POST", B, { body: { database: true } });
    await step("create: database false, rest false", "owner", "POST", B, { body: { database: false, objects: false } });
    await step("create: unknown members", "owner", "POST", B, { body: { database: true, extra: 1 } });
    await step("create: capital names", "owner", "POST", B, { body: { Database: true } });
    for (const parts of [{ objects: true }, { keys: true }, { instance: true }, { database: true, objects: true }, { database: true, keys: true, instance: true }, { database: true, objects: true, keys: true, instance: true }]) {
      await step(`create: ${JSON.stringify(parts)}`, "owner", "POST", B, { body: parts });
      await settle();
    }
    const [a, b] = await Promise.all([who.owner.post(B, { objects: true, keys: true }), who.owner.post(B, { objects: true, keys: true })]);
    lines.push("## create: two at once", [a.status, b.status].sort().join(","));
    await settle();
    await step("list: after creating", "owner", "GET", B);

    // settings
    const S = { enabled: true, intervalHours: 24, retryMinutes: 15, initialDelayMinutes: 5, retentionDays: 14 };
    await step("settings: put valid", "owner", "PUT", SET, { body: S });
    await step("settings: get after put", "owner", "GET", SET);
    await step("list: retention after put", "owner", "GET", B);
    for (const [member, values] of [["intervalHours", [0, 1, 8760, 8761, -1, 2147483647, 2147483648, "24", " 24 ", "x", 1.5, null, true, "+5", "-5", "05", "5.0", "", "0x5", "١٢", 5e0, 1e1, 24.0, [24], {}]], ["retryMinutes", [0, 1, 1440, 1441]], ["initialDelayMinutes", [-1, 0, 1440, 1441]], ["retentionDays", [0, 1, 3650, 3651, -5]]])
      for (const v of values) await step(`settings: ${member}=${JSON.stringify(v)}`, "owner", "PUT", SET, { body: { ...S, [member]: v } });
    await step("settings: all bad at once", "owner", "PUT", SET, { body: { enabled: true, intervalHours: 0, retryMinutes: 0, initialDelayMinutes: -1, retentionDays: 0 } });
    await step("settings: empty object", "owner", "PUT", SET, { body: {} });
    await step("settings: missing retention", "owner", "PUT", SET, { body: { enabled: true, intervalHours: 1, retryMinutes: 1, initialDelayMinutes: 0 } });
    await step("settings: enabled as string", "owner", "PUT", SET, { body: { ...S, enabled: "true" } });
    await step("settings: enabled null", "owner", "PUT", SET, { body: { ...S, enabled: null } });
    await step("settings: capital names", "owner", "PUT", SET, { raw: JSON.stringify({ Enabled: true, IntervalHours: 3, RetryMinutes: 3, InitialDelayMinutes: 3, RetentionDays: 3 }), type: "application/json" });
    await step("settings: array", "owner", "PUT", SET, { raw: "[]", type: "application/json" });
    await step("settings: null", "owner", "PUT", SET, { raw: "null", type: "application/json" });
    await step("settings: empty body", "owner", "PUT", SET, { raw: "", type: "application/json" });
    await step("settings: broken", "owner", "PUT", SET, { raw: "{", type: "application/json" });
    await step("settings: text/plain", "owner", "PUT", SET, { raw: JSON.stringify(S), type: "text/plain" });
    await step("settings: put valid again", "owner", "PUT", SET, { body: { enabled: false, intervalHours: 1, retryMinutes: 1, initialDelayMinutes: 0, retentionDays: 3650 } });
    await step("settings: get", "owner", "GET", SET);
    await step("settings: reset", "owner", "DELETE", SET);
    await step("settings: get after reset", "owner", "GET", SET);
    await step("list: after reset", "owner", "GET", B);

    // r2
    const R = { enabled: false, mode: "s3", endpoint: "https://acct.r2.example", region: "auto", bucket: "bk", prefix: "stellarr" };
    await step("r2: put valid, off", "owner", "PUT", R2, { body: R });
    await step("r2: get", "owner", "GET", R2);
    await step("r2: put enabled s3 (keys are in the environment)", "owner", "PUT", R2, { body: { ...R, enabled: true } });
    await step("r2: put enabled worker (no token)", "owner", "PUT", R2, { body: { ...R, enabled: true, mode: "worker" } });
    await step("r2: put disabled worker", "owner", "PUT", R2, { body: { ...R, mode: "worker", bucket: "" } });
    await step("list: automation after r2", "owner", "GET", B);
    const pad = (n) => "x".repeat(n);
    for (const [label, change] of [
      ["mode 16", { mode: pad(16) }], ["mode unknown", { mode: "S3" }], ["mode padded", { mode: " s3" }], ["endpoint 2048", { endpoint: "https://a.example/" + pad(2048 - 19) }], ["endpoint 2049", { endpoint: "https://a.example/" + pad(2049 - 19) }],
      ["region 100", { region: pad(100) }], ["region 101", { region: pad(101) }], ["bucket 255", { bucket: pad(255) }], ["bucket 256", { bucket: pad(256) }], ["prefix 512", { prefix: pad(512) }], ["prefix 513", { prefix: pad(513) }],
      ["astral region 51", { region: "😀".repeat(51) }], ["astral region 50", { region: "😀".repeat(50) }], ["mode null", { mode: null }], ["endpoint null", { endpoint: null }], ["bucket number", { bucket: 5 }],
      ["trimmed", { endpoint: "  https://acct.r2.example  ", bucket: " bk ", prefix: " p ", region: " auto " }],
    ]) await step(`r2: ${label}`, "owner", "PUT", R2, { body: { ...R, ...change } });
    for (const [label, change] of [
      ["http", { endpoint: "http://acct.r2.example" }], ["userinfo", { endpoint: "https://u:p@acct.r2.example" }], ["query", { endpoint: "https://acct.r2.example/?a=1" }], ["fragment", { endpoint: "https://acct.r2.example/#x" }], ["relative", { endpoint: "acct" }], ["empty endpoint", { endpoint: "" }], ["path", { endpoint: "https://acct.r2.example/base" }],
      ["prefix ..", { prefix: "a/../b" }], ["prefix //", { prefix: "a//b" }], ["prefix /a", { prefix: "/a" }], ["prefix trailing /", { prefix: "a/" }], ["prefix blank", { prefix: " " }], ["prefix dot", { prefix: "./a" }], ["empty bucket", { bucket: "" }], ["empty region", { region: "" }], ["uppercase scheme", { endpoint: "HTTPS://acct.r2.example" }], ["space in endpoint", { endpoint: "https://a b.example" }],
    ]) await step(`r2 enabled: ${label}`, "owner", "PUT", R2, { body: { ...R, enabled: true, ...change } });
    await step("r2: members missing", "owner", "PUT", R2, { body: { enabled: false } });
    await step("r2: array", "owner", "PUT", R2, { raw: "[]", type: "application/json" });
    await step("r2: empty body", "owner", "PUT", R2, { raw: "", type: "application/json" });
    await step("r2: capital names", "owner", "PUT", R2, { raw: JSON.stringify({ Enabled: false, Mode: "s3", Endpoint: "", Region: "r", Bucket: "", Prefix: "p" }), type: "application/json" });
    await step("r2: get", "owner", "GET", R2);
    await step("r2: reset", "owner", "DELETE", R2);
    await step("r2: get after reset", "owner", "GET", R2);
    await step("route: trailing slash", "owner", "GET", `${B}/`);
    await step("route: POST settings", "owner", "POST", SET);
    await step("route: unknown verb on r2", "owner", "POST", R2);
  }

  if (args.phase === "stale") {
    const S = { enabled: true, intervalHours: 24, retryMinutes: 15, initialDelayMinutes: 5, retentionDays: 14 };
    const R = { enabled: false, mode: "s3", endpoint: "https://acct.r2.example", region: "auto", bucket: "bk", prefix: "stellarr" };
    for (const p of ["owner", "kim", "maya", "ada", "anon"]) {
      await step(`list [${p}]`, p, "GET", B);
      await step(`put settings [${p}]`, p, "PUT", SET, { body: S });
      await step(`put settings, malformed [${p}]`, p, "PUT", SET, { raw: "{", type: "application/json" });
      await step(`put settings, bad values [${p}]`, p, "PUT", SET, { body: { ...S, intervalHours: 0 } });
      await step(`put settings, empty body [${p}]`, p, "PUT", SET, { raw: "", type: "application/json" });
      await step(`put settings, text/plain [${p}]`, p, "PUT", SET, { raw: JSON.stringify(S), type: "text/plain" });
      await step(`put settings, no body [${p}]`, p, "PUT", SET);
      await step(`put settings, null [${p}]`, p, "PUT", SET, { raw: "null", type: "application/json" });
      await step(`put settings, array [${p}]`, p, "PUT", SET, { raw: "[]", type: "application/json" });
      await step(`put settings, wrong kind of member [${p}]`, p, "PUT", SET, { body: { ...S, intervalHours: "x" } });
      await step(`put settings, wrong kind of flag [${p}]`, p, "PUT", SET, { body: { ...S, enabled: 1 } });
      await step(`put r2, wrong kind of member [${p}]`, p, "PUT", R2, { body: { ...R, bucket: 5 } });
      await step(`put r2, null [${p}]`, p, "PUT", R2, { raw: "null", type: "application/json" });
      await step(`create, array [${p}]`, p, "POST", B, { raw: "[]", type: "application/json" });
      await step(`create, text/plain, empty [${p}]`, p, "POST", B, { raw: "", type: "text/plain" });
      await step(`create, wrong kind of flag [${p}]`, p, "POST", B, { body: { database: 1 } });
      await step(`delete settings [${p}]`, p, "DELETE", SET);
      await step(`put r2 [${p}]`, p, "PUT", R2, { body: R });
      await step(`put r2, malformed [${p}]`, p, "PUT", R2, { raw: "{", type: "application/json" });
      await step(`put r2, invalid fields [${p}]`, p, "PUT", R2, { body: { ...R, mode: "x" } });
      await step(`delete r2 [${p}]`, p, "DELETE", R2);
      await step(`download [${p}]`, p, "GET", `${B}/${GOOD}/download`);
      await step(`download, unknown [${p}]`, p, "GET", `${B}/${id("1")}.zip/download`);
      await step(`download, bad id [${p}]`, p, "GET", `${B}/nonsense/download`);
      await step(`create [${p}]`, p, "POST", B, { body: { database: true } });
      await step(`create, text/plain [${p}]`, p, "POST", B, { raw: '{"database":true}', type: "text/plain" });
      await step(`create, malformed [${p}]`, p, "POST", B, { raw: "{", type: "application/json" });
      await step(`verify [${p}]`, p, "POST", `${B}/${GOOD}/verify`);
      await step(`verify, unknown [${p}]`, p, "POST", `${B}/${id("1")}.zip/verify`);
    }
    await step("list afterwards", "owner", "GET", B);
  }

  if (args.phase === "fresh") {
    const S = { enabled: true, intervalHours: 6, retryMinutes: 5, initialDelayMinutes: 1, retentionDays: 30 };
    await step("put settings, fresh", "owner", "PUT", SET, { body: S });
    await step("put settings, fresh, bad body", "owner", "PUT", SET, { body: { ...S, intervalHours: 0 } });
    await step("delete settings, fresh", "owner", "DELETE", SET);
    await step("put r2, fresh", "owner", "PUT", R2, { body: { enabled: false, mode: "s3", endpoint: "", region: "auto", bucket: "", prefix: "stellarr" } });
    await step("delete r2, fresh", "owner", "DELETE", R2);
    await step("download, fresh", "owner", "GET", `${B}/${GOOD}/download`);
    await step("download, fresh, unknown", "owner", "GET", `${B}/${id("1")}.zip/download`);
  }

  if (args.phase === "cockpit-open" || args.phase === "cockpit-passed") {
    for (const p of ["anon", "maya", "ada", "owner"]) {
      await step(`list [${p}]`, p, "GET", B);
      await step(`put settings [${p}]`, p, "PUT", SET, { body: { enabled: true, intervalHours: 6, retryMinutes: 5, initialDelayMinutes: 1, retentionDays: 30 } });
      await step(`download [${p}]`, p, "GET", `${B}/${GOOD}/download`);
      await step(`create [${p}]`, p, "POST", B, { body: { database: true } });
    }
  }

  if (args.phase === "disabled") {
    for (const p of ["owner", "maya"]) {
      await step(`list [${p}]`, p, "GET", B);
      await step(`download [${p}]`, p, "GET", `${B}/${GOOD}/download`);
    }
  }
  // The destination with the credentials missing or partly there: which of the
  // three secrets the environment holds is the phase's own start (the script
  // starts both servers without the others), and the answers must agree.
  if (args.phase.startsWith("r2-")) {
    const R = { enabled: true, mode: "s3", endpoint: "https://acct.r2.example", region: "auto", bucket: "bk", prefix: "stellarr" };
    if (args.phase === "r2-all") {
      // The automatic backup tries the unreachable destination the settings
      // name, so that a failure is logged; the script greps the logs.
      // Wait for the thing itself, on this server, and only then ask: the
      // copy has failed when the list says so. A few minutes is the ceiling,
      // whatever else the machine is doing.
      const limit = Date.now() + 5 * 60 * 1000;
      for (;;) {
        const r = await who.owner.get(B);
        let failed = false;
        try { failed = r.status === 200 && JSON.parse(r.text).automation.lastError !== null; } catch {}
        if (failed) break;
        if (Date.now() > limit) { lines.push("## the failed copy never showed in the list"); break; }
        await new Promise((x) => setTimeout(x, 1000));
      }
      await step("list: automation after a failed copy", "owner", "GET", B);
    }
    await step("r2: before anything", "owner", "GET", R2);
    await step("r2: put enabled s3", "owner", "PUT", R2, { body: R });
    await step("r2: get after s3", "owner", "GET", R2);
    await step("r2: put enabled worker", "owner", "PUT", R2, { body: { ...R, mode: "worker", bucket: "" } });
    await step("r2: put disabled s3", "owner", "PUT", R2, { body: { ...R, enabled: false } });
    await step("r2: put enabled s3, bad endpoint", "owner", "PUT", R2, { body: { ...R, endpoint: "http://acct.r2.example" } });
    await step("r2: get", "owner", "GET", R2);
    await step("list: automation", "owner", "GET", B);
    await step("r2: reset", "owner", "DELETE", R2);
  }
  if (seen.length) lines.push("## SECRETS SEEN", ...seen);
  writeFileSync(args.out, lines.join("\n") + "\n");
}

if (mode === "seed") await seed();
if (mode === "login") await login();
if (mode === "ask") await ask();
