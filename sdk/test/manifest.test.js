import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { manifestOf, roomOptions } from "../dev/files.js";

const { base, cases, rawCases } = JSON.parse(await readFile(new URL("./fixtures/manifest-parity.json", import.meta.url)));
const parse = patch => manifestOf(JSON.stringify({ ...base, ...patch }));

test("SDK manifest validation matches the production manifest cases", () => {
  for (const { label, fields, valid } of cases) {
    if (valid) assert.doesNotThrow(() => parse(fields), label);
    else assert.throws(() => parse(fields), { status: 400 }, label);
  }

  const defaults = parse({ authority: null, seats: null, icon: null, cover: null, devices: null });
  assert.deepEqual(defaults, base);
  assert.deepEqual(roomOptions(defaults), { authority: undefined, seats: undefined, permissions: undefined });
});

test("raw manifests match Serde's treatment of duplicate fields", () => {
  const prefix = JSON.stringify(base).slice(0, -1);
  for (const { label, members, valid } of rawCases) {
    const manifest = `${prefix},${members}}`;
    if (valid) assert.doesNotThrow(() => manifestOf(manifest), label);
    else assert.throws(() => manifestOf(manifest), { status: 400, message: /Duplicate manifest field/ }, label);
  }
});

test("manifest text and hosts use the production byte and count limits", () => {
  const maxName = "é".repeat(120);
  const maxDescription = "é".repeat(2000);
  const maxReason = "é".repeat(512);
  const exactHost = ["a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(61)].join(".");
  const connects = Array.from({ length: 64 }, () => ({ host: exactHost, why: maxReason }));
  assert.doesNotThrow(() => parse({ name: maxName, description: maxDescription, connects }));
  assert.throws(() => parse({ name: maxName + "x" }), { status: 400 });
  assert.throws(() => parse({ description: maxDescription + "x" }), { status: 400 });
  assert.throws(() => parse({ connects: [{ host: exactHost, why: maxReason + "x" }] }), { status: 400 });
  assert.throws(() => parse({ connects: [{ host: exactHost + "d", why: "Video" }] }), { status: 400 });
  assert.throws(() => parse({ connects: [{ host: "a".repeat(64) + ".example", why: "Video" }] }), { status: 400 });
  assert.throws(() => parse({ connects: [...connects, connects[0]] }), { status: 400 });
});

test("manifest unsigned fields use integer JSON tokens and text is valid UTF-8", () => {
  const manifest = JSON.stringify({ ...base, seats: 2 });
  for (const protocol of ["1.0", "1e0", "1.00000000000000001"])
    assert.throws(() => manifestOf(manifest.replace('"protocol":1', '"protocol":' + protocol)), { status: 400 });
  for (const seats of ["2.0", "2e0", "2.00000000000000001"])
    assert.throws(() => manifestOf(manifest.replace('"seats":2', '"seats":' + seats)), { status: 400 });

  const unknown = { protocol: 1, seats: 2 };
  const withExtension = manifest.replace(/}$/, ',"extension":{"protocol":1.0,"seats":2e0}}');
  assert.deepEqual(manifestOf(withExtension).extension, unknown);
  const invalidUtf8 = Buffer.from(JSON.stringify(base));
  invalidUtf8[invalidUtf8.indexOf("Manifest parity")] = 0xff;
  assert.throws(() => manifestOf(invalidUtf8), { status: 400 });
  assert.throws(() => parse({ name: "\ud800" }), { status: 400 });
});
