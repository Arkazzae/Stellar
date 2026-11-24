// Runs every flow of the identity provider group against one server and prints
// a conversation: what was asked, and the status, the redirect, the cookies'
// attributes and the body of each answer, with ids, times and secrets blanked.
// The script runs it against both servers and diffs the two.
//
//   node driver.mjs <label> <base> <setup-code> <control-port> <pass> <fake-host>
//
// pass: `flows`, or `limits` (one refusal of each rate limit).
const [label, base, setupCode, control, pass, fakeHost] = process.argv.slice(2);
const out = [];
const log = (...parts) => out.push(parts.join(" "));
const HEADERS = { "x-stellarr-client": "web", "x-stellarr-client-version": "2.0.0" };

class Browser {
  constructor() { this.jar = new Map(); }
  async send(method, path, { json, form, headers } = {}) {
    const h = { ...HEADERS, ...headers };
    // As a browser does: a cookie goes only to the paths under its own.
    const here = path.split("?")[0];
    const cookie = [...this.jar]
      .filter(([, c]) => here === c.path || here.startsWith(c.path.endsWith("/") ? c.path : `${c.path}/`) || c.path === "/")
      .map(([k, c]) => `${k}=${c.value}`).join("; ");
    if (cookie) h.cookie = cookie;
    let body;
    if (json !== undefined) { h["content-type"] = "application/json"; body = JSON.stringify(json); }
    if (form !== undefined) { h["content-type"] = "application/x-www-form-urlencoded"; body = form; }
    const res = await fetch(base + path, { method, headers: h, body, redirect: "manual" });
    const lines = res.headers.getSetCookie();
    for (const line of lines) {
      const [pair] = line.split(";");
      const at = pair.indexOf("=");
      const name = pair.slice(0, at).trim(), value = pair.slice(at + 1).trim();
      const cookiePath = /;\s*path=([^;]*)/i.exec(line)?.[1] ?? "/";
      if (value === "" || /expires=Thu, 01 Jan 1970/i.test(line)) this.jar.delete(name); else this.jar.set(name, { value, path: cookiePath });
    }
    return { status: res.status, headers: res.headers, cookies: lines, text: await res.text() };
  }
  get(path, o) { return this.send("GET", path, o); }
  post(path, json) { return this.send("POST", path, { json }); }
}

const blank = (s) => s
  .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, "<id>")
  .replace(/\b[0-9a-f]{32}\b/gi, "<id>")
  .replace(/(state|nonce|ticket|code_challenge)=[^&\s]+/g, "$1=<$1>")
  .replace(/\bp4[45]\b/g, "pK")
  .replace(/"traceId":"[^"]*"/g, "\"traceId\":\"<trace>\"")
  .replace(/\d{4}-\d\d-\d\dT[\d:.]+Z/g, "<time>")
  .replace(new RegExp(base.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "g"), "<base>")
  .replace(/:\d{4,5}\b/g, ":<port>");

function cookieShape(line) {
  const parts = line.split(";").map((p) => p.trim());
  const name = parts[0].split("=")[0];
  const kind = /^(\.AspNetCore\.(Correlation|OpenIdConnect\.Nonce)|stellarr-oidc-)/.test(name) ? "<binding>" : name;
  const deleted = parts[0].endsWith("=") ? "deleted" : "set";
  const rest = parts.slice(1).map((p) => p.replace(/^expires=.*/i, "expires=<date>")).filter((p) => !/^max-age/i.test(p));
  return `${kind}[${deleted}] ${rest.join("; ").toLowerCase()}`;
}

function show(what, r) {
  let location = r.headers.get("location") ?? "";
  if (/[?&]state=/.test(location) && location.includes("redirect_uri")) {
    const u = new URL(location);
    const keep = ["client_id", "response_type", "scope", "code_challenge_method", "response_mode", "redirect_uri", "max_age", "prompt"];
    const shown = keep.filter((k) => u.searchParams.has(k)).map((k) => `${k}=${u.searchParams.get(k)}`);
    const names = [...u.searchParams.keys()].filter((k) => !keep.includes(k) && !k.startsWith("x-client")).sort();
    location = `${u.origin}${u.pathname} ${shown.join(" ")} +${names.join(",")}`;
  }
  const cookies = [...new Set(r.cookies.map(cookieShape))].sort().join(" | ");
  const cache = ["cache-control", "pragma"].map((h) => r.headers.get(h) ?? "-").join("/");
  const text = r.text.length > 300 ? `${r.text.slice(0, 300)}...(${r.text.length})` : r.text;
  log(`${what} -> ${r.status} loc=[${blank(location)}] cache=${cache} cookies=[${blank(cookies)}] body=${blank(text)}`);
}

