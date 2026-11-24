// Asks a server that started on a restored data folder what a person would:
// sign in, read the history, download what was uploaded, open the hub, and
// what happens to an account whose authenticator app the first server sealed.
//
//   node backups-archives-restored.mjs --base URL --facts facts.json --first-server-sealed 1|0
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({ options: { base: { type: "string" }, facts: { type: "string" }, sealed: { type: "string" } } });
const facts = JSON.parse(readFileSync(args.facts, "utf8"));
const PASSWORD = "Corona!2026-long";
const headers = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? "ok " : "NO "} ${what}`); if (!ok) failed++; };

function session() {
  const cookies = new Map();
  return {
    async send(method, path, body, extra = {}) {
      const h = { ...headers, ...extra };
      if (cookies.size) h.Cookie = [...cookies].map(([k, v]) => `${k}=${v}`).join("; ");
      if (body !== undefined) h["Content-Type"] = "application/json";
      const r = await fetch(args.base + path, { method, headers: h, body: body === undefined ? undefined : JSON.stringify(body) });
      for (const line of r.headers.getSetCookie()) {
        const [pair] = line.split(";"); const at = pair.indexOf("=");
        const v = pair.slice(at + 1); if (v === "") cookies.delete(pair.slice(0, at)); else cookies.set(pair.slice(0, at), v);
      }
      return r;
    },
  };
}

check((await fetch(args.base + "/health/ready")).status === 200, "the server is ready on the restored folder");
const maya = session();
const login = await maya.send("POST", "/api/auth/login", { username: "maya", password: PASSWORD });
check(login.status === 200, `maya signs in with her password (${login.status})`);
const me = await maya.send("GET", "/api/me");
check(me.status === 200, `a signed-in session is known (${me.status})`);

if (facts.message) {
  const history = await maya.send("GET", `/api/channels/${facts.message.channel}/messages`, undefined, { "X-Stellarr-Server": facts.message.server });
  const text = await history.text();
  console.log(`   history ${history.status}`); check(history.status === 200 && text.includes(facts.message.id), "the history holds the message that was written");
  const asset = await maya.send("GET", `/assets/${facts.message.attachment}`);
  const got = Buffer.from(await asset.arrayBuffer());
  check(asset.status === 200 && got.toString("hex") === facts.message.bytesHex, "the attachment downloads with the bytes that were uploaded");
}
if (facts.encrypted) {
  const asset = await maya.send("GET", `/assets/${facts.encrypted.id}`);
  const got = Buffer.from(await asset.arrayBuffer());
  check(asset.status === 200 && createHash("sha256").update(got).digest("hex") === facts.encrypted.sha, "the encrypted upload downloads whole");
}
const negotiate = await maya.send("POST", "/hubs/app/negotiate?negotiateVersion=1");
const negotiated = await negotiate.text();
check(negotiate.status === 200 && negotiated.includes("WebSockets"), `the hub negotiates (${negotiate.status})`);

// The authenticator app was sealed by the first server's keys; this server
// cannot read it, so the account is asked for a second step it cannot check.
const zed = session();
const z = await zed.send("POST", "/api/auth/login", { username: "zed", password: PASSWORD });
const zt = await z.text();
if (args.sealed === "1" && facts.steps["second step: confirm"] === "done") {
  check(z.status === 200 && /challenge/i.test(zt), `zed is asked for a second step, not let in (${z.status})`);
} else {
  console.log(`-- zed: ${z.status} (no readable app expected: ${args.sealed})`);
}
process.exit(failed ? 1 : 0);
