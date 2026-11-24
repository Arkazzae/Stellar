// Many idle hub connections, then one event to all of them: what the server
// holds for each, and how long the event takes to arrive.
//
//   ulimit -n 20000
//   hub_load 5320 /tmp/some-empty-folder &        # built with the examples
//   node tests/realtime-load.mjs --port 5320 --pid <the server's pid> --connections 3000
//
// Needs Node 26. The first account is founded with the setup code the example
// serves at /load/setup-code (or --setup-code); every connection is a tab of that one account,
// as many tabs are, which is the case where presence is told once.

import { parseArgs } from "node:util";
import { execFileSync } from "node:child_process";

const { values: args } = parseArgs({
  options: {
    port: { type: "string" },
    pid: { type: "string" },
    connections: { type: "string", default: "3000" },
    "setup-code": { type: "string" },
    "ramp-per-second": { type: "string", default: "1500" },
  },
});
const base = `http://127.0.0.1:${args.port}`;
const total = Number(args.connections);
const headers = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json" };
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const rss = () => Number(execFileSync("ps", ["-o", "rss=", "-p", args.pid]).toString().trim()) * 1024;
const mib = (bytes) => (bytes / 1048576).toFixed(1);

const register = await fetch(`${base}/api/auth/registration`).then((r) => r.json()).catch(() => ({}));
if (register.setupRequired !== false) {
  const made = await fetch(`${base}/api/auth/register`, {
    method: "POST",
    headers,
    body: JSON.stringify({
      username: "loader",
      displayName: "Loader",
      email: "loader@example.test",
      password: "Passw0rd!2026",
      setupCode: args["setup-code"] ?? (await fetch(`${base}/load/setup-code`).then((r) => r.text())),
    }),
  });
  if (made.status !== 201) throw new Error(`founding failed: ${made.status}`);
}
const login = await fetch(`${base}/api/auth/login`, { method: "POST", headers, body: JSON.stringify({ username: "loader", password: "Passw0rd!2026" }) });
const cookie = login.headers.getSetCookie().map((line) => line.split(";")[0]).join("; ");
if (!login.ok || !cookie) throw new Error(`sign-in failed: ${login.status}`);

await sleep(1000);
const before = rss();
console.log(`server memory before: ${mib(before)} MiB`);

const arrivals = new Array(total).fill(null);
let open = 0;
const sockets = [];
for (let index = 0; index < total; index += 1) {
  const socket = new WebSocket(`${base.replace(/^http/, "ws")}/hubs/app`, {
    protocols: ["stellarr.v1"],
    headers: { Cookie: cookie, Origin: base },
  });
  socket.onmessage = (event) => {
    const record = JSON.parse(event.data);
    if (record.type === "ready") open += 1;
    if (record.type === "event" && record.name === "Probe") arrivals[index] = performance.now();
  };
  sockets.push(socket);
  if (index % Number(args["ramp-per-second"]) === Number(args["ramp-per-second"]) - 1) await sleep(1000);
}
const started = Date.now();
while (open < total && Date.now() - started < 60000) await sleep(100);
console.log(`${open} of ${total} connections are open`);
await sleep(5000);
const idle = rss();
console.log(`server memory with ${open} idle connections: ${mib(idle)} MiB, ${((idle - before) / open / 1024).toFixed(1)} KiB for each`);

const sent = performance.now();
const count = await fetch(`${base}/load/publish`).then((r) => r.text());
const waited = Date.now();
while (arrivals.some((at) => at === null) && Date.now() - waited < 30000) await sleep(5);
const got = arrivals.filter((at) => at !== null).sort((a, b) => a - b);
console.log(`the server counted ${count} connections; ${got.length} of ${total} received the event`);
if (got.length > 0) {
  const pick = (share) => (got[Math.min(got.length - 1, Math.floor(got.length * share))] - sent).toFixed(1);
  console.log(`time from the request to arrival: first ${pick(0)} ms, median ${pick(0.5)} ms, 99th ${pick(0.99)} ms, last ${(got.at(-1) - sent).toFixed(1)} ms`);
}
console.log(`server memory after the event: ${mib(rss())} MiB`);
for (const socket of sockets) socket.close();
await sleep(1500);
console.log(`server memory after everyone left: ${mib(rss())} MiB`);
process.exit(open === total && got.length === total ? 0 : 1);
