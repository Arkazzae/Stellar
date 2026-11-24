// What a client of the encryption routes observes, as a transcript, from one
// server: every answer (status, the headers a browser cares about, the JSON
// as written), and every event each of the connections that are open receives,
// in order. Run it against this server and against the first one on copies of
// one database (see encryption-oracle.sh) and compare the two transcripts.
//
//   node encryption-oracle.mjs --base http://127.0.0.1:5000 --phase story --out story.txt
//
// Needs Node 26 (the global WebSocket takes request headers). The accounts
// (maja, ada, ilya and zed), the conversations and what the story reads from the
// database are made by the shell script. Ids and moments are replaced by names
// and placeholders in the order they appear, so the two transcripts compare.

import { createHash } from "node:crypto";
import { writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({
  options: {
    base: { type: "string" },
    out: { type: "string" },
    phase: { type: "string", default: "story" },
    password: { type: "string", default: "Corona!2026-long" },
    ids: { type: "string" },
  },
});
const base = args.base;
const clientHeaders = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
const RS = "\u001e";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// ---- the transcript ----

const known = new Map();
const lines = [];
const out = (text) => lines.push(text);

function scrub(text) {
  let result = text;
  for (const [id, name] of known) result = result.replaceAll(id, `<${name}>`);
  const seen = new Map();
  result = result.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, (id) => {
    if (!seen.has(id)) seen.set(id, `id${seen.size + 1}`);
    return `<${seen.get(id)}>`;
  });
  return result
    .replace(/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z/g, "<time>")
    .replace(/00-[0-9a-f]{32}-[0-9a-f]{16}-00/g, "<trace>");
}

// ---- bytes the way a browser would send them, but the same every run ----

const bytes = (length, seed = 0) => Buffer.from(Array.from({ length }, (_, i) => (i * 31 + seed + 1) & 255));
const b64 = (buffer) => buffer.toString("base64");
const backupOf = (authKey) => ({
  salt: b64(bytes(16, 1)),
  iterations: 600000,
  verifier: createHash("sha256").update(authKey).digest("base64"),
  iv: b64(bytes(12, 2)),
  ciphertext: b64(bytes(64, 3)),
});

// ---- people ----

// Ids that a server makes up are named for what they are, so that transcripts
// agree on who is who, and arrays that come in the order of the ids (which
// differs from one run to the next) are put in the order of the names. That the
// ids were in order is checked first, here, on each side.
const labels = new Map();
function name(kind, label) {
  const n = (labels.get(`${kind}-${label}`) ?? 0) + 1;
  labels.set(`${kind}-${label}`, n);
  return `${kind}-${label}${n > 1 ? `-${n}` : ""}`;
}

function learn(person, method, path, status, text) {
  if (status !== 200 && status !== 201) return;
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    return;
  }
  const account = (key) => {
    if (key && !known.has(key.keyId)) known.set(key.keyId, name("key", person));
  };
  if (path === "/api/me/encryption" || path.endsWith("/account-key") || path.endsWith("/account-key/reset")) account(body.accountKey);
  if (path === "/api/me/encryption/devices" && method === "POST" && body.id && !known.has(body.id)) known.set(body.id, name("device", body.label));
  if (path === "/api/me/encryption/devices" && method === "GET") for (const device of body.devices) if (!known.has(device.id)) known.set(device.id, name("device", device.label));
  if (path === "/api/me/encryption" && body.device && !known.has(body.device.id)) known.set(body.device.id, name("device", body.device.label));
}

const label = (id) => known.get(id) ?? id;
let violations = [];
// An epoch that was just made has its envelopes in the order of the keys it
// was made for (account keys, then devices); one that is read has them in the
// order of the ids of the keys.
let made = false;

function normalise(value) {
  if (Array.isArray(value)) {
    const items = value.map(normalise);
    if (items.length > 1 && items.every((item) => typeof item === "string" && /^[0-9a-f-]{36}$/.test(item))) {
      if (items.some((id, i) => i > 0 && items[i - 1] > id)) violations.push("the ids are not in order");
      return items.sort((a, b) => label(a).localeCompare(label(b)));
    }
    if (items.length > 1 && items.every((item) => item && typeof item === "object" && "recipientKeyId" in item)) {
      const raw = items.map((item) => item.recipientKeyId);
      if (made) {
        const names = raw.map(label);
        if (names.some((n, i) => i > 0 && n.startsWith("key-") && names[i - 1].startsWith("device-"))) violations.push("the envelopes are not in the order they were made in");
      } else if (raw.some((id, i) => i > 0 && raw[i - 1] > id)) violations.push("the envelopes are not in the order of their keys");
      return items.sort((a, b) => label(a.recipientKeyId).localeCompare(label(b.recipientKeyId)));
    }
    if (items.length > 1 && items.every((item) => item && typeof item === "object" && "keyId" in item && "kind" in item && "userId" in item)) {
      const order = (item) => `${item.userId}|${item.kind}|${item.keyId}`;
      if (items.some((item, i) => i > 0 && order(items[i - 1]) > order(item))) violations.push("the recipients are not in order");
      const named = (item) => `${label(item.userId)}|${item.kind}|${label(item.keyId)}`;
      return items.sort((a, b) => named(a).localeCompare(named(b)));
    }
    return items;
  }
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, normalise(v)]));
  return value;
}

