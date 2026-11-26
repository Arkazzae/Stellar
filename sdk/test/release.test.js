import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, readdir, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { packageRelease } from "../dev/release.js";

const run = promisify(execFile);
const cli = fileURLToPath(new URL("../cli.js", import.meta.url));
const icon = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aKioAAAAASUVORK5CYII=", "base64");
const manifest = id => ({ protocol: 1, id, name: id.toUpperCase(), description: "Activity fixture.", version: "1.0.0", icon: "art/icon.png" });

async function activity(folder, id, artwork = true) {
  await mkdir(join(folder, "art"), { recursive: true });
  await writeFile(join(folder, "index.html"), "<title>" + id + "</title>");
  await writeFile(join(folder, "script [1].js"), "export const ready = true;");
  const value = manifest(id);
  if (!artwork) delete value.icon;
  await writeFile(join(folder, "stellarr.json"), JSON.stringify(value));
  if (artwork) await writeFile(join(folder, "art", "icon.png"), icon);
}

function zipPaths(bytes) {
  const end = bytes.length - 22;
  assert.equal(bytes.readUInt32LE(end), 0x06054b50, "ZIP end record");
  const paths = [];
  let at = bytes.readUInt32LE(end + 16);
  const count = bytes.readUInt16LE(end + 10);
  for (let index = 0; index < count; index++) {
    assert.equal(bytes.readUInt32LE(at), 0x02014b50, "ZIP entry");
    const size = bytes.readUInt16LE(at + 28);
    paths.push(bytes.subarray(at + 46, at + 46 + size).toString("utf8"));
    at += 46 + size + bytes.readUInt16LE(at + 30) + bytes.readUInt16LE(at + 32);
  }
  return paths;
}

test("independent release builds produce the same listing, entries and digest", async t => {
  const folder = await mkdtemp(join(tmpdir(), "activity-release-test-"));
  t.after(() => rm(folder, { recursive: true, force: true }));
  const first = join(folder, "one");
  const second = join(folder, "two");
  const output = join(first, "release output");
  const secondOutput = join(folder, "independent output");
  await activity(first, "one");
  await activity(second, "two");
  await mkdir(output);
  await writeFile(join(output, "not-an-activity.js"), "do not package");
  await mkdir(join(first, "old release"));
  await writeFile(join(first, "old release", "stellarr-release.json"), "{}");

  const items = await packageRelease([second, first], output);
  assert.deepEqual(items.map(item => item.id), ["one", "two"]);
  const firstIndex = await readFile(join(output, "stellarr-release.json"));
  assert.deepEqual(JSON.parse(firstIndex), { protocol: 1, activities: items });
  for (const item of items) {
    const bytes = await readFile(join(output, item.asset));
    assert.equal(bytes.length, item.bytes);
    assert.equal(createHash("sha256").update(bytes).digest("hex"), item.sha256);
    assert.deepEqual(zipPaths(bytes), ["art/icon.png", "index.html", "script [1].js", "stellarr.json"]);
  }

  // The same browser files produce the same archives after an unrelated checkout or mtime change.
  await rm(join(output, "not-an-activity.js"));
  await utimes(join(first, "index.html"), new Date(0), new Date(0));
  await utimes(join(second, "art", "icon.png"), new Date(0), new Date(0));
  assert.deepEqual(await packageRelease([first, second], secondOutput), items);
  assert.deepEqual(await readFile(join(secondOutput, "stellarr-release.json")), firstIndex);
  for (const item of items)
    assert.deepEqual(await readFile(join(secondOutput, item.asset)), await readFile(join(output, item.asset)));

  const existing = await readdir(output);
  await writeFile(join(first, "index.html"), "<title>Unpublished change</title>");
  await assert.rejects(packageRelease([first, first], output), /Duplicate activity ID/);
  assert.deepEqual(await readFile(join(output, "stellarr-release.json")), firstIndex);
  assert.deepEqual(await readdir(output), existing);
  for (const item of items)
    assert.deepEqual(await readFile(join(output, item.asset)), await readFile(join(secondOutput, item.asset)));
});

test("validate and pack require an icon, and accept a valid icon without cover", async t => {
  const folder = await mkdtemp(join(tmpdir(), "activity-release-cli-"));
  t.after(() => rm(folder, { recursive: true, force: true }));
  const source = join(folder, "activity");
  const output = join(folder, "release");
  await activity(source, "missing", false);
  for (const args of [["validate"], ["pack", "--out", output]]) {
    await assert.rejects(run(process.execPath, [cli, ...args, "--dir", source]), error =>
      error.code === 1 && /Add an icon path to stellarr.json/.test(error.stderr));
  }
  await activity(source, "valid");
  const validated = await run(process.execPath, [cli, "validate", "--dir", source]);
  assert.match(validated.stdout, /valid 1\.0\.0 · 4 files/);
  const packed = await run(process.execPath, [cli, "pack", "--dir", source, "--out", output]);
  assert.match(packed.stdout, /stellarr-release\.json/);
  assert.equal(JSON.parse(await readFile(join(output, "stellarr-release.json"))).activities[0].id, "valid");
  await assert.rejects(run(process.execPath, [cli, "pack", "--dir", source, "--out", source]), error =>
    error.code === 1 && /separate from the activity root/.test(error.stderr));
});
