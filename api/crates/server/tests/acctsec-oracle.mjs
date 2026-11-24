// What a signed-in person does to the security of their own account, asked of
// this server and of the first one as transcripts to compare.
//
//   node acctsec-oracle.mjs prepare --base URL --setup CODE --out state.json
//   node acctsec-oracle.mjs ask     --base URL --state state.json --out transcript.txt --public URL --mail PORT
//   node acctsec-oracle.mjs mail-down --base URL --state state.json --out transcript.txt --public URL --mail PORT
//   node acctsec-oracle.mjs limits  --base URL --out transcript.txt --only PATH
//
// `prepare` runs on each server, fresh: it founds the instance and signs a
// roster of accounts in, most of them in more than one browser. Between
// `prepare` and `ask` the script that runs both (acctsec-shared-database.sh)
// edits each database file in the same way, so that the accounts have second
// steps, passwords or none, sessions that passed a second step a while ago or
// only just, and so on; the sessions made by `prepare` stay as they are.
// `ask` then puts every route to every kind of account, with a hub connection
// open for each browser, and writes what each request answered, which
// connections closed and which mails were sent, with the ids, moments and
// secrets that differ by nature replaced.
//
// Needs Node 26 (the global WebSocket takes request headers).

import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    base: { type: "string" },
    setup: { type: "string" },
    state: { type: "string" },
    out: { type: "string" },
    public: { type: "string" },
    mail: { type: "string" },
    only: { type: "string" },
  },
});
const mode = positionals[0];
const base = args.base;
const PASSWORD = "Corona!2026-long";
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "User-Agent": "acctsec-probe" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** The accounts, and how many browsers each is signed in with. */
const ROSTER = {
  solo: 3, // no second step, signed in just now
  old: 2, // no second step, sign-in too old to stand in for a password
  armed: 2, // a second step; the first browser passed it lately, the second did not
  armed2: 3, // the same, for taking methods away
  sole: 2, // exactly one method
  ppk: 2, // no password, one passkey
  ppk2: 2, // no password, a passkey and an authenticator app
  pwpk: 2, // a password and one passkey
  pk2: 2, // no password, two passkeys
  nopass: 2, // no password, signed in just now
  nopassold: 1, // no password, sign-in too old
  nopassm: 1, // no password, a second step the session did not pass lately
  mailer: 2, // a second step of its own with mail, for the address
  other: 1, // somebody else, for ids that are not the caller's
};
const TABS = "abc";

/** A person's browser: a cookie jar and the web client's headers. */
class Browser {
  constructor(name) {
    this.name = name;
    this.cookies = new Map();
  }

  async send(method, path, { body, raw, type, headers = {} } = {}) {
    const request = { method, headers: { ...clientHeaders, ...headers }, redirect: "manual" };
    if (this.cookies.size > 0) request.headers.Cookie = [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
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
      if (value === "" || /expires=Thu, 01 Jan 1970/i.test(line)) this.cookies.delete(pair.slice(0, at));
      else this.cookies.set(pair.slice(0, at), value);
    }
    const text = await response.text();
    let json = null;
    try {
      json = JSON.parse(text);
    } catch {}
    return { status: response.status, headers: response.headers, text, json, setCookies: response.headers.getSetCookie() };
  }

  async login(name) {
    const response = await this.send("POST", "/api/auth/login", { body: { username: name, password: PASSWORD } });
    if (response.status !== 200) throw new Error(`signing in ${name}: ${response.status} ${response.text}`);
    this.id = response.json.id ?? response.json.user?.id;
    return this;
  }
}

const lines = [];
const out = (text) => lines.push(text);
const names = new Map();
let fresh = 0;

/** Ids, moments, codes and addresses that differ by nature are replaced; the rest is kept. */
function scrub(text) {
  let result = text;
  result = result.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, (id) => {
    const key = id.toLowerCase();
    if (!names.has(key)) names.set(key, `new${++fresh}`);
    return `<${names.get(key)}>`;
  });
  result = result.replace(/(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(\.\d+)?(Z?)/g, (_, _a, _b, z) => `<time${z}>`);
  result = result.replace(/"traceId":"[^"]*"/g, '"traceId":"<trace>"');
  result = result.replace(/"ipAddress":"[^"]*"/g, '"ipAddress":"<ip>"');
  result = result.replace(/^IP address: .*$/gm, "IP address: <ip>").replace(/^When: .*$/gm, "When: <when>");
  result = result.replace(/oklch\(0\.58 0\.14 \d+\)/g, "oklch(<hue>)");
  result = result.replace(/\b[A-Z0-9]{7}(-[A-Z0-9]{7}){3}\b/g, "<recovery-code>");
  result = result.replace(/token=[A-Za-z0-9_-]{20,}/g, "token=<token>");
  result = result.replace(/=[A-Za-z0-9_%.\-]{40,}; ?/g, "=<value>; ");
  result = result.replace(/expires=[A-Za-z]+, [0-9]+ [A-Za-z]+ [0-9]+ [0-9:]+ GMT/gi, "expires=<date>");
  result = result.replace(/max-age=\d+/gi, "max-age=<n>");
  if (args.public) result = result.split(args.public).join("<origin>");
  return result;
}

