// Writes node_fixtures.json: a registration and an assertion for every
// algorithm the crate offers, made with Node's crypto (OpenSSL) and with CBOR
// encoded by the helpers below, so that nothing in them was produced by the
// crate under test or by aws-lc-rs.
//
//   node generate_node.mjs > node_fixtures.json
import crypto from "node:crypto";

const b64u = (b) => Buffer.from(b).toString("base64url");
const sha256 = (b) => crypto.createHash("sha256").update(b).digest();

// A tiny CBOR writer: unsigned and negative integers, byte and text strings, maps.
function head(major, n) {
  if (n < 24) return Buffer.from([(major << 5) | n]);
  if (n < 256) return Buffer.from([(major << 5) | 24, n]);
  const b = Buffer.alloc(3);
  b[0] = (major << 5) | 25;
  b.writeUInt16BE(n, 1);
  return b;
}
const int = (n) => (n >= 0 ? head(0, n) : head(1, -1 - n));
const bytes = (b) => Buffer.concat([head(2, b.length), b]);
const text = (s) => Buffer.concat([head(3, Buffer.byteLength(s)), Buffer.from(s)]);
const map = (pairs) => Buffer.concat([head(5, pairs.length), ...pairs.flat()]);

const RP_ID = "chat.test";
const ORIGIN = "https://chat.test";
const USER_HANDLE = Buffer.from("00112233445566778899aabbccddeeff", "hex");

const suites = {
  EdDSA: { cose: -8, make: () => crypto.generateKeyPairSync("ed25519"), sign: (k, d) => crypto.sign(null, d, k) },
  ES256: { cose: -7, make: () => crypto.generateKeyPairSync("ec", { namedCurve: "P-256" }), sign: (k, d) => crypto.sign("sha256", d, k), crv: 1 },
  ES384: { cose: -35, make: () => crypto.generateKeyPairSync("ec", { namedCurve: "P-384" }), sign: (k, d) => crypto.sign("sha384", d, k), crv: 2 },
  ES512: { cose: -36, make: () => crypto.generateKeyPairSync("ec", { namedCurve: "P-521" }), sign: (k, d) => crypto.sign("sha512", d, k), crv: 3 },
  RS256: { cose: -257, make: rsa, sign: (k, d) => crypto.sign("sha256", d, k) },
  RS384: { cose: -258, make: rsa, sign: (k, d) => crypto.sign("sha384", d, k) },
  RS512: { cose: -259, make: rsa, sign: (k, d) => crypto.sign("sha512", d, k) },
  PS256: { cose: -37, make: rsa, sign: (k, d) => pss(k, d, "sha256", 32) },
  PS384: { cose: -38, make: rsa, sign: (k, d) => pss(k, d, "sha384", 48) },
  PS512: { cose: -39, make: rsa, sign: (k, d) => pss(k, d, "sha512", 64) },
};

function rsa() {
  return crypto.generateKeyPairSync("rsa", { modulusLength: 2048 });
}
function pss(key, data, hash, salt) {
  return crypto.sign(hash, data, { key, padding: crypto.constants.RSA_PKCS1_PSS_PADDING, saltLength: salt });
}

function coseKey(name, suite, publicKey) {
  const jwk = publicKey.export({ format: "jwk" });
  const d = (s) => Buffer.from(s, "base64url");
  if (name === "EdDSA") {
    return map([[int(1), int(1)], [int(3), int(-8)], [int(-1), int(6)], [int(-2), bytes(d(jwk.x))]]);
  }
  if (jwk.kty === "EC") {
    return map([[int(1), int(2)], [int(3), int(suite.cose)], [int(-1), int(suite.crv)], [int(-2), bytes(d(jwk.x))], [int(-3), bytes(d(jwk.y))]]);
  }
  return map([[int(1), int(3)], [int(3), int(suite.cose)], [int(-1), bytes(d(jwk.n))], [int(-2), bytes(d(jwk.e))]]);
}

function clientData(type, challenge) {
  return Buffer.from(JSON.stringify({ type, challenge: b64u(challenge), origin: ORIGIN, crossOrigin: false }));
}

const out = [];
for (const [name, suite] of Object.entries(suites)) {
  const { publicKey, privateKey } = suite.make();
  const credentialId = crypto.randomBytes(32);
  const cose = coseKey(name, suite, publicKey);

  const regChallenge = crypto.randomBytes(32);
  const attested = Buffer.concat([Buffer.alloc(16, 0x11), Buffer.from([0, credentialId.length]), credentialId, cose]);
  const regAuth = Buffer.concat([sha256(RP_ID), Buffer.from([0x45 | 0x08 | 0x10 /* UP UV AT BE BS */]), Buffer.from([0, 0, 0, 3]), attested]);
  const regClient = clientData("webauthn.create", regChallenge);
  const regSig = suite.sign(privateKey, Buffer.concat([regAuth, sha256(regClient)]));
  const attestation = map([
    [text("fmt"), text("packed")],
    [text("attStmt"), map([[text("alg"), int(suite.cose)], [text("sig"), bytes(regSig)]])],
    [text("authData"), bytes(regAuth)],
  ]);

  const asChallenge = crypto.randomBytes(32);
  const asAuth = Buffer.concat([sha256(RP_ID), Buffer.from([0x01 | 0x04 | 0x08 | 0x10]), Buffer.from([0, 0, 0, 5])]);
  const asClient = clientData("webauthn.get", asChallenge);
  const asSig = suite.sign(privateKey, Buffer.concat([asAuth, sha256(asClient)]));

  out.push({
    algorithm: name,
    cose_alg: suite.cose,
    rp_id: RP_ID,
    origin: ORIGIN,
    registration_challenge: b64u(regChallenge),
    registration: {
      id: b64u(credentialId),
      rawId: b64u(credentialId),
      type: "public-key",
      clientExtensionResults: {},
      response: {
        attestationObject: b64u(attestation),
        clientDataJSON: b64u(regClient),
        transports: ["internal", "hybrid"],
      },
    },
    stored: { public_key: b64u(cose), user_handle: b64u(USER_HANDLE), sign_count: 4, backup_eligible: true },
    assertion_challenge: b64u(asChallenge),
    assertion: {
      id: b64u(credentialId),
      rawId: b64u(credentialId),
      type: "public-key",
      clientExtensionResults: {},
      response: {
        authenticatorData: b64u(asAuth),
        clientDataJSON: b64u(asClient),
        signature: b64u(asSig),
        userHandle: b64u(USER_HANDLE),
      },
    },
  });
}
console.log(JSON.stringify(out, null, 1));
