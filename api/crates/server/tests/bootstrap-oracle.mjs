// What a client loading its bootstrap observes, as a transcript, from one
// server. Run it against this server and against the first one on copies of
// one database (see bootstrap-shared-database.sh) and compare the transcripts.
//
//   node bootstrap-oracle.mjs --base http://127.0.0.1:5000 --home ID --beta ID --secret S --out transcript.txt
//
// Needs Node 26. The accounts and the servers are made by the shell script.
// Ids are replaced by names and by the order they were first seen in; the
// relay's credentials, which hold the moment they end, are checked against the
// secret instead of compared.

import { createHmac } from "node:crypto";
import { writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({
  options: { base: { type: "string" }, out: { type: "string" }, home: { type: "string" }, beta: { type: "string" }, secret: { type: "string" }, password: { type: "string", default: "Corona!2026-long" } },
});
const base = args.base;
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const names = new Map();
const seen = new Map();
const lines = [];
const out = (text) => lines.push(text);

function scrub(value) {
  let text = typeof value === "string" ? value : JSON.stringify(value);
  for (const [id, name] of names) text = text.replaceAll(id, `<${name}>`).replaceAll(id.replaceAll("-", ""), `<${name}>`);
  text = text.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, (id) => {
    const key = id.toLowerCase();
    if (!seen.has(key)) seen.set(key, `id${seen.size + 1}`);
    return `<${seen.get(key)}>`;
  });
  text = text.replace(/\d{4}-\d\d-\d\dT[\d:.]+(Z?)/g, "<time>$1");
  return text.replace(/00-[0-9a-f]{32}-[0-9a-f]{16}-00/g, "<trace>");
}

async function http(who, method, path, headers = {}) {
  const response = await fetch(base + path, { method, headers: { ...clientHeaders, ...(who?.cookie ? { Cookie: who.cookie } : {}), ...headers } });
  const text = await response.text();
  const keep = ["content-type", "cache-control", "pragma", "expires", "vary", "etag", "x-content-type-options", "allow"];
  const shown = keep.filter((h) => response.headers.has(h)).map((h) => `${h}: ${response.headers.get(h)}`).join("; ");
  return { status: response.status, text, shown };
}

async function signIn(username) {
  const response = await fetch(base + "/api/auth/login", { method: "POST", headers: { ...clientHeaders, "Content-Type": "application/json" }, body: JSON.stringify({ username, password: args.password }) });
  if (response.status !== 200) return null;
  const body = await response.json();
  const id = body.id ?? body.user?.id;
  names.set(id, username);
  return { username, id, cookie: response.headers.getSetCookie().map((line) => line.split(";")[0]).join("; ") };
}