function describe(response) {
  const parts = [String(response.status)];
  for (const name of ["content-type", "cache-control", "pragma", "expires", "location", "allow", "retry-after", "www-authenticate"]) {
    const value = response.headers.get(name);
    if (value) parts.push(`${name}: ${value}`);
  }
  for (const line of response.setCookies ?? []) parts.push(`set-cookie: ${line.replace(/^([^=]*)=[^;]*/, "$1=<value>")}`);
  parts.push(`BODY: ${response.text}`);
  return scrub(parts.join(" | "));
}

/** One hub connection of one browser. It notes when the server closes it. */
class Listener {
  constructor(browser) {
    this.browser = browser;
    this.closed = false;
  }

  async open() {
    const cookie = [...this.browser.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
    this.socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app`, { headers: { Cookie: cookie, ...clientHeaders } });
    await new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = () => reject(new Error(`the hub refused ${this.browser.name}`));
    });
    this.socket.onclose = () => {
      this.closed = true;
    };
    this.socket.send(`{"protocol":"json","version":1}${RS}`);
    return this;
  }
}

// ---------------------------------------------------------------------------

async function prepare() {
  const state = { people: {}, jars: {} };
  const owner = new Browser("owner");
  const founded = await owner.send("POST", "/api/auth/register", {
    body: { username: "owner", displayName: "Owner", email: "owner@example.test", password: PASSWORD, setupCode: args.setup },
  });
  if (founded.status !== 201) throw new Error(`founding: ${founded.status} ${founded.text}`);
  state.people.owner = (await owner.login("owner")).id;
  state.jars.owner = [[...owner.cookies]];
  for (const [name, tabs] of Object.entries(ROSTER)) {
    const invite = await owner.send("POST", "/api/admin/invites", {});
    if (invite.status !== 201) throw new Error(`an invite: ${invite.status} ${invite.text}`);
    const code = invite.json.url.split("/").pop();
    const first = new Browser(name);
    const registered = await first.send("POST", "/api/auth/register", {
      body: { username: name, displayName: name[0].toUpperCase() + name.slice(1), email: `${name}@example.test`, password: PASSWORD, inviteCode: code },
    });
    if (registered.status !== 201) throw new Error(`registering ${name}: ${registered.status} ${registered.text}`);
    state.jars[name] = [];
    for (let tab = 0; tab < tabs; tab++) {
      // Each tab is a sign-in of its own, a moment apart so that they have an order.
      await sleep(30);
      const browser = new Browser(name);
      await browser.login(name);
      state.people[name] = browser.id;
      state.jars[name].push([...browser.cookies]);
    }
  }
  writeFileSync(args.out, JSON.stringify(state));
}

// ---------------------------------------------------------------------------

async function mails() {
  const port = args.mail;
  const list = await (await fetch(`http://127.0.0.1:${port}/api/v1/messages`)).json();
  const found = [];
  for (const summary of list.messages ?? []) {
    const message = await (await fetch(`http://127.0.0.1:${port}/api/v1/message/${summary.ID}`)).json();
    const to = message.To.map((x) => x.Address).join(",");
    const text = (message.Text ?? "").replace(/\r/g, "").trim();
    // The mail library writes the domain of an address in lower case; the first server kept it as typed.
    found.push(`TO ${to.replace(/@.*$/, (domain) => domain.toLowerCase())} | SUBJECT ${message.Subject}\n${text}`);
  }
  return found.sort();
}

async function clearMail() {
  await fetch(`http://127.0.0.1:${args.mail}/api/v1/messages`, { method: "DELETE" });
}

/** Waits until the mail has stopped arriving: the alerts are sent by a worker. */
async function settledMail(expectAtLeast = 0, waitUpTo = 25000) {
  const until = Date.now() + waitUpTo;
  let count = -1;
  let stable = 0;
  while (Date.now() < until) {
    const now = (await mails()).length;
    stable = now === count ? stable + 1 : 0;
    count = now;
    if (stable >= 3 && now >= expectAtLeast) break;
    await sleep(1500);
  }
  return mails();
}

function load(state) {
  const people = {};
  for (const [name, id] of Object.entries(state.people)) names.set(id, `person:${name}`);
  for (const [name, jars] of Object.entries(state.jars)) {
    people[name] = jars.map((jar, index) => {
      const browser = new Browser(`${name}.${TABS[index]}`);
      browser.cookies = new Map(jar);
      browser.id = state.people[name];
      return browser;
    });
  }
  return people;
}

async function ask() {
  const state = JSON.parse(readFileSync(args.state, "utf8"));
  const people = load(state);
  const tab = (name, letter) => people[name]["abc".indexOf(letter)];
  const anonymous = new Browser("anonymous");
  const listeners = {};
  for (const [name, browsers] of Object.entries(people)) {
    for (const browser of browsers) listeners[browser.name] = await new Listener(browser).open();
  }
  await sleep(600);
  await clearMail();

  /** One request; what the hub did about it follows when the request ends sessions. */
  async function step(label, browser, method, path, options = {}) {
    const response = await browser.send(method, path, options);
    out(`## ${label}`);
    out(describe(response));
    if (options.ends) {
      await sleep(2600);
      const closed = Object.entries(listeners).filter(([, listener]) => listener.closed).map(([name]) => name);
      out(`  hub closed so far: ${closed.join(", ") || "nobody"}`);
    }
    return response;
  }
  const get = (label, who, path, options) => step(label, who, "GET", path, options);
  const post = (label, who, path, body, options = {}) => step(label, who, "POST", path, { body, ...options });
  const del = (label, who, path, options) => step(label, who, "DELETE", path, options);
  const idOf = (response, member) => response.json?.[member];
  const sessionsOf = async (who) => (await who.send("GET", "/api/me/security/sessions")).json ?? [];
  const sessionIdOf = async (who) => (await sessionsOf(who)).find((s) => s.current)?.id;
  const secure = "/api/me/security";
  const NIL = "00000000-0000-0000-0000-000000000000";
  // The confirmations of an address are printed where they are asked for. The
  // alerts are sent by a worker on its own clock: they are collected and
  // printed once, sorted, after it has had time to look more than once.
  const alerts = [];
  const tokens = new Map();
  const confirmFor = (label, address, again = false) =>
    post(label, anonymous, "/api/auth/email/confirm", { token: tokens.get(address) ?? "missing" });
  const mailNow = async (label, expect = 0, wait = 8000) => {
    out(`## mail: ${label}`);
    const until = Date.now() + wait;
    let confirmations = [];
    for (;;) {
      const found = await mails();
      confirmations = found.filter((message) => message.includes("SUBJECT Confirm your new address"));
      if (confirmations.length >= expect || Date.now() > until) {
        for (const message of found) if (!confirmations.includes(message) && !alerts.includes(message)) alerts.push(message);
        break;
      }
      await sleep(500);
    }
    for (const message of confirmations) {
      out(scrub(message));
      tokens.set(message.match(/^TO ([^ ]+)/)[1], message.match(/token=([A-Za-z0-9_-]+)/)?.[1]);
    }
    await clearMail();
  };
  const alertsAtTheEnd = async () => {
    await sleep(Number(process.env.ALERT_WAIT_MS ?? 40000));
    for (const message of await mails()) if (!alerts.includes(message)) alerts.push(message);
    out("## mail: the alerts, all of them");
    for (const message of alerts.map(scrub).sort()) out(message);
  };

  // ----- nobody signed in: every route
  const routes = [
    ["GET", secure], ["GET", `${secure}/events`], ["POST", `${secure}/mfa/enroll/start`], ["POST", `${secure}/mfa/reverify/start`],
    ["POST", `${secure}/mfa/${NIL}/remove`], ["POST", `${secure}/recovery/regenerate`], ["DELETE", `${secure}/trusted/${NIL}`],
    ["GET", `${secure}/sessions`], ["DELETE", `${secure}/sessions/${NIL}`], ["POST", `${secure}/sessions/revoke-others`],
    ["POST", `${secure}/password`], ["POST", "/api/me/password"], ["POST", "/api/me/email"],
  ];
  for (const [method, path] of routes) await step(`anonymous ${method} ${path}`, anonymous, method, path, { body: method === "POST" ? {} : undefined });
  await post("anonymous: confirm, no token", anonymous, "/api/auth/email/confirm", {});

  // ----- an account with no second step, signed in just now
  const solo = tab("solo", "a");
  await get("solo: the page", solo, secure);
  await get("solo: the events", solo, `${secure}/events`);
  await get("solo: sessions", solo, `${secure}/sessions`);
  await del("solo: end my own session", solo, `${secure}/sessions/${await sessionIdOf(solo)}`);
  await del("solo: end a session that is not one", solo, `${secure}/sessions/not-an-id`);
  await del("solo: end an id nobody has", solo, `${secure}/sessions/${NIL}`);
  const otherSession = await sessionIdOf(tab("other", "a"));
  await del("solo: end another person's session", solo, `${secure}/sessions/${otherSession}`);
  await get("other: still signed in", tab("other", "a"), "/api/session");
  const soloB = (await sessionsOf(tab("solo", "b"))).find((s) => s.current).id;
  await del("solo: end the second tab", solo, `${secure}/sessions/${soloB}`, { ends: true });
  await del("solo: end it again", solo, `${secure}/sessions/${soloB}`);
  await get("solo: second tab is out", tab("solo", "b"), "/api/session");
  await del("solo: forget a device nobody has", solo, `${secure}/trusted/${NIL}`);
  await del("solo: forget a device, not an id", solo, `${secure}/trusted/x`);
  await post("solo: remove a method, not an id", solo, `${secure}/mfa/x/remove`, {});
  await post("solo: remove a method nobody has", solo, `${secure}/mfa/${NIL}/remove`, {});
  await post("solo: remove a method, no body", solo, `${secure}/mfa/${NIL}/remove`, undefined);
  await step("solo: remove a method, a body that is not JSON", solo, "POST", `${secure}/mfa/${NIL}/remove`, { raw: "x", type: "text/plain" });
  await step("solo: remove a method, broken JSON", solo, "POST", `${secure}/mfa/${NIL}/remove`, { raw: "{", type: "application/json" });
  await post("solo: new recovery codes, no second step", solo, `${secure}/recovery/regenerate`, {});
  await post("solo: new recovery codes, no body", solo, `${secure}/recovery/regenerate`, undefined);
  await post("solo: prove the second step again, none to prove", solo, `${secure}/mfa/reverify/start`, undefined);
  await post("solo: set up a second step, no body", solo, `${secure}/mfa/enroll/start`, undefined);
  await post("solo: set up a second step, empty object", solo, `${secure}/mfa/enroll/start`, {});
  await post("solo: set up a second step, wrong password", solo, `${secure}/mfa/enroll/start`, { password: "nope" });
  await step("solo: set up a second step, a body that is not JSON", solo, "POST", `${secure}/mfa/enroll/start`, { raw: "x", type: "text/plain" });
  await step("solo: set up a second step, broken JSON", solo, "POST", `${secure}/mfa/enroll/start`, { raw: "{", type: "application/json" });
  await post("solo: first password, already has one", solo, `${secure}/password`, { newPassword: "A-long-new-password" });
  await post("solo: first password, too short", solo, `${secure}/password`, { newPassword: "short" });
  await post("solo: first password, missing", solo, `${secure}/password`, {});
  await post("solo: first password, too long", solo, `${secure}/password`, { newPassword: "x".repeat(257) });
  await step("solo: first password, no body", solo, "POST", `${secure}/password`, {});
  // A body that is JSON but not an object.
  for (const [label, path] of [
    ["remove a method", `${secure}/mfa/${NIL}/remove`], ["new recovery codes", `${secure}/recovery/regenerate`],
    ["first password", `${secure}/password`], ["change the password", "/api/me/password"], ["address", "/api/me/email"],
    ["set up a second step", `${secure}/mfa/enroll/start`],
  ]) {
    await step(`solo: ${label}, an array`, solo, "POST", path, { raw: '["Corona!2026-long","A-long-new-password"]', type: "application/json" });
    await step(`solo: ${label}, null`, solo, "POST", path, { raw: "null", type: "application/json" });
    await step(`solo: ${label}, a string`, solo, "POST", path, { raw: '"x"', type: "application/json" });
  }
  // The password, its policy, and what each refusal says.
  await post("solo: change, too short", solo, "/api/me/password", { currentPassword: PASSWORD, newPassword: "short" });
  await post("solo: change, too long", solo, "/api/me/password", { currentPassword: PASSWORD, newPassword: "x".repeat(257) });
  await post("solo: change, nothing given", solo, "/api/me/password", {});
  await post("solo: change, no current", solo, "/api/me/password", { newPassword: "A-long-new-password" });
  await post("solo: change, wrong current", solo, "/api/me/password", { currentPassword: "nope-nope-nope", newPassword: "A-long-new-password" });
  await post("solo: change, the same", solo, "/api/me/password", { currentPassword: PASSWORD, newPassword: PASSWORD });
  await post("solo: change, exactly eight", solo, "/api/me/password", { currentPassword: "nope-nope-nope", newPassword: "12345678" });
  await post("solo: change, exactly 256", solo, "/api/me/password", { currentPassword: "nope-nope-nope", newPassword: "x".repeat(256) });
  await step("solo: change, a body that is not JSON", solo, "POST", "/api/me/password", { raw: "x", type: "text/plain" });
  await step("solo: change, no content type", solo, "POST", "/api/me/password", { raw: "{}" });
  // The address.
  for (const [label, email] of [
    ["not an address", "not-an-address"], ["empty", ""], ["a name and an address", "Ala <ala@example.test>"],
    ["too long", `${"a".repeat(310)}@example.test`], ["two at signs", "a@@example.test"],
  ]) await post(`solo: address, ${label}`, solo, "/api/me/email", { email, password: PASSWORD });
  await post("solo: address, none given", solo, "/api/me/email", { password: PASSWORD });
  await post("solo: address, wrong password", solo, "/api/me/email", { email: "solo-new@example.test", password: "nope-nope-nope" });
  await post("solo: address, no password", solo, "/api/me/email", { email: "solo-new@example.test" });
  await post("solo: address, my own", solo, "/api/me/email", { email: "solo@example.test", password: PASSWORD });
  await post("solo: address, my own in capitals", solo, "/api/me/email", { email: "SOLO@Example.Test", password: PASSWORD });
  await post("solo: address, somebody else's", solo, "/api/me/email", { email: "other@example.test", password: PASSWORD });
  await post("solo: address, somebody else's in capitals", solo, "/api/me/email", { email: "OTHER@example.test", password: PASSWORD });
  await post("solo: address, the dotless i", solo, "/api/me/email", { email: "other@EXAMPLE.TEST".replace("o", "ı"), password: PASSWORD });
  await post("solo: address, a new one, with spaces round it", solo, "/api/me/email", { email: "  solo-new@example.test  ", password: PASSWORD });
  await mailNow("the confirmation of the first address", 1);
  await post("solo: address, a second one while the first waits", solo, "/api/me/email", { email: "solo-newer@example.test", password: PASSWORD });
  await mailNow("the confirmation of the second address", 1);
  await confirmFor("solo: confirm the first address, which the second replaced", "solo-new@example.test");
  await confirmFor("solo: confirm the second address", "solo-newer@example.test");
  await confirmFor("solo: confirm it again", "solo-newer@example.test");
  await get("solo: the events after the address changed", solo, `${secure}/events`);
  await post("solo: confirm, nonsense", anonymous, "/api/auth/email/confirm", { token: "nonsense" });
  await post("solo: confirm, too long", anonymous, "/api/auth/email/confirm", { token: "x".repeat(257) });
  await post("solo: confirm, blank", anonymous, "/api/auth/email/confirm", { token: "   " });
  // The password changes while two browsers are signed in.
  const soloC = tab("solo", "c");
  const soloAfter = await post("solo: change the password", solo, "/api/me/password", { currentPassword: PASSWORD, newPassword: "A-long-new-password" }, { ends: true });
  await get("solo: the browser that changed it still works", solo, "/api/session");
  await get("solo: the third tab is out", soloC, "/api/session");
  await get("solo: the events afterwards", solo, `${secure}/events`);
  await get("solo: the page afterwards", solo, secure);
  await post("solo: sign in with the old password", anonymous, "/api/auth/login", { username: "solo", password: PASSWORD });
  await post("solo: sign in with the new one", anonymous, "/api/auth/login", { username: "solo", password: "A-long-new-password" });
  await post("solo: end the others, nobody else left", solo, `${secure}/sessions/revoke-others`, undefined);

  // ----- no second step, a sign-in too old to stand in for a password
  const old = tab("old", "a");
  await get("old: the page", old, secure);
  await post("old: set up a second step, no password", old, `${secure}/mfa/enroll/start`, undefined);
  await post("old: set up a second step, wrong password", old, `${secure}/mfa/enroll/start`, { password: "nope" });
  await post("old: set up a second step, the password", old, `${secure}/mfa/enroll/start`, { password: PASSWORD });
  await post("old: new recovery codes", old, `${secure}/recovery/regenerate`, {});
  await post("old: remove a method nobody has", old, `${secure}/mfa/${NIL}/remove`, {});
  await post("old: prove again", old, `${secure}/mfa/reverify/start`, undefined);
  await post("old: change the password, wrong", old, "/api/me/password", { currentPassword: "nope-nope-nope", newPassword: "A-long-new-password" });
  await post("old: address, wrong password", old, "/api/me/email", { email: "old-new@example.test", password: "nope-nope-nope" });
  await post("old: address, right password", old, "/api/me/email", { email: "old-new@example.test", password: PASSWORD });
  await mailNow("old", 1);
  await del("old: end the other tab, which did not ask", old, `${secure}/sessions/${await sessionIdOf(tab("old", "b"))}`);
  await confirmFor("old: confirm the address of the session that is still there", "old-new@example.test");
  await post("old: address, from the tab that is left", old, "/api/me/email", { email: "old-newer@example.test", password: PASSWORD });
  await mailNow("old, again", 1);
  await post("old: first password, has one", old, `${secure}/password`, { newPassword: "A-long-new-password" });

  // ----- a second step; one browser passed it lately, the other did not
  const armedFresh = tab("armed", "a");
  const armedStale = tab("armed", "b");
  await get("armed: the page, fresh browser", armedFresh, secure);
  await get("armed: the page, stale browser", armedStale, secure);
  await get("armed: sessions", armedFresh, `${secure}/sessions`);
  const armed = (await armedFresh.send("GET", secure)).json;
  const methodId = armed.methods[0].id;
  const deviceId = armed.trustedDevices[0].id;
  for (const [label, body] of [["no password", {}], ["the password", { password: PASSWORD }], ["a wrong password", { password: "nope" }]]) {
    await post(`armed stale: set up a second step, ${label}`, armedStale, `${secure}/mfa/enroll/start`, body);
    await post(`armed stale: new recovery codes, ${label}`, armedStale, `${secure}/recovery/regenerate`, body);
    await post(`armed stale: remove a method, ${label}`, armedStale, `${secure}/mfa/${methodId}/remove`, body);
    await post(`armed stale: remove a method nobody has, ${label}`, armedStale, `${secure}/mfa/${NIL}/remove`, body);
    await post(`armed stale: address, ${label}`, armedStale, "/api/me/email", { email: "armed-new@example.test", password: body.password });
  }
  await post("armed stale: prove again", armedStale, `${secure}/mfa/reverify/start`, undefined);
  await post("armed stale: change the password", armedStale, "/api/me/password", { currentPassword: PASSWORD, newPassword: "A-long-new-password" });
  await post("armed stale: change the password, wrong", armedStale, "/api/me/password", { currentPassword: "nope-nope-nope", newPassword: "A-long-new-password" });
  await post("armed stale: change the password, too short", armedStale, "/api/me/password", { currentPassword: PASSWORD, newPassword: "short" });
  await post("armed stale: change the password, too long", armedStale, "/api/me/password", { currentPassword: PASSWORD, newPassword: "x".repeat(257) });
  await post("armed stale: address, not an address", armedStale, "/api/me/email", { email: "nope", password: PASSWORD });
  await post("armed stale: first password, too short", armedStale, `${secure}/password`, { newPassword: "short" });
  await post("armed stale: first password", armedStale, `${secure}/password`, { newPassword: "A-long-new-password" });
  await del("armed stale: forget a device nobody has", armedStale, `${secure}/trusted/${NIL}`);
  // A browser that is remembered shows on the page and in a challenge to set up, not in one to prove again.
  armedFresh.cookies.set("__Host-stellarr-trust", "known-device-token");
  await get("armed fresh: the page with a remembered browser", armedFresh, secure);
  await post("armed fresh: set up a second step from a remembered browser", armedFresh, `${secure}/mfa/enroll/start`, undefined);
  await post("armed fresh: prove again from a remembered browser", armedFresh, `${secure}/mfa/reverify/start`, undefined);
  armedFresh.cookies.delete("__Host-stellarr-trust");
  await post("armed fresh: new recovery codes, a wrong password", armedFresh, `${secure}/recovery/regenerate`, { password: "nope" });
  await get("armed fresh: the page, ten codes", armedFresh, secure);
  await post("armed fresh: remove a method nobody has", armedFresh, `${secure}/mfa/${NIL}/remove`, {});
  await post("armed fresh: prove again", armedFresh, `${secure}/mfa/reverify/start`, undefined);
  await post("armed fresh: set up a second step", armedFresh, `${secure}/mfa/enroll/start`, undefined);
  await post("armed fresh: address, wrong password", armedFresh, "/api/me/email", { email: "armed-new@example.test", password: "nope-nope-nope" });
  await post("armed fresh: address, right password", armedFresh, "/api/me/email", { email: "armed-new@example.test", password: PASSWORD });
  await mailNow("armed", 1);
  await get("armed fresh: the page before the address changes", armedFresh, secure);
  await confirmFor("armed: confirm the address", "armed-new@example.test");
  await get("armed fresh: the page after the address changed", armedFresh, secure);
  await get("armed fresh: the events after the address changed", armedFresh, `${secure}/events`);
  await post("armed fresh: address, again", armedFresh, "/api/me/email", { email: "armed-newer@example.test", password: PASSWORD });
  await mailNow("armed, a second address", 1);
  await del("armed fresh: forget a remembered browser", armedFresh, `${secure}/trusted/${deviceId}`);
  await del("armed fresh: forget it again", armedFresh, `${secure}/trusted/${deviceId}`);
  await get("armed fresh: the page, one device left", armedFresh, secure);
  await post("armed fresh: change the password, wrong", armedFresh, "/api/me/password", { currentPassword: "nope-nope-nope", newPassword: "A-long-new-password" });
  await post("armed fresh: change the password", armedFresh, "/api/me/password", { currentPassword: PASSWORD, newPassword: "A-long-new-password" }, { ends: true });
  await get("armed fresh: the page afterwards", armedFresh, secure);
  await confirmFor("armed: confirm the address after the password changed", "armed-newer@example.test");
  await get("armed: the stale browser is out", armedStale, "/api/session");

  // ----- taking methods away
  const armed2 = tab("armed2", "a");
  const two = (await armed2.send("GET", secure)).json;
  await post("armed2: remove the authenticator app", armed2, `${secure}/mfa/${two.methods[0].id}/remove`, {}, { ends: true });
  await get("armed2: the page afterwards", armed2, secure);
  await get("armed2: the second tab is out", tab("armed2", "b"), "/api/session");
  await post("armed2: remove the same again", armed2, `${secure}/mfa/${two.methods[0].id}/remove`, {});
  await post("armed2: remove the last method", armed2, `${secure}/mfa/${two.methods[1].id}/remove`, {});
  await get("armed2: the page, no methods", armed2, secure);
  const sole = tab("sole", "a");
  const soleMethod = (await sole.send("GET", secure)).json.methods[0].id;
  await post("sole: remove the only method", sole, `${secure}/mfa/${soleMethod}/remove`, {}, { ends: true });
  await get("sole: the page", sole, secure);
  // The instance asks its administrators for a second step: the owner has one.
  const owner = people.owner[0];
  await get("owner: the page, the instance asks for a second step", owner, secure);
  const ownerMethod = (await owner.send("GET", secure)).json.methods[0].id;
  await post("owner: remove the only method", owner, `${secure}/mfa/${ownerMethod}/remove`, {});

  // ----- the last way in
  const ppk = tab("ppk", "a");
  const ppkMethod = (await ppk.send("GET", secure)).json.methods[0].id;
  await get("ppk: the page", ppk, secure);
  await post("ppk: remove the only passkey", ppk, `${secure}/mfa/${ppkMethod}/remove`, {});
  await post("ppk: first password", ppk, `${secure}/password`, { newPassword: "A-long-new-password" });
  await post("ppk: remove the passkey, now there is a password", ppk, `${secure}/mfa/${ppkMethod}/remove`, {}, { ends: true });
  await get("ppk: the page", ppk, secure);
  const ppk2 = tab("ppk2", "a");
  const ppk2Methods = (await ppk2.send("GET", secure)).json.methods;
  const passkey = ppk2Methods.find((m) => m.type === "passkey").id;
  const totp = ppk2Methods.find((m) => m.type === "totp").id;
  await post("ppk2: remove the passkey", ppk2, `${secure}/mfa/${passkey}/remove`, {});
  await post("ppk2: remove the authenticator app", ppk2, `${secure}/mfa/${totp}/remove`, {});
  await get("ppk2: the page", ppk2, secure);
  const pwpk = tab("pwpk", "a");
  const pwpkMethod = (await pwpk.send("GET", secure)).json.methods[0].id;
  await post("pwpk: remove the passkey, there is a password", pwpk, `${secure}/mfa/${pwpkMethod}/remove`, {});
  const pk2 = tab("pk2", "a");
  const pk2Methods = (await pk2.send("GET", secure)).json.methods;
  await post("pk2: remove one of two passkeys", pk2, `${secure}/mfa/${pk2Methods[0].id}/remove`, {});
  await post("pk2: remove the other", pk2, `${secure}/mfa/${pk2Methods[1].id}/remove`, {});

  // ----- no password: made through a provider
  const nopass = tab("nopass", "a");
  await get("nopass: the page", nopass, secure);
  await post("nopass: change the password", nopass, "/api/me/password", { currentPassword: PASSWORD, newPassword: "A-long-new-password" });
  await post("nopass: address, a password", nopass, "/api/me/email", { email: "nopass-new@example.test", password: PASSWORD });
  await post("nopass: address, none", nopass, "/api/me/email", { email: "nopass-new@example.test" });
  await mailNow("nopass", 1);
  await post("nopass: set up a second step", nopass, `${secure}/mfa/enroll/start`, undefined);
  await post("nopass: first password, too short", nopass, `${secure}/password`, { newPassword: "short" });
  await post("nopass: first password", nopass, `${secure}/password`, { newPassword: "A-long-new-password" });
  await post("nopass: first password again", nopass, `${secure}/password`, { newPassword: "A-long-new-password" });
  await get("nopass: the page afterwards", nopass, secure);
  await get("nopass: the other tab is still in", tab("nopass", "b"), "/api/session");
  await get("nopass: the events", nopass, `${secure}/events`);
  const nopassold = tab("nopassold", "a");
  await post("nopassold: first password", nopassold, `${secure}/password`, { newPassword: "A-long-new-password" });
  await post("nopassold: first password, too short", nopassold, `${secure}/password`, { newPassword: "short" });
  await post("nopassold: set up a second step", nopassold, `${secure}/mfa/enroll/start`, undefined);
  await post("nopassold: set up a second step, with a password", nopassold, `${secure}/mfa/enroll/start`, { password: PASSWORD });
  await post("nopassold: address", nopassold, "/api/me/email", { email: "nopassold-new@example.test", password: PASSWORD });
  await post("nopassold: change the password", nopassold, "/api/me/password", { currentPassword: PASSWORD, newPassword: "A-long-new-password" });
  const nopassm = tab("nopassm", "a");
  await post("nopassm: first password", nopassm, `${secure}/password`, { newPassword: "A-long-new-password" });
  await post("nopassm: address", nopassm, "/api/me/email", { email: "nopassm-new@example.test" });

  // ----- the address of an account that confirms by mail
  const mailer = tab("mailer", "a");
  await post("mailer: address, right password", mailer, "/api/me/email", { email: "mailer-new@example.test", password: PASSWORD });
  await mailNow("mailer", 1);
  await get("mailer: the page", mailer, secure);
  await confirmFor("mailer: confirm the address", "mailer-new@example.test");
  await get("mailer: the page afterwards", mailer, secure);
  await get("mailer: the events afterwards", mailer, `${secure}/events`);
  await get("mailer: the other tab", tab("mailer", "b"), "/api/session");

  await alertsAtTheEnd();
  writeFileSync(args.out, lines.join("\n") + "\n");
  for (const listener of Object.values(listeners)) listener.socket.close();
  process.exit(0);
}

// ---------------------------------------------------------------------------
// Mail that cannot be sent: the address is written all the same.

async function mailDown() {
  const state = JSON.parse(readFileSync(args.state, "utf8"));
  const people = load(state);
  const solo = people.solo[0];
  const down = people.mailer[1];
  const post = async (label, who, path, body) => {
    const response = await who.send("POST", path, { body });
    out(`## ${label}`);
    out(describe(response));
  };
  await post("the mail server is down: a new address", down, "/api/me/email", { email: "down-new@example.test", password: PASSWORD });
  await post("the mail server is down: the same again", down, "/api/me/email", { email: "down-new@example.test", password: PASSWORD });
  await post("the mail server is down: somebody else's address", down, "/api/me/email", { email: "other@example.test", password: PASSWORD });
  writeFileSync(args.out, lines.join("\n") + "\n");
}

// ---------------------------------------------------------------------------
// The limits of each route: one refusal of each, from a server that has the
// real ones.

const SIBLINGS = {
  "/api/me/security/mfa/enroll/start": [
    "/api/me/security/mfa/reverify/start", "/api/me/security/mfa/00000000-0000-0000-0000-000000000000/remove",
    "/api/me/security/recovery/regenerate", "/api/me/security/password", "/api/me/password", "/api/me/email",
  ],
  "/api/me/password": ["/api/auth/login", "/api/me/security/password", "/api/me/email"],
  "/api/me/email": ["/api/auth/register", "/api/me/password", "/api/me/security/password"],
};

async function limits() {
  const anonymous = new Browser("anonymous");
  const cases = [
    ["POST", "/api/me/email", 5], ["POST", "/api/auth/email/confirm", 5], ["POST", "/api/me/password", 10],
    ["POST", "/api/me/security/mfa/enroll/start", 30], ["POST", "/api/me/security/mfa/reverify/start", 30],
    ["POST", "/api/me/security/mfa/00000000-0000-0000-0000-000000000000/remove", 30],
    ["POST", "/api/me/security/recovery/regenerate", 30], ["POST", "/api/me/security/password", 30],
  ];
  for (const [method, path, permits] of cases) {
    // Policies are shared by routes: a fresh server is needed for each, so the
    // script that runs this asks for one per case (--only).
    if (args.only && args.only !== path) continue;
    let last;
    let first429 = 0;
    for (let n = 1; n <= permits + 2; n++) {
      last = await anonymous.send(method, path, { body: {} });
      if (last.status === 429 && !first429) {
        first429 = n;
        out(`## ${method} ${path}: the request that is refused is number ${n}`);
        out(describe(last));
      }
    }
    if (!first429) out(`## ${method} ${path}: never refused in ${permits + 2} requests`);
    // The routes that share the policy are refused now, the others are not.
    for (const sibling of SIBLINGS[path] ?? []) {
      const answer = await anonymous.send("POST", sibling, { body: {} });
      out(`## after that, ${sibling}: ${answer.status}`);
    }
  }
  writeFileSync(args.out, lines.join("\n") + "\n");
}

if (mode === "prepare") await prepare();
if (mode === "ask") await ask();
if (mode === "mail-down") await mailDown();
if (mode === "limits") await limits();
