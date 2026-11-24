// A provider both servers are pointed at: HTTPS on <port>, scripted over plain
// HTTP on 127.0.0.1:<control>. One provider per path prefix (/p1, /p2, ...), so
// that each scenario has discovery and keys of its own, which the servers keep.
//
//   node fake-provider.mjs <host> <port> <control-port> <cert.pem> <key.pem>
//
// POST /<prefix> with a JSON object changes that provider's script (see
// `fresh()` for the fields); GET /<prefix>/seen lists what it was asked.
import https from "node:https";
import http from "node:http";
import fs from "node:fs";
import crypto from "node:crypto";

const [host, port, control, certFile, keyFile] = process.argv.slice(2);
const providers = new Map();
const b64 = (b) => Buffer.from(b).toString("base64url");
const newKey = () => crypto.generateKeyPairSync("ec", { namedCurve: "P-256" });

function fresh(prefix) {
  return {
    prefix, subject: "subject", nonce: "", amr: null, authTime: null, claims: {}, userinfo: null,
    signing: "published", alg: "ES256", issuerInDiscovery: null, instead: {}, noAccessToken: false,
    key: newKey(), kid: "k1", seen: [],
  };
}
const get = (prefix) => { if (!providers.has(prefix)) providers.set(prefix, fresh(prefix)); return providers.get(prefix); };
const issuerOf = (p) => `https://${host}:${port}${p.prefix}`;

function jwk(p) {
  const j = p.key.publicKey.export({ format: "jwk" });
  return { kty: "EC", crv: "P-256", alg: "ES256", use: "sig", kid: p.kid, x: j.x, y: j.y };
}

function idToken(p) {
  const now = Math.floor(Date.now() / 1000);
  const claims = { iss: issuerOf(p), aud: "stellarr-tests", sub: p.subject, nonce: p.nonce, iat: now, exp: now + 300 };
  if (p.amr) claims.amr = p.amr;
  if (p.authTime !== null) claims.auth_time = p.authTime;
  for (const [k, v] of Object.entries(p.claims)) { if (v === null) delete claims[k]; else claims[k] = v; }
  const alg = p.signing === "none" ? "none" : p.alg;
  const kid = p.signing === "strangerKid" ? "other" : p.kid;
  const head = b64(JSON.stringify({ alg, typ: "JWT", kid }));
  const input = `${head}.${b64(JSON.stringify(claims))}`;
  if (p.signing === "none") return `${input}.`;
  const key = p.signing === "published" ? p.key.privateKey : newKey().privateKey;
  const sig = crypto.sign("sha256", Buffer.from(input), { key, dsaEncoding: "ieee-p1363" });
  return `${input}.${b64(sig)}`;
}

function answer(p, method, path, form, auth) {
  if (method === "GET" && path === "/.well-known/openid-configuration") {
    const doc = { issuer: p.issuerInDiscovery ?? issuerOf(p), authorization_endpoint: `${issuerOf(p)}/authorize`,
      token_endpoint: `${issuerOf(p)}/token`, jwks_uri: `${issuerOf(p)}/jwks` };
    if (p.userinfo) doc.userinfo_endpoint = `${issuerOf(p)}/userinfo`;
    return [200, JSON.stringify(doc)];
  }
  if (method === "GET" && path === "/jwks") return [200, JSON.stringify({ keys: [jwk(p)] })];
  if (method === "POST" && path === "/token") {
    if (!form.includes("client_secret=test-client-secret") || !form.includes("code=the-code"))
      return [400, JSON.stringify({ error: "invalid_grant" })];
    const out = { id_token: idToken(p), token_type: "Bearer", expires_in: 300 };
    if (!p.noAccessToken) out.access_token = "access-token";
    return [200, JSON.stringify(out)];
  }
  if (method === "GET" && path === "/userinfo") {
    if (p.userinfo && auth === "Bearer access-token") return [200, JSON.stringify({ ...p.userinfo, sub: p.subject })];
    return [401, ""];
  }
  return [404, ""];
}

https.createServer({ key: fs.readFileSync(keyFile), cert: fs.readFileSync(certFile) }, (req, res) => {
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", async () => {
    const m = /^(\/p\d+)(\/[^?]*)/.exec(req.url);
    if (!m) { res.writeHead(404).end(); return; }
    const p = get(m[1]);
    const form = Buffer.concat(chunks).toString();
    const auth = req.headers.authorization ?? "";
    p.seen.push(`${req.method} ${m[2]}`);
    let [status, body] = answer(p, req.method, m[2], form, auth);
    const over = p.instead[m[2]];
    const headers = { "content-type": "application/json" };
    if (over) {
      if (over.delayMs) await new Promise((r) => setTimeout(r, over.delayMs));
      status = over.status ?? status; body = over.body ?? body;
      if (over.location) headers.location = over.location.replace("@", `https://${host}:${port}`);
    }
    res.writeHead(status, headers).end(body);
  });
}).listen(Number(port), "0.0.0.0");

http.createServer((req, res) => {
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => {
    const m = /^(\/p\d+)(\/seen)?$/.exec(req.url);
    if (!m) { res.writeHead(404).end(); return; }
    const p = get(m[1]);
    if (m[2]) { res.end(JSON.stringify(p.seen)); return; }
    const change = JSON.parse(Buffer.concat(chunks).toString() || "{}");
    if (change.rotate) { p.key = newKey(); p.kid = `k${Date.now()}`; delete change.rotate; }
    Object.assign(p, change);
    res.end("{}");
  });
}).listen(Number(control), "127.0.0.1");
