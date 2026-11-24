// The desktop browser sign-in of this server and of the first one, asked the
// same questions on copies of one database file, as transcripts to compare.
//
//   node desktop-login-oracle.mjs seed   --base URL --setup CODE --state state.json
//   node desktop-login-oracle.mjs ask    --base URL --state state.json --out transcript.txt --ids ids.json
//   node desktop-login-oracle.mjs after  --base URL --state state.json --ids ids.json --out transcript.txt
//
// `seed` founds the first server and makes the accounts the questions need.
// `ask` signs people in, starts requests and walks every state a request, a
// page and an approval can be in; what it needs changed in the database (a
// request that ran out, a session that was ended) it leaves in `ids.json`,
// where the script that runs both servers reads it. `after` asks again once
// those changes are made. Ids, moments and secrets that differ by nature are
// replaced in the transcript. Needs Node 26.
//
// The exchange is not asked: this server does not serve it yet. `--exchange`
// asks it, for the first server alone.
//
// See desktop-login-shared-database.sh, which runs both.

import { readFileSync, writeFileSync, appendFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    base: { type: "string" },
    setup: { type: "string" },
    state: { type: "string" },
    ids: { type: "string" },
    out: { type: "string" },
    exchange: { type: "boolean" },
  },
});
const mode = positionals[0];
const base = args.base;
const PASSWORD = "Corona!2026-long";
const web = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
const desktopClient = { "X-Stellarr-Client": "desktop-linux", "X-Stellarr-Client-Version": "0.1.3" };

/** One browser or one application: a cookie jar and the headers of a client. */
class Agent {
  constructor(headers = web) {
    this.cookies = new Map();
    this.headers = headers;
  }

  async send(method, path, { body, form, type, headers = {}, raw } = {}) {
    const request = { method, headers: { ...this.headers, ...headers }, redirect: "manual" };
    if (this.cookies.size > 0) request.headers.Cookie = [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
    if (form !== undefined) {
      request.body = form;
      if (type !== null) request.headers["Content-Type"] = type ?? "application/x-www-form-urlencoded";
    } else if (raw !== undefined) {
      request.body = raw;
      if (type) request.headers["Content-Type"] = type;
    } else if (body !== undefined) {
      request.body = JSON.stringify(body);
      request.headers["Content-Type"] = "application/json";
    }
    const response = await fetch(`${base}${path}`, request);
    const setCookie = response.headers.getSetCookie();
    for (const line of setCookie) {
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
    return { status: response.status, headers: response.headers, text, json, setCookie };
  }

  get(path, options) { return this.send("GET", path, options); }
  post(path, body, options) { return this.send("POST", path, { ...options, body }); }

  async signIn(username) {
    const response = await this.post("/api/auth/login", { username, password: PASSWORD });
    if (response.status !== 200) throw new Error(`signing ${username} in: ${response.status} ${response.text}`);
  }
}

const verify = (id) => `/api/auth/desktop/browser/verify/${id}`;
const tokenOf = (html) => /name="token" value="([^"]*)"/.exec(html)?.[1];

// ---------------------------------------------------------------------------

async function seed() {
  const owner = new Agent();
  const founded = await owner.post("/api/auth/register", {
    username: "maja", displayName: "Maja", email: "maja@example.test", password: PASSWORD, setupCode: args.setup,
  });
  if (founded.status !== 201) throw new Error(`founding: ${founded.status} ${founded.text}`);
  await owner.signIn("maja");
  for (const name of ["ada", "kim", "zed", "neo"]) {
    const invite = await owner.post("/api/admin/invites");
    const person = new Agent();
    const made = await person.post("/api/auth/register", {
      username: name, displayName: name[0].toUpperCase() + name.slice(1), email: `${name}@example.test`,
      password: PASSWORD, inviteCode: invite.json.url.split("/").pop(),
    });
    if (made.status !== 201) throw new Error(`registering ${name}: ${made.status} ${made.text}`);
  }
  writeFileSync(args.state, JSON.stringify({ ok: true }));
}

// ---------------------------------------------------------------------------

