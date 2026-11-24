// What the people of a server hear on the hub while a plan of requests is sent
// to it, as a transcript, from one server. Run it against this server and the
// first one, each on a copy of the same database, and compare the transcripts
// (see roles-shared-database.sh).
//
//   node roles-events.mjs --base http://127.0.0.1:5000 --plan plan.jsonl --users users.txt --worlds 30 --out transcript.txt
//
// Needs Node 26 (the global WebSocket takes request headers). Ids are replaced
// by the names the plan gives them.

import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({
  options: {
    base: { type: "string" },
    plan: { type: "string" },
    users: { type: "string" },
    worlds: { type: "string", default: "30" },
    out: { type: "string" },
  },
});
const PASSWORD = "Corona!2026-long";
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const lines = [];

async function signIn(username) {
  const response = await fetch(`${args.base}/api/auth/login`, {
    method: "POST",
    headers: clientHeaders,
    body: JSON.stringify({ username, password: PASSWORD }),
  });
  if (response.status !== 200) throw new Error(`signing in ${username}: ${response.status}`);
  return response.headers.getSetCookie().map((line) => line.split(";")[0]).join("; ");
}

class Client {
  constructor(label, cookie, server) {
    this.label = label;
    this.cookie = cookie;
    this.server = server;
    this.records = [];
    this.buffer = "";
  }

  async open() {
    const url = `${args.base.replace("http", "ws")}/hubs/app?serverId=${this.server}`;
    this.socket = new WebSocket(url, { headers: { Cookie: this.cookie } });
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
        if (record.type === 1) this.records.push(record);
      }
    };
    this.socket.onclose = () => {
      this.records.push({ closed: true });
    };
    this.socket.send(`{"protocol":"json","version":1}${RS}`);
    return this;
  }

  take() {
    const taken = this.records;
    this.records = [];
    return taken;
  }

  close() {
    try {
      this.socket.close(1000);
    } catch {}
  }
}

const GUID = /[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/g;

const users = readFileSync(args.users, "utf8").split("\n").filter(Boolean).map((line) => line.split(" ")[0]);
const cookies = {};
for (const name of users) cookies[name] = await signIn(name);

const worlds = readFileSync(args.plan, "utf8").split("\n").filter(Boolean).slice(0, Number(args.worlds));
for (const [number, text] of worlds.entries()) {
  const world = JSON.parse(text);
  const labels = new Map(Object.entries(world.labels).map(([id, label]) => [id.toLowerCase(), label]));
  const made = new Map();
  const scrub = (value) =>
    JSON.stringify(value).replace(GUID, (found) => {
      const key = found.toLowerCase();
      if (labels.has(key)) return `<${labels.get(key)}>`;
      if (key === world.server) return "<server>";
      if (!made.has(key)) made.set(key, made.size + 1);
      return `<made ${made.get(key)}>`;
    });
  lines.push(`# world ${number}`);
  const clients = [];
  for (const name of [...world.members].sort()) {
    try {
      clients.push(await new Client(name, cookies[name], world.server).open());
    } catch (error) {
      lines.push(`   ${name}: ${error.message}`);
    }
  }
  await sleep(300);
  for (const client of clients) client.take();
  for (const [index, step] of world.steps.entries()) {
    if (step.method === "GET") continue;
    const response = await fetch(`${args.base}${step.path}`, {
      method: step.method,
      headers: { ...clientHeaders, Cookie: cookies[step.actor], "X-Stellarr-Server": world.server },
      body: step.body === null ? undefined : JSON.stringify(step.body),
    });
    await response.arrayBuffer();
    lines.push(`## ${index} ${step.actor} ${step.method} ${scrub(step.path)} -> ${response.status}`);
    await sleep(250);
    for (const client of clients) {
      for (const record of client.take()) {
        lines.push(`   ${client.label}: ${record.closed ? "closed" : `${record.target} ${scrub(record.arguments)}`}`);
      }
    }
  }
  for (const client of clients) client.close();
  await sleep(100);
}
writeFileSync(args.out, lines.join("\n") + "\n");
