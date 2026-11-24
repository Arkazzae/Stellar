// Makes fixtures.json: keys, key sets and ID tokens signed with Node's crypto,
// never with the code under test. Run: node generate.mjs > fixtures.json
// Every token differs from `base` in the fields its name says and is signed
// properly, so a test that expects a refusal fails only when its own check goes.
import crypto from "node:crypto";

const NOW = 1_800_000_000;
const ISS = "https://identity.test";
const AUD = "stellarr-tests";
const ACCESS_TOKEN = "access-token-value";

const b64u = (b) => Buffer.from(b).toString("base64url");
const json = (o) => b64u(JSON.stringify(o));

const rsa = (bits = 2048) => crypto.generateKeyPairSync("rsa", { modulusLength: bits });
const ec = (curve) => crypto.generateKeyPairSync("ec", { namedCurve: curve });

const rsa1 = rsa();
const rsa2 = rsa();
const weak = rsa(1024);
const p256 = ec("P-256");
const p384 = ec("P-384");
const p521 = ec("P-521");

const jwk = (pair, kid, extra = {}) => ({
  ...pair.publicKey.export({ format: "jwk" }),
  kid,
  ...extra,
});

const spec = {
  RS256: { hash: "sha256" },
  RS384: { hash: "sha384" },
  RS512: { hash: "sha512" },
  PS256: { hash: "sha256", pss: 32 },
  PS384: { hash: "sha384", pss: 48 },
  PS512: { hash: "sha512", pss: 64 },
  ES256: { hash: "sha256", ieee: true },
  ES384: { hash: "sha384", ieee: true },
  ES512: { hash: "sha512", ieee: true },
};

function sign(alg, pair, header, claims) {
  const input = `${json(header)}.${json(claims)}`;
  const s = spec[alg];
  const opts = { key: pair.privateKey };
  if (s.pss) {
    opts.padding = crypto.constants.RSA_PKCS1_PSS_PADDING;
    opts.saltLength = s.pss;
  }
  if (s.ieee) opts.dsaEncoding = "ieee-p1363";
  return `${input}.${b64u(crypto.sign(s.hash, Buffer.from(input), opts))}`;
}

const atHashOf = (hash) => {
  const digest = crypto.createHash(hash).update(ACCESS_TOKEN).digest();
  return b64u(digest.subarray(0, digest.length / 2));
};
const atHash = b64u(crypto.createHash("sha256").update(ACCESS_TOKEN).digest().subarray(0, 16));
const base = {
  iss: ISS,
  aud: AUD,
  sub: "subject-1",
  nonce: "n-1",
  iat: NOW - 5,
  exp: NOW + 300,
  auth_time: NOW - 30,
  amr: ["pwd", "mfa"],
  email: "ola@example.test",
  email_verified: true,
  name: "Ola Łęcka",
  at_hash: atHash,
};
const without = (o, ...names) => Object.fromEntries(Object.entries(o).filter(([k]) => !names.includes(k)));
// claims may be any JSON value here
const good = (claims, alg = "RS256", pair = rsa1, kid = "rsa-1") =>
  sign(alg, pair, { alg, typ: "JWT", kid }, claims);

