// Fills the first server through its own routes with what the groups merged
// since the seed add: profile pictures, a server icon, a sound, an encrypted
// upload and an authenticator app. Tiny data. Prints one line per step and
// writes what a restored instance is asked afterwards to --out (JSON).
//
//   node backups-archives-fill.mjs --base URL --state state.json --out facts.json
import { createHash, createHmac, randomBytes } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({ options: { base: { type: "string" }, state: { type: "string" }, out: { type: "string" } } });
const state = JSON.parse(readFileSync(args.state, "utf8"));
const PASSWORD = "Corona!2026-long";
const headers = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");
const ogg = Buffer.concat([Buffer.from("OggS"), randomBytes(60)]);
const facts = { steps: {} };

class Person {
  constructor(name) { this.name = name; this.cookies = new Map(); }
  async send(method, path, { body, form, server } = {}) {
    const h = { ...headers };
    if (this.cookies.size) h.Cookie = [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
    if (server) h["X-Stellarr-Server"] = server;
    let payload;
    if (form) payload = form; else if (body !== undefined) { h["Content-Type"] = "application/json"; payload = JSON.stringify(body); }
    const response = await fetch(args.base + path, { method, headers: h, body: payload });
    for (const line of response.headers.getSetCookie()) {
      const [pair] = line.split(";"); const at = pair.indexOf("=");
      const value = pair.slice(at + 1);
      if (value === "") this.cookies.delete(pair.slice(0, at)); else this.cookies.set(pair.slice(0, at), value);
    }
    const text = await response.text();
    let json = null; try { json = JSON.parse(text); } catch {}
    return { status: response.status, json, text };
  }
  async login() {
    const r = await this.send("POST", "/api/auth/login", { body: { username: this.name, password: PASSWORD } });
    if (r.status !== 200) throw new Error(`login ${this.name}: ${r.status} ${r.text}`);
    this.id = r.json.id ?? r.json.user?.id;
    return this;
  }
}
const file = (name, type, bytes, extra = {}) => {
  const f = new FormData();
  f.append("file", new Blob([bytes], { type }), name);
  for (const [k, v] of Object.entries(extra)) f.append(k, String(v));
  return f;
};
async function step(name, run, wanted = [200, 201, 204]) {
  try {
    const r = await run();
    const ok = wanted.includes(r.status);
    facts.steps[name] = ok ? "done" : `refused ${r.status} ${r.text.slice(0, 120)}`;
    console.log(`${ok ? "ok " : "NO "} ${name}: ${r.status}`);
    return ok ? r : null;
  } catch (e) { facts.steps[name] = `error ${e.message}`; console.log(`NO  ${name}: ${e.message}`); return null; }
}

const owner = await new Person("owner").login();
const maya = await new Person("maya").login();
const orbit = state.servers.orbit;
facts.owner = owner.id; facts.maya = maya.id;

for (const kind of ["avatar", "banner", "wallpaper"]) {
  await step(`profile ${kind}`, () => owner.send("POST", `/api/me/images/${kind}`, { form: file(`${kind}.png`, "image/png", png) }));
}
await step("server icon", () => maya.send("POST", "/api/admin/server/icon", { form: file("icon.png", "image/png", png), server: orbit }));

const sound = await step("sound upload", () => maya.send("POST", "/api/uploads", { form: file("beep.ogg", "audio/ogg", ogg, { durationSeconds: 1.5 }), server: orbit }));
if (sound) await step("soundboard sound", () => maya.send("POST", "/api/admin/soundboard/sounds", { body: { attachmentId: sound.json.id, name: "beep", glyph: "B", trimStartSeconds: 0, trimEndSeconds: 1 }, server: orbit }));

const sealed = randomBytes(300);
const encrypted = await step("encrypted upload", () => maya.send("POST", "/api/uploads/encrypted", { form: file("x.bin", "application/octet-stream", sealed), server: orbit }));
if (encrypted) facts.encrypted = { id: encrypted.json.id, sha: createHash("sha256").update(sealed).digest("hex") };

// An attachment on a message, to download again after the restore.
const channel = state.channels["orbit-general"];
const up = await step("attachment upload", () => maya.send("POST", "/api/uploads", { form: file("keep.png", "image/png", png), server: orbit }));
if (up) {
  const posted = await step("message with attachment and a link", () => maya.send("POST", `/api/channels/${channel}/messages`, { body: { content: "kept https://example.invalid/page", attachmentIds: [up.json.id] }, server: orbit }));
  if (posted) facts.message = { id: posted.json.id, channel, server: orbit, content: "kept https://example.invalid/page", attachment: up.json.id, bytesHex: png.toString("hex") };
}

// An authenticator app for zed: start, read the secret, confirm with its code.
function base32(s) { const a = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"; let bits = 0, val = 0; const out = []; for (const ch of s.replace(/=+$/, "").toUpperCase()) { val = (val << 5) | a.indexOf(ch); bits += 5; if (bits >= 8) { out.push((val >>> (bits - 8)) & 255); bits -= 8; } } return Buffer.from(out); }
function totp(secret) { const c = Buffer.alloc(8); c.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30000))); const h = createHmac("sha1", base32(secret)).update(c).digest(); const o = h[19] & 15; return String(((h.readUInt32BE(o) & 0x7fffffff) % 1_000_000)).padStart(6, "0"); }
const zed = await new Person("zed").login();
facts.zed = zed.id;
const begin = await step("second step: start enrollment", () => zed.send("POST", "/api/me/security/mfa/enroll/start", { body: { password: PASSWORD } }));
if (begin) {
  const challengeId = begin.json.challengeId ?? begin.json.id;
  const started = await step("second step: authenticator", () => zed.send("POST", "/api/auth/mfa/enroll/totp/start", { body: { challengeId, name: "phone" } }));
  if (started) await step("second step: confirm", () => zed.send("POST", "/api/auth/mfa/enroll/totp/confirm", { body: { challengeId, methodId: started.json.methodId, code: totp(started.json.secret), name: "phone" } }));
}
writeFileSync(args.out, JSON.stringify(facts, null, 1));