const ctl = (pid, change) => fetch(`http://127.0.0.1:${control}/${pid}`, { method: "POST", body: JSON.stringify(change) });
const seen = async (pid) => (await (await fetch(`http://127.0.0.1:${control}/${pid}/seen`)).json()).join(",");
const enc = encodeURIComponent;

async function begin(b, pid, extra = "", route = `/api/auth/oidc/${pid}/login`) {
  const r = await b.get(`${route}${extra}`);
  show(`GET ${route.replace(pid, "<pid>")}${extra.slice(0, 60)}`, r);
  const loc = r.headers.get("location");
  if (r.status !== 302 || !loc || !loc.startsWith("https://")) return null;
  const q = new URL(loc).searchParams;
  await ctl(pid, { nonce: q.get("nonce") });
  return { state: q.get("state"), url: loc };
}
async function finish(b, pid, started, how = "get", extra = "") {
  const route = `/api/auth/oidc/${pid}/callback`;
  const code = `code=the-code&state=${enc(started.state)}${extra}`;
  const r = how === "get" ? await b.get(`${route}?${code}`) : await b.send("POST", route, { form: code });
  show(`${how.toUpperCase()} callback <pid>`, r);
  return r;
}
const session = async (b) => { const r = await b.get("/api/session"); log(`session -> ${r.status} user=${blank(JSON.stringify(JSON.parse(r.text).user?.username ?? null))}`); };

let owner;
async function ownerBrowser() {
  const b = new Browser();
  const r = await b.post("/api/auth/login", { username: "owner", password: "Passw0rd!2026" });
  if (r.status !== 200) throw new Error(`owner login ${r.status} ${r.text}`);
  return b;
}
async function signInAs(pid, returnUrl = "/", b = new Browser(), pre) {
  if (pre) await ctl(pid, pre);
  const s = await begin(b, pid, `?returnUrl=${enc(returnUrl)}`);
  if (!s) return null;
  await finish(b, pid, s);
  return b;
}