/** Turns what differs by nature into what does not, in the order it first appears. */
function normaliser() {
  const ids = new Map();
  const id = (text) => {
    const key = text.toLowerCase();
    if (!ids.has(key)) ids.set(key, `<id${ids.size + 1}>`);
    return ids.get(key);
  };
  return (text) => text
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, id)
    .replace(/\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z/g, "<time>")
    .replace(/(Mon|Tue|Wed|Thu|Fri|Sat|Sun), \d\d \w{3} \d{4} \d\d:\d\d:\d\d GMT/g, "<date>")
    .replace(/(name="token" value=")[A-Za-z0-9_-]{43}(")/g, "$1<token>$2")
    .replace(/"secret":"[A-Za-z0-9_-]{43}"/g, '"secret":"<secret>"')
    .replace(/traceId":"[^"]*"/g, 'traceId":"<trace>"');
}

const HIDDEN = new Set(["date", "server", "transfer-encoding", "connection", "keep-alive", "content-length"]);

function describe(label, response, clean) {
  const lines = [`## ${label}`, `status ${response.status}`];
  const names = [...response.headers.keys()].filter((name) => !HIDDEN.has(name) && name !== "set-cookie").sort();
  for (const name of names) lines.push(`${name}: ${clean(response.headers.get(name))}`);
  if ((response.headers.get("content-type") ?? "").startsWith("text/html")) {
    lines.push(`content-length: ${response.headers.get("content-length")}`);
  }
  for (const cookie of response.setCookie) lines.push(`set-cookie: ${clean(cookie.replace(/=[^;]*/, "=<value>"))}`);
  lines.push(`body ${JSON.stringify(clean(response.text))}`);
  return lines.join("\n");
}

class Transcript {
  constructor(path) {
    this.path = path;
    this.clean = normaliser();
    this.chunks = [];
  }

  add(label, response) {
    this.chunks.push(describe(label, response, this.clean));
    return response;
  }

  note(text) {
    this.chunks.push(`# ${this.clean(text)}`);
  }

  write() {
    writeFileSync(this.path, this.chunks.join("\n\n") + "\n");
  }
}

async function start(application, transcript, label) {
  const response = await application.post("/api/auth/desktop/browser/start");
  transcript.add(label, response);
  if (response.status !== 200) throw new Error(`start: ${response.status} ${response.text}`);
  return response.json;
}

