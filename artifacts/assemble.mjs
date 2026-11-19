/**
 * Puts every prebuilt file the browser loads at run time where it is served,
 * after checking that its bytes are the ones artifacts/manifest.json records.
 *
 * `AudioWorklet.addModule`, `WebAssembly.instantiate`, `importScripts` and the
 * model downloads all need a real URL, so these files cannot go through the
 * bundler. The manifest is the one place that says where each of them came
 * from. Every entry has:
 *
 *   id        a logical name, which outlives any one origin of the file
 *   path      the path it is served under: its URL path, and its place below
 *             web/public/, the directory this script writes to
 *   source    where this script takes the bytes from, one of
 *               { "file": "<path below artifacts/>" }  a file committed here
 *               { "recipe": "<name>", "output": "<file name>" }
 *                 a file built by artifacts/recipes/<name>/ into its out/
 *                 directory, as the README there describes
 *   sha256, size  of those bytes
 *   upstream  { project, from, verified }: the project the file belongs to,
 *             the URL or npm package file it was taken from, and what its
 *             bytes were checked against when it was. A file taken from an
 *             npm package may also record the tarball's `integrity`, so the
 *             check can be repeated once no lockfile names the package
 *   license   an SPDX expression
 *
 * Replacing a file, say a prebuilt WASM with one built from source here, is a
 * change to its entry alone: the served path, and so the client, stay the same.
 *
 * Runs from the predev, prebuild and pretest npm hooks of web/, so the dev
 * server, the build and the tests always see the recorded files. Any missing file or
 * any difference from the manifest fails the run, in development as in an
 * image build. A committed file that is missing or different means a broken
 * checkout or an edit nobody recorded. A recipe's output that is missing has
 * not been built yet, and one that differs was built from other inputs, such
 * as an out/ folder left from before the recipe changed. None of these should
 * reach a browser or be hidden by the client falling back to something else.
 * This script never builds a recipe itself; `docker buildx bake recipes` does.
 *
 *   node artifacts/assemble.mjs
 */
import { createHash } from "node:crypto";
import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const artifactsRoot = fileURLToPath(new URL(".", import.meta.url));
const manifestFile = join(artifactsRoot, "manifest.json");
const servedRoot = resolve(artifactsRoot, "..", "web", "public");

const SHA256 = /^[0-9a-f]{64}$/;
const RECIPE_NAME = /^[a-z0-9][a-z0-9-]*$/;

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

function sourceFile(source) {
  if (typeof source?.file === "string") return below(artifactsRoot, source.file);
  if (RECIPE_NAME.test(source?.recipe ?? "")) {
    return below(join(artifactsRoot, "recipes", source.recipe, "out"), source.output);
  }
  return null;
}

/** Problems with the entry itself, before any file is read. */
function entryProblems(entry, seenIds, seenPaths) {
  const problems = [];
  if (typeof entry.id !== "string" || entry.id === "") problems.push("has no id");
  else if (seenIds.has(entry.id)) problems.push("repeats an id");
  if (below(servedRoot, entry.path) === null) problems.push("has a path outside web/public");
  else if (seenPaths.has(entry.path)) problems.push(`repeats the path ${entry.path}`);
  if (sourceFile(entry.source) === null) problems.push("has no usable source");
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

    const source = sourceFile(entry.source);
    const bytes = await readIfPresent(source);
    if (bytes === null) {
      const hint =
        entry.source.recipe === undefined
          ? ""
          : " (build the recipes first: docker buildx bake recipes, at the repository root)";
      problems.push(`${name}: ${source} is missing${hint}`);
      continue;
    }
    const hash = sha256(bytes);
    if (bytes.byteLength !== entry.size || hash !== entry.sha256) {
      const hint =
        entry.source.recipe === undefined ? "" : " (rebuild the recipes: docker buildx bake recipes)";
      problems.push(
        `${name}: ${source} is ${bytes.byteLength} bytes with sha256 ${hash}, ` +
          `the manifest records ${entry.size} bytes with sha256 ${entry.sha256}${hint}`,
      );
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