async function flows() {
  // Founding.
  const reg = await new Browser().post("/api/auth/register", { username: "owner", displayName: "Owner", email: "owner@example.test", password: "Passw0rd!2026", setupCode });
  log(`founding -> ${reg.status}`);
  owner = await ownerBrowser();

  log("== nothing known / status");
  show("GET /api/auth/oidc", await new Browser().get("/api/auth/oidc"));
  show("GET login unknown provider", await new Browser().get("/api/auth/oidc/nope/login"));
  show("GET callback unknown provider", await new Browser().get("/api/auth/oidc/nope/callback"));
  show("GET callback default", await new Browser().get("/api/auth/oidc/callback"));
  show("GET login default/login", await new Browser().get("/api/auth/oidc/default/login"));
  show("GET login keepSignedIn=maybe", await new Browser().get("/api/auth/oidc/p1/login?keepSignedIn=maybe"));
  show("GET link no ticket", await new Browser().get("/api/auth/oidc/p1/link"));
  show("GET pending none", await new Browser().get("/api/auth/oidc/pending"));
  show("DELETE pending none", await new Browser().send("DELETE", "/api/auth/oidc/pending"));
  show("PUT callback", await new Browser().send("PUT", "/api/auth/oidc/p1/callback"));
  show("HEAD callback", await new Browser().send("HEAD", "/api/auth/oidc/p1/callback"));

  log("== unlinked identity, read, dropped");
  await ctl("p1", { subject: "s-p1", claims: { email: "Ola@Example.test", email_verified: true, name: "Ola Nowak" } });
  let b = await signInAs("p1", "/invite/x?y=1");
  show("GET pending", await b.get("/api/auth/oidc/pending"));
  await session(b);
  show("GET pending other browser", await new Browser().get("/api/auth/oidc/pending"));
  show("DELETE pending", await b.send("DELETE", "/api/auth/oidc/pending"));
  show("GET pending after drop", await b.get("/api/auth/oidc/pending"));

  log("== link by password sign-in, then sign in, and every return address");
  await ctl("p2", { subject: "s-p2" });
  b = await signInAs("p2");
  const lr = await b.post("/api/auth/login", { username: "owner", password: "Passw0rd!2026", linkPending: true });
  log(`login linkPending -> ${lr.status} ${blank(lr.text.replace(/"user":\{.*?\},/, ""))} cookies=[${blank([...new Set(lr.cookies.map(cookieShape))].join(" | "))}]`);
  const hostile = ["/", "/invite/abc?source=oidc", "https://evil.example/", "//evil.example/", "/\\evil.example/", "/.//evil.example", "/%2e//evil.example",
    "/a/..//evil.example", "/%2F%2Fevil.example", "/invite/v1.2", "javascript:alert(1)", "/a\u0001b", "/ż", "/%C5%BC", "/a b", "/a%20b", "/?x=//y",
    "/#//evil", "http://evil", "///evil", "/\\/evil", "/%5Cevil", "/%0d%0aSet-Cookie:x=y", " /padded ", "/%25%32%65%25%32%65//evil", "/./a", "/a/./b", "/..", "", "/x".repeat(1500)];
  for (const url of hostile) {
    const c = await signInAs("p2", url);
    if (c) await session(c);
  }
  const keep = new Browser();
  const s = await begin(keep, "p2", "?keepSignedIn=false&returnUrl=%2F");
  await finish(keep, "p2", s);

  log("== refusals and a second answer");
  await ctl("p3", { subject: "s-p3" });
  b = new Browser();
  let st = await begin(b, "p3", "?returnUrl=%2Fx");
  show("GET callback error=access_denied", await b.get(`/api/auth/oidc/p3/callback?error=access_denied&state=${enc(st.state)}`));
  st = await begin(b, "p3");
  show("GET callback error=server_error", await b.get(`/api/auth/oidc/p3/callback?error=server_error&state=${enc(st.state)}`));
  st = await begin(b, "p3");
  show("POST callback error=access_denied", await b.send("POST", "/api/auth/oidc/p3/callback", { form: `error=access_denied&state=${enc(st.state)}` }));
  st = await begin(b, "p3");
  show("GET callback no code", await b.get(`/api/auth/oidc/p3/callback?state=${enc(st.state)}`));
  st = await begin(b, "p3");
  show("GET callback other browser", await new Browser().get(`/api/auth/oidc/p3/callback?code=the-code&state=${enc(st.state)}`));
  st = await begin(b, "p3");
  await finish(b, "p3", st);
  show("GET callback replayed", await b.get(`/api/auth/oidc/p3/callback?code=the-code&state=${enc(st.state)}`));
  show("GET callback garbage state", await b.get("/api/auth/oidc/p3/callback?code=the-code&state=garbage"));
  show("GET callback no state", await b.get("/api/auth/oidc/p3/callback?code=the-code"));
  show("POST callback json", await b.send("POST", "/api/auth/oidc/p3/callback", { json: { code: "x" } }));
  st = await begin(b, "p3");
  show("GET callback state of p2", await b.get(`/api/auth/oidc/p2/callback?code=the-code&state=${enc(st.state)}`));

  log("== linking from the security page");
  await ctl("p4", { subject: "s-p4" });
  const w = await ownerBrowser();
  show("POST link unknown provider", await w.post("/api/me/security/identities/nope/link", {}));
  show("POST link no auth", await new Browser().post("/api/me/security/identities/p4/link", {}));
  const ticket = await w.post("/api/me/security/identities/p4/link", {});
  show("POST link p4", ticket);
  const url = JSON.parse(ticket.text).url;
  show("GET link ticket in another browser", await new Browser().get(url));
  show("GET link bad ticket", await w.get("/api/auth/oidc/p4/link?ticket=junk"));
  show("GET link ticket for another provider", await w.get(url.replace("/p4/", "/p5/")));
  const started = await (async () => { const r = await w.get(url); show("GET link ticket", r); const q = new URL(r.headers.get("location")).searchParams; await ctl("p4", { nonce: q.get("nonce") }); return { state: q.get("state") }; })();
  await finish(new Browser(), "p4", started);
  const again = await (async () => { const r = await w.get(url); const q = new URL(r.headers.get("location")).searchParams; await ctl("p4", { nonce: q.get("nonce") }); return { state: q.get("state") }; })();
  await finish(w, "p4", again);
  const page = await w.get("/api/me/security");
  let ids = [];
  try { ids = JSON.parse(page.text).identities ?? []; } catch { log("the security page is not on this server"); }
  log(`security identities=${blank(JSON.stringify(ids.map((i) => i.name)))}`);
  show("GET link when already linked", await (async () => { const t = await w.post("/api/me/security/identities/p4/link", {}); const r = await w.get(JSON.parse(t.text).url); const q = new URL(r.headers.get("location")).searchParams; await ctl("p4", { nonce: q.get("nonce") }); return await w.get(`/api/auth/oidc/p4/callback?code=the-code&state=${enc(q.get("state"))}`); })());
  const ended = await ownerBrowser();
  const t2 = await ended.post("/api/me/security/identities/p6/link", {});
  await ctl("p6", { subject: "s-p6" });
  const r2 = await ended.get(JSON.parse(t2.text).url);
  const q2 = new URL(r2.headers.get("location")).searchParams;
  await ctl("p6", { nonce: q2.get("nonce") });
  show("POST logout (session ended)", await ended.post("/api/auth/logout"));
  show("GET callback after session ended", await new Browser().get(`/api/auth/oidc/p6/callback?code=the-code&state=${enc(q2.get("state"))}`));
  show("GET callback after session ended, same browser", await ended.get(`/api/auth/oidc/p6/callback?code=the-code&state=${enc(q2.get("state"))}`));
  if (ids[0]) {
    show("POST remove unknown", await w.post("/api/me/security/identities/00000000-0000-0000-0000-000000000000/remove", {}));
    show("POST remove not-a-guid", await w.post("/api/me/security/identities/xyz/remove", {}));
    show("POST remove", await w.post(`/api/me/security/identities/${ids[0].id}/remove`, {}));
    show("POST remove again", await w.post(`/api/me/security/identities/${ids[0].id}/remove`, {}));
  }
  show("POST remove no auth", await new Browser().post("/api/me/security/identities/00000000-0000-0000-0000-000000000000/remove", {}));

  log("== a fresh sign-in at the provider as proof");
  const proofs = new Browser();
  const login2 = await proofs.post("/api/auth/login", { username: "owner", password: "Passw0rd!2026" });
  log(`owner login -> ${login2.status}`);
  show("POST reauthenticate no auth", await new Browser().post("/api/me/security/identities/p4/reauthenticate"));
  show("POST reauthenticate unlinked provider", await proofs.post("/api/me/security/identities/p3/reauthenticate"));
  show("POST reauthenticate unknown provider", await proofs.post("/api/me/security/identities/nope/reauthenticate"));
  const nowSecs = Math.floor(Date.now() / 1000);
  for (const [what, change, back] of [
    ["fresh", { subject: "s-p4", authTime: nowSecs - 30 }, "/settings?tab=security"],
    ["three minutes old", { subject: "s-p4", authTime: nowSecs - 180 }, "/"],
    ["too old", { subject: "s-p4", authTime: nowSecs - 3600 }, "https://evil.example/"],
    ["no auth_time", { subject: "s-p4", authTime: null }, "/"],
    ["somebody else", { subject: "s-other", authTime: nowSecs }, "/"],
  ]) {
    log(`-- ${what}`);
    await ctl("p4", change);
    const t = await proofs.post("/api/me/security/identities/p4/reauthenticate");
    show("POST reauthenticate p4", t);
    const addr = JSON.parse(t.text).url + `&returnUrl=${enc(back)}`;
    const r = await proofs.get(addr);
    show("GET reauthenticate", r);
    const q = new URL(r.headers.get("location")).searchParams;
    await ctl("p4", { nonce: q.get("nonce") });
    await proofs.get(`/api/auth/oidc/p4/callback?code=the-code&state=${enc(q.get("state"))}`).then((x) => show("GET callback", x));
  }
  const t3 = await proofs.post("/api/me/security/identities/p4/reauthenticate");
  const link3 = JSON.parse(t3.text).url;
  show("GET reauthenticate in another browser", await new Browser().get(link3));
  show("GET link with a reauthenticate ticket", await proofs.get(link3.replace("/reauthenticate?", "/link?")));
  show("GET reauthenticate bad ticket", await proofs.get("/api/auth/oidc/p4/reauthenticate?ticket=junk&returnUrl=%2Fa"));
  const t4 = await proofs.post("/api/me/security/identities/p4/link", {});
  show("GET reauthenticate with a link ticket", await proofs.get(JSON.parse(t4.text).url.replace("/link?", "/reauthenticate?")));

  log("== a provider that misbehaves: an unlinked sign-in ends on the page that says so, or fails");
  const bad = [
    ["valid", {}], ["stranger key", { signing: "stranger" }], ["unknown kid", { signing: "strangerKid" }], ["none", { signing: "none" }],
    ["HS256", { alg: "HS256" }], ["expired", { claims: { exp: 1 } }], ["not yet valid", { claims: { nbf: 4102444800 } }],
    ["audience", { claims: { aud: "x" } }], ["issuer", { claims: { iss: "https://evil.test" } }], ["no nonce", { claims: { nonce: null } }],
    ["wrong nonce", { claims: { nonce: "no" } }], ["no iat", { claims: { iat: null } }], ["no exp", { claims: { exp: null } }],
    ["huge subject", { claims: { sub: "x".repeat(10000) } }], ["subject 256", { claims: { sub: "y".repeat(256) } }], ["subject 255", { claims: { sub: "z".repeat(255) } }],
    ["no subject", { claims: { sub: null } }], ["no access token", { noAccessToken: true }], ["token 500", { instead: { "/token": { status: 500 } } }],
    ["token not json", { instead: { "/token": { body: "nope" } } }], ["token error", { instead: { "/token": { status: 400, body: "{\"error\":\"invalid_grant\"}" } } }],
    ["email false", { claims: { email: "a@b.test", email_verified: false } }], ["azp other", { claims: { azp: "other" } }],
    ["userinfo ok", { userinfo: { email: "u@b.test", email_verified: true } }], ["userinfo 500", { userinfo: {}, instead: { "/userinfo": { status: 500 } } }],
    ["userinfo other sub", { userinfo: {}, instead: { "/userinfo": { body: "{\"sub\":\"other\"}", status: 200 } } }],
    ["userinfo not json", { userinfo: {}, instead: { "/userinfo": { body: "x", status: 200 } } }],
  ];
  let n = 10;
  for (const [what, change] of bad) {
    const pid = `p${n++}`;
    log(`-- ${what}`);
    const c = await signInAs(pid, "/", new Browser(), { subject: `s-${pid}`, ...change });
    if (c) show("GET pending", await c.get("/api/auth/oidc/pending"));
  }

  log("== discovery");
  const disc = [
    ["discovery 500", { instead: { "/.well-known/openid-configuration": { status: 500 } } }],
    ["discovery not json", { instead: { "/.well-known/openid-configuration": { body: "nope" } } }],
    ["discovery slow", { instead: { "/.well-known/openid-configuration": { delayMs: 2500 } } }],
    ["discovery moved", { instead: { "/.well-known/openid-configuration": { status: 302, location: "@/p39/.well-known/openid-configuration" } } }],
    ["discovery names another issuer", { issuerInDiscovery: "https://other.test" }],
    ["jwks 500", { instead: { "/jwks": { status: 500 } } }],
    ["jwks empty", { instead: { "/jwks": { body: "{\"keys\":[]}" } } }],
  ];
  for (const [what, change] of disc) {
    const pid = `p${n++}`;
    log(`-- ${what}`);
    await signInAs(pid, "/", new Browser(), { subject: `s-${pid}`, ...change });
  }
  {
    // Each server has a provider of its own, whose keys the other has not seen.
    const pid = label === "first" ? "p44" : "p45";
    log("-- keys change between two sign-ins");
    await signInAs(pid, "/", new Browser(), { subject: `s-${pid}` });
    await ctl(pid, { rotate: true });
    await signInAs(pid, "/", new Browser());
    await ctl(pid, { rotate: true });
    await signInAs(pid, "/", new Browser());
    log(`asked: ${await seen(pid)}`);
  }
}