async function ask() {
  const t = new Transcript(args.out);
  const ids = {};
  const application = new Agent(desktopClient);
  const [maja1, maja2, ada] = [new Agent(), new Agent(), new Agent()];
  await maja1.signIn("maja");
  await maja2.signIn("maja");
  await ada.signIn("ada");
  const nobody = new Agent();

  // The whole hand-over's browser half.
  const first = await start(application, t, "start");
  ids.first = first.requestId;
  t.note(`the address to open: ${first.verificationUrl}`);
  t.note(`the secret is ${first.secret.length} characters, the request lives ${(new Date(first.expiresAt) - Date.now()) > 0 ? "on" : "no longer"}`);
  t.add("page, nobody signed in", await nobody.get(verify(first.requestId)));
  t.add("page, an id that is no id", await maja1.get(verify("nope")));
  t.add("page, an id nobody started", await maja1.get(verify("11111111-2222-4333-8444-555555555555")));
  t.add("post, nobody signed in", await nobody.send("POST", verify(first.requestId), { form: "token=x" }));
  const page1 = t.add("page, maja", await maja1.get(verify(first.requestId)));
  const token1 = tokenOf(page1.text);
  t.add("page, ada after maja looked", await ada.get(verify(first.requestId)));
  const page2 = t.add("page, maja in another browser", await maja2.get(verify(first.requestId)));
  const token2 = tokenOf(page2.text);
  t.note(`the second look gives another token: ${token1 !== token2}`);
  t.add("post, the first page's token after the second look", await maja1.send("POST", verify(first.requestId), { form: `token=${token1}` }));
  t.add("post, the second page's token from the first browser", await maja1.send("POST", verify(first.requestId), { form: `token=${token2}` }));
  t.add("post, ada with maja's token", await ada.send("POST", verify(first.requestId), { form: `token=${token2}` }));
  t.add("post, no token", await maja2.send("POST", verify(first.requestId), { form: "" }));
  t.add("post, a token that is too short", await maja2.send("POST", verify(first.requestId), { form: "token=abc" }));
  t.add("post, a token of the right length that is wrong", await maja2.send("POST", verify(first.requestId), { form: `token=${"a".repeat(43)}` }));
  t.add("post, a foreign origin", await maja2.send("POST", verify(first.requestId), { form: `token=${token2}`, headers: { Origin: "https://evil.test" } }));
  t.add("post, a foreign referer", await maja2.send("POST", verify(first.requestId), { form: `token=${token2}`, headers: { Referer: "https://evil.test/x" } }));
  t.add("post, origin null", await maja2.send("POST", verify(first.requestId), { form: `token=${token2}`, headers: { Origin: "null" } }));
  t.add("post, origin with a path", await maja2.send("POST", verify(first.requestId), { form: `token=${token2}`, headers: { Origin: `${base}/x` } }));
  t.add("post, JSON", await maja2.send("POST", verify(first.requestId), { body: { token: token2 } }));
  t.add("post, no content type", await maja2.send("POST", verify(first.requestId), { raw: `token=${token2}` }));
  t.add("post, plain text", await maja2.send("POST", verify(first.requestId), { raw: `token=${token2}`, type: "text/plain" }));
  t.add("post, a multipart form that is no form", await maja2.send("POST", verify(first.requestId), { form: "garbage", type: "multipart/form-data; boundary=B" }));
  t.add("post, too many fields", await maja2.send("POST", verify(first.requestId), { form: Array.from({ length: 1025 }, (_, i) => `f${i}=1`).join("&") + `&token=${token2}` }));
  t.add("post, a name that is too long", await maja2.send("POST", verify(first.requestId), { form: `${"k".repeat(2049)}=1&token=${token2}` }));
  t.add("post, a value that is too long", await maja2.send("POST", verify(first.requestId), { form: `x=${"a".repeat(4194305)}&token=${token2}` }));
  t.add("post, the token twice", await maja2.send("POST", verify(first.requestId), { form: `token=${token2}&token=${token2}` }));
  t.add("page, the request is still not approved", await maja2.get(verify(first.requestId)));
  const page3 = await maja2.get(verify(first.requestId));
  const token3 = tokenOf(page3.text);
  t.add("post, approved", await maja2.send("POST", verify(first.requestId), { form: `token=${token3}`, headers: { Origin: base } }));
  t.add("post, approved again", await maja2.send("POST", verify(first.requestId), { form: `token=${token3}` }));
  t.add("page, after approval, the approver", await maja2.get(verify(first.requestId)));
  t.add("page, after approval, the approver's other browser", await maja1.get(verify(first.requestId)));
  t.add("page, after approval, another person", await ada.get(verify(first.requestId)));
  t.add("post, after approval, another person", await ada.send("POST", verify(first.requestId), { form: `token=${token3}` }));
  t.add("page, PUT", await maja2.send("PUT", verify(first.requestId)));
  t.add("page, HEAD", await maja2.send("HEAD", verify(first.requestId)));
  t.add("start, GET", await application.get("/api/auth/desktop/browser/start"));
  t.add("start, with a body", await application.send("POST", "/api/auth/desktop/browser/start", { body: { x: 1 } }));
  t.add("start, from the web client", await nobody.post("/api/auth/desktop/browser/start"));
  t.add("start, from no client", await fetch(`${base}/api/auth/desktop/browser/start`, { method: "POST" }).then(async (r) => ({ status: r.status, headers: r.headers, text: await r.text(), setCookie: [] })));

  // Each way the form's token is accepted, spelled differently.
  const spellings = {
    "a field name in capitals": (token) => `TOKEN=${token}`,
    "a percent-encoded value": (token) => `token=${token.replace("-", "%2D").replace("_", "%5F")}`,
    "an empty field before it": (token) => `token=&token=${token}`,
    "a multipart form": (token) => ({ form: `--B\r\nContent-Disposition: form-data; name="token"\r\n\r\n${token}\r\n--B--\r\n`, type: "multipart/form-data; boundary=B" }),
    "a content type with a character set": (token) => ({ form: `token=${token}`, type: "application/x-www-form-urlencoded; charset=utf-8" }),
    "a form of 1024 fields": (token) => Array.from({ length: 1023 }, (_, i) => `f${i}=1`).join("&") + `&token=${token}`,
    "an origin with slashes after it": (token) => ({ form: `token=${token}`, headers: { Origin: `${base}//` } }),
    "a referer with a path and user name": (token) => ({ form: `token=${token}`, headers: { Referer: `${base.replace("//", "//u:p@")}/a?b#c` } }),
    "a forwarded scheme and host the origin agrees with": (token) => ({ form: `token=${token}`, headers: { "X-Forwarded-Proto": "https", "X-Forwarded-Host": "chat.test", Origin: "https://chat.test" } }),
    "a forwarded scheme the origin does not": (token) => ({ form: `token=${token}`, headers: { "X-Forwarded-Proto": "https", Origin: base } }),
  };
  for (const [label, spell] of Object.entries(spellings)) {
    const handle = await start(application, new Transcript("/dev/null"), "");
    const token = tokenOf((await maja1.get(verify(handle.requestId))).text);
    const spelled = spell(token);
    const options = typeof spelled === "string" ? { form: spelled } : spelled;
    t.add(`post, ${label}`, await maja1.send("POST", verify(handle.requestId), options));
  }

  // Requests whose state the database is changed for: the script that runs
  // both servers does it between this and `after`. Each is opened by a
  // browser of its own, so that the change touches nothing else.
  const people = { ada2: "ada", zed: "zed", neo: "neo", kim1: "kim", kim2: "kim" };
  const browsers = { maja1 };
  for (const [name, account] of Object.entries(people)) {
    browsers[name] = new Agent();
    await browsers[name].signIn(account);
  }
  const watched = async (name, who, approve) => {
    const handle = await start(application, new Transcript("/dev/null"), "");
    ids.requests[name] = { id: handle.requestId, secret: handle.secret, browser: who };
    const page = await browsers[who].get(verify(handle.requestId));
    ids.requests[name].token = tokenOf(page.text);
    if (approve) await browsers[who].send("POST", verify(handle.requestId), { form: `token=${ids.requests[name].token}` });
  };
  ids.requests = {};
  await watched("ranOut", "maja1", false);
  await watched("used", "maja1", true);
  await watched("endedSession", "ada2", false);
  await watched("disabledAccount", "zed", false);
  await watched("passwordChanged", "neo", false);
  await watched("staleSecondStep", "kim1", false);
  await watched("freshSecondStep", "kim2", false);
  ids.cookies = Object.fromEntries(Object.entries(browsers).map(([name, agent]) => [name, Object.fromEntries(agent.cookies)]));
  writeFileSync(args.ids, JSON.stringify(ids));
  t.write();
}

async function after() {
  const ids = JSON.parse(readFileSync(args.ids, "utf8"));
  const t = new Transcript(args.out);
  const browser = (name) => {
    const found = new Agent();
    for (const [k, v] of Object.entries(ids.cookies[name])) found.cookies.set(k, v);
    return found;
  };
  const labels = {
    ranOut: "a request that ran out",
    used: "a request that was used",
    endedSession: "a session that ended",
    disabledAccount: "an account that was disabled",
    passwordChanged: "a password that was changed",
    staleSecondStep: "a second step passed ten minutes ago",
    freshSecondStep: "a second step passed a moment ago",
  };
  for (const [name, label] of Object.entries(labels)) {
    const request = ids.requests[name];
    const who = browser(request.browser);
    const page = t.add(`${label}: the page`, await who.get(verify(request.id)));
    // A look at the page that is let through makes a new token; one that is
    // not leaves the person with the token they had.
    const token = tokenOf(page.text) ?? request.token;
    t.add(`${label}: the post`, await who.send("POST", verify(request.id), { form: `token=${token}` }));
  }
  t.write();
}

if (mode === "seed") await seed();
else if (mode === "ask") await ask();
else if (mode === "after") await after();
else throw new Error("seed, ask or after");
