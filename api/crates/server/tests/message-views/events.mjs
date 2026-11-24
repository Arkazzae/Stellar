// What the people on a hub hear when reactions and pins change, from one server.
//   node events.mjs --base http://127.0.0.1:5215 --state state.json --messages ids.json --out transcript.txt
import { writeFileSync, readFileSync } from "node:fs";
import { parseArgs } from "node:util";
const { values: args } = parseArgs({ options: { base: { type: "string" }, state: { type: "string" }, messages: { type: "string" }, out: { type: "string" } } });
const base = args.base;
const state = JSON.parse(readFileSync(args.state, "utf8"));
const ids = JSON.parse(readFileSync(args.messages, "utf8"));
const headers = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json" };
const RS = "\u001e";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const names = new Map();
async function login(username) {
  const r = await fetch(`${base}/api/auth/login`, { method: "POST", headers, body: JSON.stringify({ username, password: "Corona!2026-long" }) });
  const body = await r.json();
  names.set(body.id, username);
  return { username, id: body.id, cookie: r.headers.getSetCookie().map((l) => l.split(";")[0]).join("; ") };
}
const scrub = (v) => {
  let t = JSON.stringify(v);
  for (const [id, n] of names) t = t.replaceAll(id, `<${n}>`);
  for (const [n, id] of Object.entries(ids)) t = t.replaceAll(id, `<${n}>`);
  for (const [n, id] of Object.entries(state.channels)) t = t.replaceAll(id, `<#${n}>`);
  return t.replace(/\d{4}-\d\d-\d\dT[\d:.]+Z/g, "<time>");
};
async function listen(person) {
  const socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app`, { headers: { Cookie: person.cookie } });
  await new Promise((res, rej) => { socket.onopen = res; socket.onerror = () => rej(new Error("upgrade " + person.username)); });
  const heard = [];
  let buffer = "";
  socket.onmessage = (e) => {
    buffer += String(e.data);
    let end;
    while ((end = buffer.indexOf(RS)) >= 0) {
      const rec = JSON.parse(buffer.slice(0, end)); buffer = buffer.slice(end + 1);
      if (rec.type === 1) heard.push(`${rec.target} ${scrub(rec.arguments)}`);
    }
  };
  socket.send(`{"protocol":"json","version":1}${RS}`);
  await sleep(300);
  return { person, heard, socket, mark: 0 };
}
const people = {};
for (const n of ["boss", "anna", "bartek", "celina", "dawid"]) people[n] = await login(n);
const hubs = {};
for (const n of Object.keys(people)) hubs[n] = await listen(people[n]);
const lines = [];
async function act(label, who, method, path) {
  const r = await fetch(base + path, { method, headers: { ...headers, Cookie: people[who].cookie } });
  await r.text();
  await sleep(250);
  lines.push(`## ${label} -> ${r.status}`);
  for (const n of Object.keys(hubs)) {
    const fresh = hubs[n].heard.slice(hubs[n].mark).filter((l) => !l.startsWith("UserPresence") && !l.startsWith("Presence"));
    hubs[n].mark = hubs[n].heard.length;
    for (const l of fresh) lines.push(`  ${n}: ${l}`);
  }
}
const e = encodeURIComponent;
await act("boss reacts in general", "boss", "PUT", `/api/messages/${ids.g0}/reactions/${e("👍")}`);
await act("boss reacts again", "boss", "PUT", `/api/messages/${ids.g0}/reactions/${e("👍")}`);
await act("anna reacts", "anna", "PUT", `/api/messages/${ids.g0}/reactions/${e("👍")}`);
await act("boss takes back", "boss", "DELETE", `/api/messages/${ids.g0}/reactions/${e("👍")}`);
await act("boss takes back again", "boss", "DELETE", `/api/messages/${ids.g0}/reactions/${e("👍")}`);
await act("bartek reacts in secret", "bartek", "PUT", `/api/messages/${ids.s0}/reactions/${e("🎉")}`);
await act("dawid cannot", "dawid", "PUT", `/api/messages/${ids.s0}/reactions/${e("🎉")}`);
await act("anna denied in noreact", "anna", "PUT", `/api/messages/${ids.n0}/reactions/${e("🎉")}`);
await act("bad emoji", "boss", "PUT", `/api/messages/${ids.g0}/reactions/${e("a b")}`);
await act("boss pins in secret", "boss", "PUT", `/api/messages/${ids.s0}/pin`);
await act("boss pins again", "boss", "PUT", `/api/messages/${ids.s0}/pin`);
await act("anna cannot pin", "anna", "PUT", `/api/messages/${ids.g1}/pin`);
await act("boss pins in general", "boss", "PUT", `/api/messages/${ids.g1}/pin`);
await act("boss unpins", "boss", "DELETE", `/api/messages/${ids.g1}/pin`);
await act("boss unpins again", "boss", "DELETE", `/api/messages/${ids.g1}/pin`);
await act("lounge over the limit", "boss", "PUT", `/api/messages/${ids.l0}/pin`);
for (const h of Object.values(hubs)) h.socket.close();
writeFileSync(args.out, lines.join("\n") + "\n");