function shown(text, created = false) {
  violations = [];
  made = created;
  let result = text;
  try {
    result = JSON.stringify(normalise(JSON.parse(text)));
  } catch {
    return text;
  }
  return violations.length ? `${result} !! ${violations.join("; ")}` : result;
}

class Person {
  constructor(name) {
    this.name = name;
    this.cookies = new Map();
  }

  async request(method, path, body, extra = {}) {
    const headers = { ...clientHeaders, ...extra };
    let payload;
    if (body !== undefined) {
      headers["Content-Type"] = extra["Content-Type"] ?? "application/json";
      payload = typeof body === "string" ? body : JSON.stringify(body);
    }
    headers.Cookie = [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
    let response;
    try {
      response = await fetch(base + path, { method, headers, body: payload, redirect: "manual" });
    } catch (error) {
      // The server answered before it had read the whole body, and closed.
      return { status: 0, text: `the connection broke: ${error.cause?.code ?? error.message}`, headers: new Headers() };
    }
    for (const line of response.headers.getSetCookie()) {
      const [pair] = line.split(";");
      const index = pair.indexOf("=");
      const name = pair.slice(0, index);
      const value = pair.slice(index + 1);
      if (value === "" || /expires=Thu, 01 Jan 1970/i.test(line)) this.cookies.delete(name);
      else this.cookies.set(name, value);
    }
    const text = await response.text();
    learn(this.name, method, path, response.status, text);
    return { status: response.status, text, headers: response.headers };
  }

  get cookieHeader() {
    return [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
  }
}

async function signIn(name) {
  const person = new Person(name);
  const response = await person.request("POST", "/api/auth/login", { username: name, password: args.password });
  if (response.status !== 200) throw new Error(`signing in ${name}: ${response.status} ${response.text}`);
  const body = JSON.parse(response.text);
  person.id = body.id ?? body.user?.id;
  known.set(person.id, name);
  return person;
}

class Hub {
  constructor(label, person) {
    this.label = label;
    this.person = person;
    this.records = [];
    this.buffer = "";
  }

  async open() {
    this.socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app`, { headers: { Cookie: this.person.cookieHeader } });
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
        if (record.type === 1 && record.target !== "PresenceChanged") this.records.push(record);
      }
    };
    this.socket.send(`{"protocol":"json","version":1}${RS}`);
    await sleep(500);
    this.records = [];
    return this;
  }

  take() {
    const taken = this.records;
    this.records = [];
    return taken;
  }
}

const hubs = [];
async function listen(label, person) {
  const hub = await new Hub(label, person).open();
  hubs.push(hub);
  return hub;
}

function describe(response) {
  const headers = ["content-type", "location", "cache-control"]
    .map((name) => (response.headers.get(name) ? `${name}: ${response.headers.get(name)}` : null))
    .filter(Boolean);
  return `${response.status}${headers.length ? ` [${headers.join("; ")}]` : ""} ${shown(response.text, response.status === 201)}`;
}

async function step(title, action) {
  out(`## ${title}`);
  try {
    const result = await action();
    const list = Array.isArray(result) ? result : result === undefined ? [] : [result];
    for (const one of list) out(`   ${scrub(typeof one === "string" ? one : describe(one))}`);
  } catch (error) {
    out(`   error: ${error.message}`);
  }
  await sleep(450);
  for (const hub of hubs) {
    for (const record of hub.take()) out(`   ${hub.label} hears ${record.target} ${scrub(shown(JSON.stringify(record.arguments[0]), record.target === "DirectEncryptionEpochCreated"))}`);
  }
}

// ---- the story ----

const ACCOUNT = "/api/me/encryption";
const conversationPath = (id, what) => `/api/social/conversations/${id}/encryption/${what}`;
const uuid = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const pinOf = (n) => bytes(32, 100 + n);
const wrongPin = b64(bytes(32, 99));

async function story(ids) {
  const people = {};
  for (const name of ["maja", "ada", "ilya", "zed"]) people[name] = await signIn(name);
  const { maja, ada, ilya, zed } = people;
  const A = ids.A;
  const B = ids.B;
  const C = ids.C;
  const E = ids.E;
  for (const [id, name] of [[A, "A(maja-ada)"], [B, "B(maja-ilya)"], [C, "C(ada-ilya)"], [E, "E(ilya-zed)"]]) known.set(id, name);
  const mHub = await listen("maja", maja);
  const aHub = await listen("ada", ada);
  const iHub = await listen("ilya", ilya);
  void mHub;
  void aHub;
  void iHub;

  const setUp = (person, n) =>
    person.request("POST", `${ACCOUNT}/account-key`, { algorithm: "x25519", publicKey: b64(bytes(32, n)), backup: backupOf(pinOf(n)) });

  await step("nobody has anything yet", async () => [await maja.request("GET", ACCOUNT), await maja.request("GET", `${ACCOUNT}/backup`), await maja.request("GET", `${ACCOUNT}/devices`)]);
  await step("asking for a request or keys before setting up", async () => [
    await maja.request("POST", conversationPath(A, "request")),
    await ada.request("POST", conversationPath(A, "confirm")),
    await maja.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 1)), label: "Firefox" }),
    await maja.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: wrongPin }),
    await maja.request("PUT", `${ACCOUNT}/backup`, { currentPassword: "x", accountKeyId: uuid(1), backup: backupOf(pinOf(1)) }),
    await maja.request("PUT", `${ACCOUNT}/backup`, { currentPassword: args.password, accountKeyId: uuid(1), backup: backupOf(pinOf(1)) }),
    await maja.request("POST", `${ACCOUNT}/account-key/reset`, { currentPassword: args.password, algorithm: "x25519", publicKey: b64(bytes(32, 1)), backup: backupOf(pinOf(1)) }),
  ]);
  await step("refused set-ups change nothing", async () => {
    const good = { algorithm: "x25519", publicKey: b64(bytes(32, 1)), backup: backupOf(pinOf(1)) };
    const variants = [
      { ...good, publicKey: b64(bytes(31, 1)) },
      { ...good, algorithm: "rsa" },
      { ...good, backup: { ...good.backup, iterations: 1000 } },
      { ...good, backup: null },
      { ...good, backup: { ...good.backup, salt: b64(bytes(15, 1)) } },
      { ...good, backup: { ...good.backup, ciphertext: b64(bytes(16, 1)) } },
      {},
    ];
    const answers = [];
    for (const body of variants) answers.push(await maja.request("POST", `${ACCOUNT}/account-key`, body));
    answers.push(await maja.request("GET", ACCOUNT));
    return answers;
  });
  await step("maja, ada and zed set up; maja again", async () => [
    await setUp(maja, 1),
    await setUp(ada, 2),
    await setUp(zed, 4),
    await setUp(maja, 5),
    await maja.request("GET", ACCOUNT),
    await maja.request("GET", `${ACCOUNT}/backup`),
  ]);
  await step("another account sees nothing of maja's setup", async () => [await ilya.request("GET", ACCOUNT), await ilya.request("GET", `${ACCOUNT}/backup`)]);

  // Devices.
  const laptop = await signIn("maja");
  const phone = await signIn("maja");
  const adaPhone = await signIn("ada");
  await step("maja registers her browser, again, renamed, with a new key, and badly", async () => [
    await maja.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 11)), label: "  Firefox, Linux  " }),
    await maja.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 11)), label: "Firefox, Linux (renamed)" }),
    await maja.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 12)), label: "Firefox, Linux" }),
    await maja.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 12)), label: " " }),
    await maja.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(31, 12)), label: "x" }),
    await maja.request("GET", ACCOUNT),
    await maja.request("GET", `${ACCOUNT}/devices`),
  ]);
  await step("ada, ilya's key and ada's phone", async () => [
    await ada.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 21)), label: "Chrome, macOS" }),
    await adaPhone.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 22)), label: "Safari, iOS" }),
  ]);

  // Switches.
  await step("a request, twice, confirmed by the one who asked, confirmed by ada", async () => [
    await maja.request("POST", conversationPath(A, "request")),
    await maja.request("POST", conversationPath(A, "request")),
    await maja.request("POST", conversationPath(A, "confirm")),
    await ada.request("POST", conversationPath(A, "cancel")),
    await ada.request("POST", conversationPath(A, "confirm")),
    await ada.request("POST", conversationPath(A, "confirm")),
  ]);
  await step("the third person and a stranger find nothing", async () => [
    await ilya.request("POST", conversationPath(A, "request")),
    await ilya.request("POST", conversationPath(A, "disable")),
    await ilya.request("GET", conversationPath(A, "keys")),
    await ilya.request("GET", conversationPath(A, "epochs")),
    await zed.request("GET", conversationPath(A, "keys")),
    await maja.request("GET", conversationPath(uuid(7), "keys")),
    await maja.request("POST", conversationPath(uuid(7), "request")),
    await maja.request("POST", `/api/social/conversations/not-an-id/encryption/request`),
  ]);
  await step("the keys, as each person sees them", async () => [
    await maja.request("GET", conversationPath(A, "keys")),
    await ada.request("GET", conversationPath(A, "keys")),
    await adaPhone.request("GET", conversationPath(A, "keys")),
  ]);

  // Epochs.
  const keysOf = async (person, conversation) => {
    const response = await person.request("GET", conversationPath(conversation, "keys"));
    return JSON.parse(response.text);
  };
  const epochBody = (epoch, previous, recipients, tweak = (e) => e) => ({
    epochId: epoch,
    previousEpochId: previous,
    envelopes: recipients.map((keyId) => {
      // What an envelope holds depends on whom it is for, and not on where
      // that key stands in a list, which follows ids that differ per run.
      const seed = [...label(keyId)].reduce((sum, c) => (sum * 31 + c.charCodeAt(0)) & 0xff, 7);
      return tweak({
        recipientKeyId: keyId,
        ephemeralPublicKey: b64(bytes(32, seed)),
        iv: b64(bytes(12, seed + 1)),
        ciphertext: b64(bytes(48, seed + 2)),
      });
    }),
  });
  let keys = await keysOf(maja, A);
  const all = keys.recipients.map((r) => r.keyId);
  await step("epochs: stale, uncovered, spelled badly, then the first", async () => [
    await maja.request("POST", conversationPath(A, "epochs"), epochBody(uuid(101), uuid(999), all)),
    await maja.request("POST", conversationPath(A, "epochs"), epochBody(uuid(101), null, all.slice(1).concat([uuid(555)]))),
    await maja.request("POST", conversationPath(A, "epochs"), epochBody(uuid(101).toUpperCase(), null, all)),
    await maja.request("POST", conversationPath(A, "epochs"), epochBody(uuid(101), null, all)),
    await maja.request("POST", conversationPath(A, "epochs"), epochBody(uuid(101), null, all)),
    await ada.request("POST", conversationPath(A, "epochs"), epochBody(uuid(101), uuid(101), all)),
  ]);
  await step("epochs: out of order and in order", async () => [
    await ada.request("POST", conversationPath(A, "epochs"), epochBody(uuid(102), null, all)),
    await ada.request("POST", conversationPath(A, "epochs"), epochBody(uuid(103), uuid(101), all)),
    await maja.request("POST", conversationPath(A, "epochs"), epochBody(uuid(104), uuid(101), all)),
    await maja.request("POST", conversationPath(A, "epochs"), epochBody(uuid(104), uuid(103), all)),
  ]);
  await step("what each reads of them", async () => [
    await maja.request("GET", conversationPath(A, "keys")),
    await ada.request("GET", conversationPath(A, "keys")),
    await adaPhone.request("GET", conversationPath(A, "keys")),
    await maja.request("GET", conversationPath(A, "epochs")),
    await ada.request("GET", conversationPath(A, "epochs?after=1")),
    await adaPhone.request("GET", conversationPath(A, "epochs?after=2")),
    await maja.request("GET", conversationPath(A, "epochs?after=")),
    await maja.request("GET", conversationPath(A, "epochs?after=x")),
    await maja.request("GET", conversationPath(A, "epochs?after=%202")),
    await maja.request("GET", conversationPath(A, "epochs?after=%2B2")),
    await maja.request("GET", conversationPath(A, "epochs?after=+2")),
    await maja.request("GET", conversationPath(A, "epochs?after=2&after=3")),
    await maja.request("GET", conversationPath(A, "epochs?after=1.0")),
    await maja.request("GET", conversationPath(A, "epochs?after=99999999999")),
    await maja.request("GET", conversationPath(A, "epochs?after=-1")),
    await maja.request("GET", conversationPath(A, "epochs?AFTER=2")),
    await maja.request("GET", conversationPath(A, "epochs?after=2%20")),
    await maja.request("GET", conversationPath(A, "epochs?after=%20")),
    await maja.request("GET", conversationPath(A, "epochs?after")),
  ]);
  await step("epoch envelopes of the wrong size", async () => {
    keys = await keysOf(maja, A);
    const current = keys.encryption.currentEpochId;
    const post = (body) => maja.request("POST", conversationPath(A, "epochs"), body);
    return [
      await post(epochBody(uuid(110), current, all, (e) => ({ ...e, iv: b64(bytes(11, 1)) }))),
      await post(epochBody(uuid(110), current, all, (e) => ({ ...e, iv: b64(bytes(13, 1)) }))),
      await post(epochBody(uuid(110), current, all, (e) => ({ ...e, ciphertext: b64(bytes(16, 1)) }))),
      await post(epochBody(uuid(110), current, all, (e) => ({ ...e, ciphertext: b64(bytes(257, 1)) }))),
      await post(epochBody(uuid(110), current, all, (e) => ({ ...e, ephemeralPublicKey: b64(bytes(33, 1)) }))),
      await post(epochBody(uuid(110), current, all, (e) => ({ ...e, ephemeralPublicKey: null }))),
      await post(epochBody(uuid(110), current, all, (e) => ({ ...e, recipientKeyId: e.recipientKeyId.toUpperCase() }))),
      await post(epochBody(uuid(110), current, [...all, all[0]])),
      await post({ epochId: uuid(110), previousEpochId: current, envelopes: [] }),
      await post({ epochId: uuid(110), previousEpochId: current }),
      await post({ ...epochBody(uuid(110), current, all), previousEpochId: "nope" }),
      await post(epochBody(uuid(110), current, all, (e) => ({ ...e, ciphertext: b64(bytes(17, 1)) }))),
    ];
  });

  // Devices change the recipients.
  await step("a new device of maja's, while the conversation is on", async () => [
    await phone.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 61)), label: "Phone" }),
    await phone.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 61)), label: "Phone" }),
    await ada.request("GET", conversationPath(A, "keys")),
  ]);
  await step("removing the keys of a device", async () => {
    const own = JSON.parse((await maja.request("GET", `${ACCOUNT}/devices`)).text).devices;
    const other = own.find((d) => !d.current);
    return [
      await maja.request("DELETE", `${ACCOUNT}/devices/${own.find((d) => d.current).id}`),
      await ada.request("DELETE", `${ACCOUNT}/devices/${other.id}`),
      await maja.request("DELETE", `${ACCOUNT}/devices/${uuid(77)}`),
      await maja.request("DELETE", `${ACCOUNT}/devices/not-an-id`),
      await maja.request("DELETE", `${ACCOUNT}/devices/${other.id}`),
      await phone.request("GET", "/api/me"),
      await maja.request("DELETE", `${ACCOUNT}/devices/${other.id}`),
      await maja.request("GET", `${ACCOUNT}/devices`),
      await ada.request("GET", conversationPath(A, "keys")),
    ];
  });
  await step("a laptop signs out", async () => [
    await laptop.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 71)), label: "Laptop" }),
    await laptop.request("POST", "/api/auth/logout"),
    await maja.request("GET", `${ACCOUNT}/devices`),
  ]);

  // The backup and its attempts.
  await step("the PIN: a right one, wrong ones, a right one, and the tenth in a row", async () => {
    const answers = [await maja.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: b64(pinOf(1)) })];
    for (let i = 0; i < 3; i++) answers.push(await maja.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: wrongPin }));
    answers.push(await maja.request("GET", `${ACCOUNT}/backup`));
    answers.push(await maja.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: b64(pinOf(1)) }));
    answers.push(await maja.request("GET", `${ACCOUNT}/backup`));
    for (let i = 0; i < 10; i++) answers.push(await maja.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: wrongPin }));
    answers.push(await maja.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: b64(pinOf(1)) }));
    answers.push(await maja.request("GET", `${ACCOUNT}/backup`));
    answers.push(await maja.request("GET", ACCOUNT));
    return answers;
  });
  await step("the PIN: proofs of the wrong size or shape", async () => [
    await ada.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: b64(bytes(31, 1)) }),
    await ada.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: b64(bytes(33, 1)) }),
    await ada.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: "" }),
    await ada.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: null }),
    await ada.request("POST", `${ACCOUNT}/backup/unlock`, {}),
    await ada.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: "***" }),
    await ada.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: 5 }),
    await ada.request("GET", `${ACCOUNT}/backup`),
  ]);
  await step("a new PIN", async () => {
    const accountKeyId = JSON.parse((await maja.request("GET", ACCOUNT)).text).accountKey.keyId;
    const put = (body) => maja.request("PUT", `${ACCOUNT}/backup`, body);
    return [
      await put({ currentPassword: "nope", accountKeyId, backup: backupOf(pinOf(8)) }),
      await put({ accountKeyId, backup: backupOf(pinOf(8)) }),
      await put({ currentPassword: args.password, accountKeyId: uuid(3), backup: backupOf(pinOf(8)) }),
      await put({ currentPassword: args.password, accountKeyId: accountKeyId.toUpperCase(), backup: backupOf(pinOf(8)) }),
      await put({ currentPassword: args.password, accountKeyId, backup: null }),
      await put({ currentPassword: args.password, accountKeyId, backup: { ...backupOf(pinOf(8)), iterations: 599999 } }),
      await put({ currentPassword: args.password, accountKeyId, backup: backupOf(pinOf(8)) }),
      await maja.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: b64(pinOf(1)) }),
      await maja.request("POST", `${ACCOUNT}/backup/unlock`, { authKey: b64(pinOf(8)) }),
      await maja.request("GET", ACCOUNT),
    ];
  });
  await step("ilya is told nothing of any of it, and sets up and asks", async () => [
    await ilya.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 81)), label: "Ilya's" }),
    await setUp(ilya, 7),
    await ilya.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 81)), label: "Ilya's" }),
    await ilya.request("POST", conversationPath(C, "request")),
    await ada.request("POST", conversationPath(C, "confirm")),
    await maja.request("POST", conversationPath(B, "request")),
    await ilya.request("POST", conversationPath(B, "confirm")),
    await ilya.request("POST", conversationPath(E, "request")),
    await zed.request("POST", conversationPath(E, "request")),
    await zed.request("GET", conversationPath(E, "keys")),
    await ilya.request("GET", conversationPath(E, "epochs")),
    await zed.request("POST", conversationPath(E, "epochs"), epochBody(uuid(201), null, [uuid(1)])),
  ]);

  // A reset.
  await step("a reset", async () => {
    const body = (password) => ({ currentPassword: password, algorithm: "x25519", publicKey: b64(bytes(32, 91)), backup: backupOf(pinOf(9)) });
    return [
      await maja.request("POST", `${ACCOUNT}/account-key/reset`, body("nope")),
      await maja.request("POST", `${ACCOUNT}/account-key/reset`, { ...body(args.password), algorithm: "rsa" }),
      await maja.request("POST", `${ACCOUNT}/account-key/reset`, body(args.password)),
      await maja.request("GET", ACCOUNT),
      await ada.request("GET", conversationPath(A, "keys")),
      await ada.request("GET", conversationPath(A, "epochs")),
    ];
  });
  await step("ada turns it off, an epoch after that, and a second turning off", async () => [
    await ada.request("POST", conversationPath(A, "disable")),
    await maja.request("POST", conversationPath(A, "epochs"), epochBody(uuid(301), null, all)),
    await maja.request("POST", conversationPath(A, "epochs"), epochBody(uuid(101), null, all)),
    await maja.request("POST", conversationPath(A, "disable")),
    await maja.request("GET", conversationPath(A, "epochs")),
  ]);

  // A block, as the database was prepared with.
  await step("a block closes the conversation", async () => [
    await ilya.request("POST", conversationPath(E, "request")),
    await zed.request("POST", conversationPath(E, "disable")),
    await zed.request("GET", conversationPath(E, "keys")),
    await ilya.request("GET", conversationPath(E, "epochs")),
    await ilya.request("POST", conversationPath(E, "epochs"), epochBody(uuid(401), null, [uuid(1)])),
  ]);

  // The limit of devices, with the places the database was prepared with.
  await step("zed's devices up to the limit and one more", async () => {
    const zedLaptop = await signIn("zed");
    const zedPhone = await signIn("zed");
    const answers = [];
    answers.push(await zed.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 1)), label: "main" }));
    answers.push(await zedLaptop.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 2)), label: "laptop" }));
    answers.push(await zedPhone.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 3)), label: "phone" }));
    answers.push(await zed.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 1)), label: "main again" }));
    answers.push(await zed.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 4)), label: "main, new key" }));
    const devices = JSON.parse((await zed.request("GET", `${ACCOUNT}/devices`)).text).devices;
    answers.push(`devices: ${devices.length}, the first label ${devices[0].label}`);
    answers.push(await zed.request("DELETE", `${ACCOUNT}/devices/${devices[0].id}`));
    answers.push(await zedPhone.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 3)), label: "phone" }));
    return answers;
  });
}