const tokens = {
  rs256: good(base),
  rs384: good(base, "RS384"),
  rs512: good(base, "RS512"),
  ps256: good(base, "PS256"),
  ps384: good(base, "PS384"),
  ps512: good(base, "PS512"),
  es256: good(base, "ES256", p256, "ec-256"),
  es384: good(base, "ES384", p384, "ec-384"),
  es512: good(base, "ES512", p521, "ec-521"),
  // no kid: tried against every key that fits
  no_kid: sign("RS256", rsa1, { alg: "RS256" }, base),
  no_kid_es256: sign("ES256", p256, { alg: "ES256" }, base),

  // the signature
  other_key_same_kid: good(base, "RS256", rsa2, "rsa-1"),
  other_key_own_kid: good(base, "RS256", rsa2, "rsa-2"),
  no_kid_other_key: sign("RS256", rsa2, { alg: "RS256" }, base),
  rsa_alg_on_ec_key: sign("RS256", rsa1, { alg: "RS256", kid: "ec-256" }, base),
  es256_on_rsa_key: (() => {
    const input = `${json({ alg: "ES256", kid: "rsa-1" })}.${json(base)}`;
    return `${input}.${b64u(crypto.sign("sha256", Buffer.from(input), { key: p256.privateKey, dsaEncoding: "ieee-p1363" }))}`;
  })(),
  alg_none: `${json({ alg: "none", typ: "JWT", kid: "rsa-1" })}.${json(base)}.`,
  alg_none_with_signature: `${json({ alg: "none", kid: "rsa-1" })}.${json(base)}.${b64u("x".repeat(256))}`,
  alg_none_upper: `${json({ alg: "NONE", kid: "rsa-1" })}.${json(base)}.`,
  alg_missing: sign("RS256", rsa1, { typ: "JWT", kid: "rsa-1" }, base),
  alg_not_text: sign("RS256", rsa1, { alg: 256, kid: "rsa-1" }, base),
  hs256_with_public_key: (() => {
    const pem = rsa1.publicKey.export({ type: "spki", format: "pem" });
    const input = `${json({ alg: "HS256", kid: "rsa-1" })}.${json(base)}`;
    return `${input}.${b64u(crypto.createHmac("sha256", pem).update(input).digest())}`;
  })(),
  hs256_with_client_secret: (() => {
    const input = `${json({ alg: "HS256", kid: "rsa-1" })}.${json(base)}`;
    return `${input}.${b64u(crypto.createHmac("sha256", "test-client-secret").update(input).digest())}`;
  })(),
  hs256_no_kid: (() => {
    const pem = rsa1.publicKey.export({ type: "spki", format: "pem" });
    const input = `${json({ alg: "HS256" })}.${json(base)}`;
    return `${input}.${b64u(crypto.createHmac("sha256", pem).update(input).digest())}`;
  })(),
  crit_header: sign("RS256", rsa1, { alg: "RS256", kid: "rsa-1", crit: ["exp"], exp: 1 }, base),
  weak_key: sign("RS256", weak, { alg: "RS256", kid: "rsa-weak" }, base),

  // the claims: each is signed by the right key
  wrong_issuer: good({ ...base, iss: "https://evil.test" }),
  issuer_with_slash: good({ ...base, iss: `${ISS}/` }),
  no_issuer: good(without(base, "iss")),
  wrong_audience: good({ ...base, aud: "someone-else" }),
  no_audience: good(without(base, "aud")),
  audience_list: good({ ...base, aud: ["other", AUD], azp: AUD }),
  audience_list_no_azp: good({ ...base, aud: ["other", AUD] }),
  audience_list_wrong_azp: good({ ...base, aud: ["other", AUD], azp: "other" }),
  single_audience_wrong_azp: good({ ...base, azp: "other" }),
  single_audience_right_azp: good({ ...base, azp: AUD }),
  audience_not_text: good({ ...base, aud: 5 }),
  wrong_nonce: good({ ...base, nonce: "n-2" }),
  no_nonce: good(without(base, "nonce")),
  nonce_not_text: good({ ...base, nonce: 1 }),
  expired: good({ ...base, exp: NOW - 301 }),
  expired_edge_ok: good({ ...base, exp: NOW - 300 }),
  no_expiry: good(without(base, "exp")),
  expiry_not_a_number: good({ ...base, exp: String(NOW + 300) }),
  expiry_fraction: good({ ...base, exp: NOW + 300.9 }),
  not_yet_valid: good({ ...base, nbf: NOW + 301, exp: NOW + 1000 }),
  not_yet_valid_edge_ok: good({ ...base, nbf: NOW + 300, exp: NOW + 1000 }),
  not_before_after_expiry: good({ ...base, nbf: NOW + 400, exp: NOW + 100 }),
  no_issued_at: good(without(base, "iat")),
  no_subject: good(without(base, "sub")),
  empty_subject: good({ ...base, sub: "  " }),
  padded_subject: good({ ...base, sub: "  subject-1  " }),
  long_subject_ok: good({ ...base, sub: "s".repeat(255) }),
  long_subject: good({ ...base, sub: "s".repeat(256) }),
  subject_not_text: good({ ...base, sub: 7 }),
  stale_auth_time: good({ ...base, auth_time: NOW - 3600 }),
  auth_time_edge_ok: good({ ...base, auth_time: NOW - 360 }),
  auth_time_just_stale: good({ ...base, auth_time: NOW - 361 }),
  future_auth_time: good({ ...base, auth_time: NOW + 7200 }),
  no_auth_time: good(without(base, "auth_time")),
  auth_time_text: good({ ...base, auth_time: "yesterday" }),
  wrong_at_hash: good({ ...base, at_hash: b64u(Buffer.alloc(16)) }),
  no_at_hash: good(without(base, "at_hash")),

  // a key id nobody has, signed by a key the provider does publish
  unknown_kid_right_key: good(base, "RS256", rsa1, "rotated-name"),
  // a key id that is not text
  kid_number: sign("RS256", rsa1, { alg: "RS256", kid: 5 }, base),
  kid_null: sign("RS256", rsa1, { alg: "RS256", kid: null }, base),
  // an empty azp counts as none
  empty_azp: good({ ...base, azp: "" }),
  audience_list_empty_azp: good({ ...base, aud: ["other", AUD], azp: "" }),
  // time claims of the wrong type
  nbf_text: good({ ...base, nbf: "soon" }),
  iat_text: good({ ...base, iat: "now" }),
  exp_null: good({ ...base, exp: null }),
  auth_time_bool: good({ ...base, auth_time: true }),
  auth_time_digits: good({ ...base, auth_time: String(NOW - 30) }),
  auth_time_null: good({ ...base, auth_time: null }),
  auth_time_negative: good({ ...base, auth_time: -5 }),
  auth_time_fraction: good({ ...base, auth_time: NOW - 30.5 }),
  auth_time_overflow: good({ ...base, auth_time: "99999999999999999999" }),
  auth_time_signed_text: good({ ...base, auth_time: "+" + String(NOW - 30) }),
  // a payload that is signed but not an object
  payload_array: sign("RS256", rsa1, { alg: "RS256", kid: "rsa-1" }, [1]),
  payload_string: sign("RS256", rsa1, { alg: "RS256", kid: "rsa-1" }, "x"),
  // at_hash with the hash of each family, and with the wrong family's hash
  at_rs384: good({ ...base, at_hash: atHashOf("sha384") }, "RS384"),
  at_rs512: good({ ...base, at_hash: atHashOf("sha512") }, "RS512"),
  at_ps512: good({ ...base, at_hash: atHashOf("sha512") }, "PS512"),
  at_es384: good({ ...base, at_hash: atHashOf("sha384") }, "ES384", p384, "ec-384"),
  at_es512: good({ ...base, at_hash: atHashOf("sha512") }, "ES512", p521, "ec-521"),
  at_rs384_wrong: good({ ...base, at_hash: atHashOf("sha256") }, "RS384"),
  at_rs512_wrong: good({ ...base, at_hash: atHashOf("sha384") }, "RS512"),
  at_es384_wrong: good({ ...base, at_hash: atHashOf("sha512") }, "ES384", p384, "ec-384"),
  at_es512_wrong: good({ ...base, at_hash: atHashOf("sha256") }, "ES512", p521, "ec-521"),
};

