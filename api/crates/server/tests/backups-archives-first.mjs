// Asks the first server (the owner's session) for a full backup archive and
// saves it: node backups-archives-first.mjs --base URL --out FILE
import { writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({ options: { base: { type: "string" }, out: { type: "string" }, user: { type: "string" } } });
const headers = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
const cookies = new Map();
async function send(method, path, body) {
  const h = { ...headers };
  if (cookies.size) h.Cookie = [...cookies].map(([k, v]) => `${k}=${v}`).join("; ");
  if (body !== undefined) h["Content-Type"] = "application/json";
  const response = await fetch(args.base + path, { method, headers: h, body: body === undefined ? undefined : JSON.stringify(body) });
  for (const line of response.headers.getSetCookie()) {
    const [pair] = line.split(";"); const at = pair.indexOf("=");
    cookies.set(pair.slice(0, at), pair.slice(at + 1));
  }
  return response;
}
const login = await send("POST", "/api/auth/login", { username: args.user ?? "owner", password: "Corona!2026-long" });
if (login.status !== 200) throw new Error(`login ${login.status}`);
const started = await send("POST", "/api/cockpit/backups", { database: true, objects: true, keys: true, instance: true });
if (started.status !== 202) throw new Error(`start ${started.status} ${await started.text()}`);
const { id } = await started.json();
for (let i = 0; i < 600; i++) {
  const list = await (await send("GET", "/api/cockpit/backups")).json();
  if (list.lastFailure) throw new Error("failed: " + JSON.stringify(list.lastFailure));
  if (!list.job && list.backups.some((b) => b.id === id)) break;
  await new Promise((r) => setTimeout(r, 200));
}
const download = await send("GET", `/api/cockpit/backups/${id}/download`);
if (download.status !== 200) throw new Error(`download ${download.status}`);
writeFileSync(args.out, Buffer.from(await download.arrayBuffer()));
console.log("first server archive", id);