// ---- how both servers read what a client may send ----

async function tables(ids) {
  const maja = await signIn("maja");
  const ada = await signIn("ada");
  const A = ids.A;
  known.set(A, "A");
  const good = (n = 1) => ({ algorithm: "x25519", publicKey: b64(bytes(32, n)), backup: backupOf(pinOf(n)) });
  const send = (person, method, path, raw, type = "application/json") =>
    person.request(method, path, raw, { "Content-Type": type });
  const compact = async (label, response) => out(`${label.padEnd(44)} ${scrub(describe(await response))}`);

  out("## set-up, in raw bodies");
  const first = await maja.request("POST", `${ACCOUNT}/account-key`, good());
  out(scrub(describe(first)));
  const raw = (publicKey, backup = JSON.stringify(good().backup)) => `{"algorithm":"x25519","publicKey":${publicKey},"backup":${backup}}`;
  const K = b64(bytes(32, 1));
  const spelled = [
    ["valid", `"${K}"`],
    ["space inside", `"${K.slice(0, 10)} ${K.slice(10)}"`],
    ["newline inside", `"${K.slice(0, 10)}\\n${K.slice(10)}"`],
    ["tab inside", `"${K.slice(0, 10)}\\t${K.slice(10)}"`],
    ["cr inside", `"${K.slice(0, 10)}\\r${K.slice(10)}"`],
    ["padded with spaces", `" ${K} "`],
    ["vertical tab", `"${K.slice(0, 10)}\\u000b${K.slice(10)}"`],
    ["no-break space", `"${K.slice(0, 10)}\\u00a0${K.slice(10)}"`],
    ["unpadded", `"${K.replace(/=+$/, "")}"`],
    ["extra padding", `"${K}="`],
    ["url alphabet", `"${b64(Buffer.alloc(32, 0xfb)).replace(/\+/g, "-")}"`],
    ["standard +/", `"${b64(Buffer.alloc(32, 0xfb))}"`],
    ["trailing bits", `"${b64(Buffer.alloc(32, 0)).slice(0, 42)}B="`],
    ["escaped slash", `"${b64(Buffer.alloc(32, 0xff)).replace(/\//g, "\\/")}"`],
    ["escaped letters", `"${K.replace(/A/g, "\\u0041")}"`],
    ["padding in the middle", `"AAAA=AAA${K.slice(8)}"`],
    ["null", "null"],
    ["empty", '""'],
    ["spaces only", '"   "'],
    ["a number", "5"],
    ["true", "true"],
    ["an array", "[1,2]"],
    ["an object", "{}"],
    ["31 bytes", `"${b64(bytes(31, 1))}"`],
    ["33 bytes", `"${b64(bytes(33, 1))}"`],
  ];
  for (const [label, publicKey] of spelled) await compact(`publicKey ${label}`, send(maja, "POST", `${ACCOUNT}/account-key`, raw(publicKey)));
  const backupWith = (change) => JSON.stringify({ ...good().backup, ...change });
  const iterations = ["600000", '"600000"', '"+600000"', '"0600000"', '" 600000"', '"600000 "', "600000.0", "6e5", "600000.5", "599999", "10000000", "10000001", "-1", "0", "null", "true", '"abc"', '""', "1e400", "600000000000", "2147483648", '"2147483648"', '"-5"', "[600000]", "{}"];
  for (const value of iterations) {
    const body = `{"algorithm":"x25519","publicKey":"${K}","backup":{"salt":"${b64(bytes(16, 1))}","iterations":${value},"verifier":"${b64(bytes(32, 1))}","iv":"${b64(bytes(12, 1))}","ciphertext":"${b64(bytes(64, 1))}"}}`;
    await compact(`iterations ${value}`, send(maja, "POST", `${ACCOUNT}/account-key`, body));
  }
  for (const [field, lengths] of [["salt", [0, 15, 16, 64, 65]], ["verifier", [0, 31, 32, 33]], ["iv", [0, 11, 12, 13]], ["ciphertext", [0, 16, 17, 1024, 1025]]]) {
    for (const length of lengths) await compact(`${field} of ${length} bytes`, send(maja, "POST", `${ACCOUNT}/account-key`, raw(`"${K}"`, backupWith({ [field]: b64(bytes(length, 1)) }))));
    await compact(`${field} null`, send(maja, "POST", `${ACCOUNT}/account-key`, raw(`"${K}"`, backupWith({ [field]: null }))));
  }
  for (const [label, body] of [
    ["algorithm upper case", `{"algorithm":"X25519","publicKey":"${K}","backup":${JSON.stringify(good().backup)}}`],
    ["algorithm with a space", `{"algorithm":"x25519 ","publicKey":"${K}","backup":${JSON.stringify(good().backup)}}`],
    ["algorithm null", `{"algorithm":null,"publicKey":"${K}","backup":${JSON.stringify(good().backup)}}`],
    ["algorithm a number", `{"algorithm":5,"publicKey":"${K}","backup":null}`],
    ["a member twice", `{"algorithm":"rsa","algorithm":"x25519","publicKey":"${K}","backup":${JSON.stringify(good().backup)}}`],
    ["a member twice, the wrong type first", `{"algorithm":5,"algorithm":"x25519","publicKey":"${K}","backup":${JSON.stringify(good().backup)}}`],
    ["a byte order mark", `﻿${raw(`"${K}"`)}`],
    ["a comment", `/* c */${raw(`"${K}"`)}`],
    ["a trailing comma", raw(`"${K}"`).replace(/}$/, ",}")],
    ["trailing garbage", `${raw(`"${K}"`)} x`],
    ["trailing space", `${raw(`"${K}"`)} \n`],
    ["empty body", ""],
    ["null body", "null"],
    ["an array body", "[]"],
    ["a string body", '"x"'],
    ["a number body", "5"],
    ["names in other case", `{"Algorithm":"x25519","PublicKey":"${K}","Backup":${JSON.stringify(good().backup)}}`],
    ["unknown members", `{"extra":{"a":[1,2,{"b":null}]},${raw(`"${K}"`).slice(1)}`],
  ]) await compact(label, send(maja, "POST", `${ACCOUNT}/account-key`, body));
  await compact("a content type of text", send(maja, "POST", `${ACCOUNT}/account-key`, raw(`"${K}"`), "text/plain"));
  await compact("a content type with a charset", send(maja, "POST", `${ACCOUNT}/account-key`, raw(`"${K}"`), "application/json; charset=utf-8"));
  await compact("a content type of problem+json", send(maja, "POST", `${ACCOUNT}/account-key`, raw(`"${K}"`), "application/problem+json"));
  await compact("a body over the ceiling", send(maja, "POST", `${ACCOUNT}/devices`, `{"label":"${"a".repeat(1024 * 1024)}"}`));
  await compact("a body just under the ceiling", send(maja, "POST", `${ACCOUNT}/devices`, `{"label":"${"a".repeat(1024 * 1024 - 20)}"}`));

  out("## devices: the label and the key");
  const device = (label, key = b64(bytes(32, 5)), algorithm = "x25519") => JSON.stringify({ algorithm, publicKey: key, label });
  for (const [what, label] of [["a", "a"], ["a hundred", "a".repeat(100)], ["a hundred and one", "a".repeat(101)], ["a hundred emoji pairs", "\u{1F600}".repeat(50)], ["fifty-one emoji", "\u{1F600}".repeat(51)], ["spaces only", "   "], ["empty", ""], ["tabs and breaks", "\t\n"], ["no-break space", " "], ["padded", `  ${"b".repeat(100)}  `], ["padded too long", ` ${"b".repeat(101)} `], ["ideographic space", `　x　`], ["zero width space", "​"], ["null", null]])
    await compact(`label ${what}`, send(maja, "POST", `${ACCOUNT}/devices`, device(label)));
  await compact("label missing", send(maja, "POST", `${ACCOUNT}/devices`, `{"algorithm":"x25519","publicKey":"${b64(bytes(32, 5))}"}`));
  await compact("label a number", send(maja, "POST", `${ACCOUNT}/devices`, `{"algorithm":"x25519","publicKey":"${b64(bytes(32, 5))}","label":5}`));
  await compact("device key 31 bytes", send(maja, "POST", `${ACCOUNT}/devices`, device("x", b64(bytes(31, 5)))));
  await compact("device key 33 bytes", send(maja, "POST", `${ACCOUNT}/devices`, device("x", b64(bytes(33, 5)))));
  await compact("device key empty", send(maja, "POST", `${ACCOUNT}/devices`, device("x", "")));
  await compact("device key null", send(maja, "POST", `${ACCOUNT}/devices`, `{"algorithm":"x25519","publicKey":null,"label":"x"}`));
  await compact("device algorithm rsa", send(maja, "POST", `${ACCOUNT}/devices`, device("x", b64(bytes(32, 5)), "rsa")));

  out("## conversation: the switch needs a key, the epoch needs a conversation that is on");
  await ada.request("POST", `${ACCOUNT}/account-key`, good(2));
  await ada.request("POST", `${ACCOUNT}/devices`, { algorithm: "x25519", publicKey: b64(bytes(32, 6)), label: "ada" });
  await maja.request("POST", conversationPath(A, "request"));
  await ada.request("POST", conversationPath(A, "confirm"));
  const keys = JSON.parse((await maja.request("GET", conversationPath(A, "keys"))).text);
  const all = keys.recipients.map((r) => r.keyId);
  const envelope = (keyId, over = {}) => ({ recipientKeyId: keyId, ephemeralPublicKey: b64(bytes(32, 7)), iv: b64(bytes(12, 8)), ciphertext: b64(bytes(48, 9)), ...over });
  const epoch = (envelopes, over = {}) => JSON.stringify({ epochId: uuid(900), previousEpochId: null, envelopes, ...over });
  const post = (body, who = maja) => send(who, "POST", conversationPath(A, "epochs"), body);
  for (const [label, body] of [
    ["an envelope that is a hole", epoch([null, ...all.slice(1).map((k) => envelope(k))])],
    ["an envelope that is a number", epoch([5, ...all.slice(1).map((k) => envelope(k))])],
    ["envelopes an object", epoch({})],
    ["envelopes null", epoch(null)],
    ["envelopes a string", epoch("x")],
    ["a previous epoch of an upper-case id", epoch(all.map((k) => envelope(k)), { previousEpochId: uuid(5).toUpperCase() })],
    ["a previous epoch without hyphens", epoch(all.map((k) => envelope(k)), { previousEpochId: uuid(5).replaceAll("-", "") })],
    ["a previous epoch in braces", epoch(all.map((k) => envelope(k)), { previousEpochId: `{${uuid(5)}}` })],
    ["a previous epoch empty", epoch(all.map((k) => envelope(k)), { previousEpochId: "" })],
    ["an epoch id without hyphens", epoch(all.map((k) => envelope(k)), { epochId: uuid(900).replaceAll("-", "") })],
    ["an epoch id in braces", epoch(all.map((k) => envelope(k)), { epochId: `{${uuid(900)}}` })],
    ["an epoch id null", epoch(all.map((k) => envelope(k)), { epochId: null })],
    ["an epoch id a number", epoch(all.map((k) => envelope(k)), { epochId: 5 })],
    ["a recipient in braces", epoch(all.map((k, i) => envelope(i === 0 ? `{${k}}` : k)))],
    ["a recipient without hyphens", epoch(all.map((k, i) => envelope(i === 0 ? k.replaceAll("-", "") : k)))],
    ["an envelope with a key that is a number", epoch(all.map((k, i) => envelope(k, i === 0 ? { ephemeralPublicKey: 5 } : {})))],
    ["an envelope with an iv in a wrong encoding", epoch(all.map((k, i) => envelope(k, i === 0 ? { iv: "***" } : {})))],
    ["202 envelopes of strangers", epoch(Array.from({ length: 202 }, (_, i) => envelope(uuid(1000 + i))))],
    ["203 envelopes of strangers", epoch(Array.from({ length: 203 }, (_, i) => envelope(uuid(1000 + i))))],
    ["one envelope with a ciphertext of 256", epoch(all.map((k, i) => envelope(k, i === 0 ? { ciphertext: b64(bytes(256, 1)) } : {})))],
  ]) await compact(label, post(body));
  await compact("the requests asked of a stranger's conversation", send(ada, "POST", `/api/social/conversations/${uuid(5)}/encryption/epochs`, epoch([envelope(uuid(1))])));
  out("## a path that is not an id");
  for (const [label, id] of [["without hyphens", A.replaceAll("-", "")], ["in braces", `{${A}}`], ["in capitals", A.toUpperCase()], ["in parentheses", `(${A})`]])
    await compact(`keys of a conversation ${label}`, maja.request("GET", conversationPath(id, "keys")));
  for (const [label, id] of [["without hyphens", A.replaceAll("-", "")], ["in capitals", A.toUpperCase()]])
    await compact(`request in a conversation ${label}`, maja.request("POST", conversationPath(id, "request")));
  await compact("a device id in capitals", maja.request("DELETE", `${ACCOUNT}/devices/${uuid(5).toUpperCase()}`));
  await compact("a device id without hyphens", maja.request("DELETE", `${ACCOUNT}/devices/${uuid(5).replaceAll("-", "")}`));
  out("## methods that are not served");
  for (const [method, path] of [["HEAD", ACCOUNT], ["PUT", ACCOUNT], ["DELETE", ACCOUNT], ["GET", `${ACCOUNT}/account-key`], ["POST", `${ACCOUNT}/backup`], ["GET", `${ACCOUNT}/backup/unlock`], ["PATCH", `${ACCOUNT}/devices`], ["GET", conversationPath(A, "request")], ["DELETE", conversationPath(A, "epochs")]])
    await compact(`${method} ${path.replace(A, "A")}`, maja.request(method, path));
  await compact("nobody signed in: account", new Person("nobody").request("GET", ACCOUNT));
  await compact("nobody signed in: unlock", new Person("nobody").request("POST", `${ACCOUNT}/backup/unlock`, { authKey: b64(bytes(32, 1)) }));
  await compact("nobody signed in: keys", new Person("nobody").request("GET", conversationPath(A, "keys")));
}

const ids = JSON.parse(args.ids);
if (args.phase === "story") await story(ids);
else await tables(ids);
for (const hub of hubs) hub.socket.close(1000);
writeFileSync(args.out, lines.join("\n") + "\n");
process.exit(0);
