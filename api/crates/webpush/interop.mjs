// An independent check of what the crate produces. This script plays the
// browser: it makes its own subscription key pair and authentication secret,
// asks the crate for a push request addressed to them, then decrypts the body
// and verifies the VAPID token using only Node's own crypto. Nothing from the
// crate is trusted except the bytes it prints.
//
// Run by hand:
//
//   node api/crates/webpush/interop.mjs
//
// It starts the crate's `emit_request` example itself, with `cargo` unless the
// ACARGO environment variable names another command (a build wrapper, for
// instance); the command runs inside the `api/` directory. The exit status is
// 0 only if every check passes.

import { execFileSync } from "node:child_process";
import {
  createDecipheriv,
  createECDH,
  createPublicKey,
  createVerify,
  hkdfSync,
  randomBytes,
} from "node:crypto";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const apiDir = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const b64 = (buffer) => Buffer.from(buffer).toString("base64url");
const unb64 = (text) => Buffer.from(text, "base64url");

// The browser's side of the subscription.
const browser = createECDH("prime256v1");
browser.generateKeys();
const uaPublic = browser.getPublicKey(); // uncompressed, 65 bytes
const authSecret = randomBytes(16);

const emitted = execFileSync(
  process.env.ACARGO || "cargo",
  [
    "run", "--quiet", "-p", "stellarr-webpush", "--example", "emit_request",
    "--", b64(uaPublic), b64(authSecret),
  ],
  { cwd: apiDir, encoding: "utf8", maxBuffer: 1 << 20 },
);
const request = JSON.parse(emitted.trim().split("\n").pop());

const header = (name) => {
  const hit = request.headers.find(([key]) => key.toLowerCase() === name.toLowerCase());
  return hit && hit[1];
};

// The request description.
assert.equal(request.method, "POST");
assert.equal(request.url, "https://push.example.test/send/subscription");
assert.equal(header("TTL"), "300");
assert.equal(header("Content-Encoding"), "aes128gcm");
assert.equal(header("Content-Type"), "application/octet-stream");
const body = unb64(request.body);
assert.equal(header("Content-Length"), undefined, "the HTTP client sets the length");

// RFC 8188 header: salt(16) | record size(4) | key id length(1) | key id.
const salt = body.subarray(0, 16);
const recordSize = body.readUInt32BE(16);
const idLength = body[20];
assert.equal(recordSize, 4096);
assert.equal(idLength, 65);
const asPublic = body.subarray(21, 21 + idLength);
const record = body.subarray(21 + idLength);
assert.ok(record.length <= recordSize, "one record fits its declared size");

// RFC 8291 section 3.4, written out from the RFC with Node's primitives.
const ecdhSecret = browser.computeSecret(asPublic);
const keyInfo = Buffer.concat([Buffer.from("WebPush: info\0"), uaPublic, asPublic]);
const ikm = Buffer.from(hkdfSync("sha256", ecdhSecret, authSecret, keyInfo, 32));
const cek = Buffer.from(hkdfSync("sha256", ikm, salt, Buffer.from("Content-Encoding: aes128gcm\0"), 16));
const nonce = Buffer.from(hkdfSync("sha256", ikm, salt, Buffer.from("Content-Encoding: nonce\0"), 12));

function decrypt(ciphertext) {
  const decipher = createDecipheriv("aes-128-gcm", cek, nonce);
  decipher.setAuthTag(ciphertext.subarray(ciphertext.length - 16));
  return Buffer.concat([
    decipher.update(ciphertext.subarray(0, ciphertext.length - 16)),
    decipher.final(),
  ]);
}

const padded = decrypt(record);
// RFC 8188: strip trailing zeros up to the delimiter; the last record ends in 0x02.
let end = padded.length;
while (end > 0 && padded[end - 1] === 0) end -= 1;
assert.equal(padded[end - 1], 0x02, "last record ends with the 0x02 delimiter");
const plaintext = padded.subarray(0, end - 1).toString("utf8");
assert.deepEqual(JSON.parse(plaintext), request.payload);
assert.equal(JSON.parse(plaintext).title, "Zażółć gęślą jaźń \u{1F680}");

// The tag must actually protect the record: any flipped bit fails.
for (const index of [0, record.length - 1, Math.floor(record.length / 2)]) {
  const tampered = Buffer.from(record);
  tampered[index] ^= 1;
  assert.throws(() => decrypt(tampered), "a tampered record must not decrypt");
}

// RFC 8292: `vapid t=<jwt>, k=<key>`; the key signs the JWT (ES256, r||s).
const authorization = header("Authorization");
const match = /^vapid t=([\w-]+)\.([\w-]+)\.([\w-]+), k=([\w-]+)$/.exec(authorization);
assert.ok(match, `Authorization has the vapid shape: ${authorization}`);
const [, h, p, s, k] = match;
assert.equal(k, request.vapidPublicKey);
assert.deepEqual(JSON.parse(unb64(h)), { typ: "JWT", alg: "ES256" });
const claims = JSON.parse(unb64(p));
assert.deepEqual(Object.keys(claims), ["aud", "exp", "sub"]);
assert.equal(claims.aud, "https://push.example.test");
assert.equal(claims.sub, "mailto:push@example.test");
assert.equal(claims.exp, request.now + 12 * 3600);

const vapidKey = unb64(k);
assert.equal(vapidKey.length, 65);
assert.equal(vapidKey[0], 4);
const publicKey = createPublicKey({
  key: { kty: "EC", crv: "P-256", x: b64(vapidKey.subarray(1, 33)), y: b64(vapidKey.subarray(33)) },
  format: "jwk",
});
const signature = unb64(s);
assert.equal(signature.length, 64);
const verifier = createVerify("SHA256").update(`${h}.${p}`);
assert.ok(
  verifier.verify({ key: publicKey, dsaEncoding: "ieee-p1363" }, signature),
  "the VAPID signature verifies",
);
const forged = createVerify("SHA256").update(`${h}.${p}x`);
assert.ok(!forged.verify({ key: publicKey, dsaEncoding: "ieee-p1363" }, signature), "and only for this token");

console.log("ok: decrypted and verified independently");
console.log(`  record ${record.length} bytes, plaintext ${plaintext.length} bytes`);
console.log(`  token valid until ${new Date(claims.exp * 1000).toISOString()}`);