async function limits() {
  await new Browser().post("/api/auth/register", { username: "owner", displayName: "Owner", email: "owner@example.test", password: "Passw0rd!2026", setupCode });
  const w = await ownerBrowser();
  log("== rate limits: 12 sign-in starts, 7 pending drops, 12 pending reads, 35 link starts");
  const run = async (what, count, f) => {
    const codes = [];
    for (let i = 0; i < count; i++) codes.push((await f()).status);
    log(`${what}: ${codes.join(",")}`);
  };
  const b = new Browser();
  await run("login", 12, () => b.get("/api/auth/oidc/p1/login"));
  await run("link", 12, () => b.get("/api/auth/oidc/p1/link"));
  await run("pending read", 12, () => b.get("/api/auth/oidc/pending"));
  await run("pending drop", 7, () => b.send("DELETE", "/api/auth/oidc/pending"));
  const poll = new Browser();
  await run("pending read, 125 in a minute", 125, () => poll.get("/api/auth/oidc/pending"));
  await run("security link, 32", 32, () => w.post("/api/me/security/identities/p1/link", {}));
  await run("security reauthenticate, 3 more", 3, () => w.post("/api/me/security/identities/p1/reauthenticate"));
}

try {
  await (pass === "limits" ? limits() : flows());
} catch (e) {
  log(`DRIVER ERROR ${e.stack}`);
}
console.log(out.join("\n"));