// A valid token with its payload swapped for another subject, signature kept.
{
  const [h, , s] = tokens.rs256.split(".");
  tokens.tampered_payload = `${h}.${json({ ...base, sub: "victim" })}.${s}`;
  tokens.truncated_signature = tokens.rs256.slice(0, -10);
  tokens.empty_signature = `${h}.${tokens.rs256.split(".")[1]}.`;
}

const keySet = (...keys) => ({ keys });
const out = {
  now: NOW,
  issuer: ISS,
  client_id: AUD,
  nonce: "n-1",
  access_token: ACCESS_TOKEN,
  rsa_public_pem: rsa1.publicKey.export({ type: "spki", format: "pem" }),
  jwks: keySet(
    jwk(rsa1, "rsa-1", { use: "sig" }),
    jwk(p256, "ec-256"),
    jwk(p384, "ec-384"),
    jwk(p521, "ec-521"),
  ),
  jwks_rotated: keySet(jwk(rsa2, "rsa-2"), jwk(rsa1, "rsa-1")),
  jwks_other: keySet(jwk(rsa2, "rsa-1")),
  jwks_weak: keySet(jwk(weak, "rsa-weak"), jwk(rsa1, "rsa-1")),
  jwks_pinned: keySet(jwk(rsa1, "rsa-1", { alg: "RS256" })),
  jwks_p256_only: keySet(jwk(p256, "ec-256")),
  rsa2_jwk: jwk(rsa2, "rsa-2"),
  rsa1_jwk: jwk(rsa1, "rsa-1"),
  p256_jwk: jwk(p256, "ec-256"),
  tokens,
};
process.stdout.write(JSON.stringify(out, null, 1) + "\n");