class Hub {
  constructor(person) { this.person = person; this.waiting = new Map(); this.next = 0; this.buffer = ""; }
  async open() {
    this.socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app`, { headers: { Cookie: this.person.cookie } });
    await new Promise((resolve, reject) => { this.socket.onopen = resolve; this.socket.onerror = () => reject(new Error("upgrade")); });
    this.socket.onmessage = (event) => this.onText(String(event.data));
    this.socket.send(`{"protocol":"json","version":1}${RS}`);
    await sleep(500);
    return this;
  }
  onText(text) {
    this.buffer += text;
    let end;
    while ((end = this.buffer.indexOf(RS)) >= 0) {
      const record = JSON.parse(this.buffer.slice(0, end));
      this.buffer = this.buffer.slice(end + 1);
      if (record.type === 3 && this.waiting.has(record.invocationId)) this.waiting.get(record.invocationId)(record);
    }
  }
  async invoke(target, ...argumentsList) {
    const id = String(++this.next);
    const done = new Promise((resolve) => this.waiting.set(id, resolve));
    this.socket.send(JSON.stringify({ type: 1, invocationId: id, target, arguments: argumentsList }) + RS);
    return Promise.race([done, sleep(3000)]);
  }
}

const P = {};
for (const name of ["ola", "mia", "noor", "omar", "pat", "uma", "tess", "sam", "quinn"]) P[name] = await signIn(name);
names.set(args.home, "home");
names.set(args.beta, "beta");

// The check of the relay's credentials against the secret.
function checkCredentials(body) {
  try {
    const answer = JSON.parse(body);
    const turn = answer.rtc.iceServers.filter((s) => s.username);
    return turn.map((s) => {
      const want = createHmac("sha1", args.secret).update(s.username).digest("base64");
      const [ends, user] = s.username.split(":");
      const left = Number(ends) - Date.now() / 1000;
      return `credential ${want === s.credential ? "matches" : "DIFFERS"}; lasts ${left > 3500 && left <= 3600 ? "an hour" : `${left} seconds`}; for ${names.get(user.replace(/(.{8})(.{4})(.{4})(.{4})(.{12})/, "$1-$2-$3-$4-$5")) ?? user}`;
    }).join(" | ");
  } catch {
    return "";
  }
}
function withoutCredentials(body) {
  try {
    const answer = JSON.parse(body);
    for (const s of answer.rtc.iceServers) if (s.username) { s.username = "<username>"; s.credential = "<credential>"; }
    return JSON.stringify(answer);
  } catch {
    return body;
  }
}

async function ask(title, who, path = "/api/bootstrap", headers = {}, method = "GET") {
  const r = await http(who, method, path, headers);
  out(`## ${title}`);
  out(`   -> ${r.status} [${r.shown}]`);
  const check = path.startsWith("/api/bootstrap") && r.status === 200 ? checkCredentials(r.text) : "";
  const text = r.status === 200 && path.startsWith("/api/bootstrap") ? withoutCredentials(r.text) : r.text;
  // The answer, one part to a line, so that a difference shows where it is.
  try {
    const answer = JSON.parse(text);
    if (path.startsWith("/api/bootstrap") && r.status === 200) {
      for (const key of Object.keys(answer)) {
        const value = answer[key];
        if (Array.isArray(value) && value.length > 12) {
          out(`   ${key}: ${value.length} items`);
          for (const item of value) out(`      ${scrub(item)}`);
        } else out(`   ${key}: ${scrub(value)}`);
      }
      out(`   size ~ ${Math.round(r.text.length / 1000)} kB`);
    } else out(`   ${scrub(text)}`);
  } catch {
    out(`   ${scrub(text)}`);
  }
  if (check) out(`   ${check}`);
}

// Who is connected, and with what status, for as long as the questions go on.
const hubs = {};
for (const name of ["ola", "mia", "noor", "omar", "tess", "sam"]) if (P[name]) hubs[name] = await new Hub(P[name]).open();
await hubs.noor.invoke("SetPresence", "idle");
await hubs.omar.invoke("SetPresence", "dnd");
await hubs.tess.invoke("SetPresence", "offline");
await sleep(500);

for (const name of ["ola", "mia", "noor", "omar", "uma", "tess", "sam"]) await ask(`${name} loads the bootstrap`, P[name]);
await ask("uma loads it again, and finds her markers there", P.uma);
await ask("pat, who belongs to no server", P.pat);
await ask("pat asks for the home server", P.pat, "/api/bootstrap", { "X-Stellarr-Server": args.home });
await ask("anybody signed in with a disabled account", P.quinn);

// A member of two servers asks for each, in every form.
await ask("omar asks for the home server by header", P.omar, "/api/bootstrap", { "X-Stellarr-Server": args.home });
await ask("omar asks for the beta server by header", P.omar, "/api/bootstrap", { "X-Stellarr-Server": args.beta });
await ask("omar asks for the beta server by query", P.omar, `/api/bootstrap?serverId=${args.beta}`);
await ask("omar asks for the home server by query in capitals", P.omar, `/api/bootstrap?SERVERID=${args.home.toUpperCase()}`);
await ask("omar names both, the header counts", P.omar, `/api/bootstrap?serverId=${args.beta}`, { "X-Stellarr-Server": args.home });
for (const [label, value] of [
  ["empty", ""], ["a space", " "], ["rubbish", "garbage"], ["a server he is not in", "11111111-1111-1111-1111-111111111111"],
  ["the id without hyphens", args.home.replaceAll("-", "")], ["in braces", `{${args.home}}`], ["in parentheses", `(${args.home})`],
  ["with spaces around", `  ${args.home}  `], ["with a trailing nul", `${args.home}\u0000`], ["two ids", `${args.home},${args.beta}`],
]) {
  await ask(`omar asks by header for: ${label}`, P.omar, "/api/bootstrap", { "X-Stellarr-Server": value }).catch((e) => out(`   error: ${e.message}`));
}
for (const [label, query] of [["empty", "?serverId="], ["rubbish", "?serverId=zzz"], ["twice", `?serverId=${args.beta}&serverId=${args.home}`], ["plus for a space", `?serverId=+${args.home}`], ["other name", `?server=${args.beta}`]]) {
  await ask(`omar asks by query for: ${label}`, P.omar, `/api/bootstrap${query}`);
}
await ask("mia asks for the beta server she is not in", P.mia, "/api/bootstrap", { "X-Stellarr-Server": args.beta });

// Nobody signed in, and what else may be asked.
await ask("nobody signed in", null);
await ask("HEAD", P.mia, "/api/bootstrap", {}, "HEAD");
await ask("POST", P.mia, "/api/bootstrap", {}, "POST");
await ask("a path with a slash after it", P.mia, "/api/bootstrap/");
await ask("giphy, signed in", P.mia, "/api/config/giphy");
await ask("giphy, nobody signed in", null, "/api/config/giphy");
await ask("giphy, POST", P.mia, "/api/config/giphy", {}, "POST");

// How long the answer takes for the biggest page, said apart from the transcript.
const took = [];
let size = 0;
for (let i = 0; i < 9; i++) {
  const started = performance.now();
  const r = await fetch(base + "/api/bootstrap", { headers: { ...clientHeaders, Cookie: P.mia.cookie } });
  size = (await r.text()).length;
  took.push(performance.now() - started);
}
took.sort((a, b) => a - b);
console.error(`${base}: median ${took[4].toFixed(1)} ms, fastest ${took[0].toFixed(1)} ms, ${size} bytes`);

writeFileSync(args.out, lines.join("\n") + "\n");
for (const hub of Object.values(hubs)) hub.socket.close(1000);
process.exit(0);
