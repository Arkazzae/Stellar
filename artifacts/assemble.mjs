/** Verify the browser package files before serving them from this instance. */
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const artifactsRoot = fileURLToPath(new URL(".", import.meta.url));
const manifestFile = join(artifactsRoot, "manifest.json");
const servedRoot = resolve(artifactsRoot, "..", "web", "public");

const SHA256 = /^[0-9a-f]{64}$/;
const web = createRequire(join(artifactsRoot, "../web/package.json"));
const packages = new Set(["@stellarr/audio-filters", "@stellarr/translation"]);

export async function readManifest() {
  return JSON.parse(await readFile(manifestFile, "utf8")).artifacts;
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

/** A relative path that stays below `root`, or null. */
function below(root, relative) {
  if (typeof relative !== "string" || relative === "" || isAbsolute(relative)) return null;
  const full = resolve(root, normalize(relative));
  return full.startsWith(root.endsWith(sep) ? root : root + sep) ? full : null;
}

function sourceFile(entry) {
  const name = entry.source?.package;
  if (!packages.has(name)) return null;
  const root = dirname(web.resolve(name + "/package.json"));
  return below(join(root, "dist"), entry.path);
}

/** Problems with the entry itself, before any file is read. */
function entryProblems(entry, seenIds, seenPaths) {
  const problems = [];
  if (typeof entry.id !== "string" || entry.id === "") problems.push("has no id");
  else if (seenIds.has(entry.id)) problems.push("repeats an id");
  if (below(servedRoot, entry.path) === null) problems.push("has a path outside web/public");
  else if (seenPaths.has(entry.path)) problems.push(`repeats the path ${entry.path}`);
  if (sourceFile(entry) === null) problems.push("has no usable source");
  if (!SHA256.test(entry.sha256 ?? "")) problems.push("has no sha256");
  if (!Number.isSafeInteger(entry.size) || entry.size < 0) problems.push("has no size");
  if (typeof entry.upstream?.project !== "string" || typeof entry.upstream?.from !== "string") {
    problems.push("does not name its upstream");
  }
  if (typeof entry.license !== "string" || entry.license === "") problems.push("has no license");
  seenIds.add(entry.id);
  seenPaths.add(entry.path);
  return problems;
}

async function readIfPresent(file) {
  try {
    return await readFile(file);
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw error;
  }
}

/** Whether `file` already holds exactly these bytes, so it need not be written. */
async function holds(file, size, hash) {
  try {
    if ((await stat(file)).size !== size) return false;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
  return sha256(await readFile(file)) === hash;
}

/**
 * Writes through a temporary sibling and renames it into place. The Docker dev
 * service creates these files as root in a bind-mounted directory, so reopening
 * one for writing from the host fails with EACCES while replacing the directory
 * entry still works. The rename also never exposes a half-written file.
 */
async function writeReplacing(target, bytes) {
  await mkdir(dirname(target), { recursive: true });
  const temporary = join(dirname(target), `.${process.pid}.${Date.now()}.tmp`);
  try {
    await writeFile(temporary, bytes);
    await rename(temporary, target);
  } catch (error) {
    await rm(temporary, { force: true });
    throw error;
  }
}

/**
 * Checks every entry and copies those whose served copy differs. Returns the
 * problems found, one line each; an empty list means every file is in place.
 */
export async function assemble({ log = () => {} } = {}) {
  const entries = await readManifest();
  const problems = [];
  const seenIds = new Set();
  const seenPaths = new Set();
  let written = 0;

  for (const entry of entries) {
    const name = entry.id ?? entry.path ?? "an entry";
    const invalid = entryProblems(entry, seenIds, seenPaths);
    if (invalid.length > 0) {
      problems.push(...invalid.map((problem) => `${name} ${problem}`));
      continue;
    }

    const source = sourceFile(entry);
    const bytes = await readIfPresent(source);
    if (bytes === null) {
      problems.push(name + ": " + source + " is missing from " + entry.source.package);
      continue;
    }
    const hash = sha256(bytes);
    if (bytes.byteLength !== entry.size || hash !== entry.sha256) {
      problems.push(name + ": expected " + entry.size + " bytes with sha256 " + entry.sha256 +
        ", found " + bytes.byteLength + " bytes with sha256 " + hash + " in " + source);
      continue;
    }

    const target = below(servedRoot, entry.path);
    if (await holds(target, entry.size, entry.sha256)) continue;
    try {
      await writeReplacing(target, bytes);
      written += 1;
    } catch (error) {
      // A directory the Docker dev service created belongs to root on a Linux
      // host, and nothing but root can add an entry to it.
      const hint = error?.code === "EACCES" ? "; remove the directory and run again" : "";
      problems.push(`${name}: cannot write ${target}: ${error.message}${hint}`);
    }
  }

  log(`[artifacts] ${entries.length} checked, ${written} written to web/public`);
  return problems;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const problems = await assemble({ log: console.log });
  if (problems.length > 0) {
    console.error(`[artifacts] ${problems.length} problem(s), and those files were not written:`);
    for (const problem of problems) console.error(`  - ${problem}`);
    process.exitCode = 1;
  }
}
