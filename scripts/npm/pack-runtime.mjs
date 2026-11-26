import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { rolldown } from "rolldown";

const [name, inputDir, destination] = process.argv.slice(2);
if (!["translation", "audio-filters"].includes(name) || !inputDir || !destination) {
  throw new Error("Usage: pack-runtime.mjs audio-filters|translation <extracted inputs> <tarball directory>");
}
const root = resolve(import.meta.dirname, "../..");
const inputs = resolve(inputDir);
const out = resolve(destination);
const staging = await mkdtemp(join(tmpdir(), "stellarr-npm-"));
const artifacts = [];
const project = "https://github.com/Arkazzae/Stellarr";

async function add(path, bytes, metadata, expected) {
  const size = bytes.length;
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  if (expected && (expected.sha256 !== sha256 || expected.size !== size)) {
    throw new Error("Unexpected bytes for " + path + ": " + sha256);
  }
  const target = join(staging, "dist", path);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, bytes);
  artifacts.push({ ...metadata, path, size, sha256 });
}

async function worklet(engine, entry, alias = {}) {
  const bundle = await rolldown({ input: entry, platform: "browser", resolve: { alias } });
  try {
    const { output } = await bundle.generate({ format: "es", minify: true });
    if (output.length !== 1 || output[0].type !== "chunk") throw new Error("Expected a single worklet for " + engine);
    await add("audio/" + engine + "-worklet.js", Buffer.from(output[0].code), {
      id: "noise-suppression/" + engine + "-worklet",
      license: "MIT",
      upstream: engine === "gtcrn"
        ? { project: "https://github.com/sapphi-red/web-noise-suppressor", from: "1e168964f8e99835c43dd8fb81fde9f273e5699b and the GTCRN JS glue built by the Stellarr recipe" }
        : { project, from: "web/src/stellarr/lib/voice/capture/" + engine + "/processor.ts" },
    });
  } finally {
    await bundle.close();
  }
}

try {
  const manifest = JSON.parse(await readFile(join(root, "artifacts/manifest.json"), "utf8"));
  const audio = name === "audio-filters";
  const registry = {};
  for (const e of manifest.artifacts) {
    if (e.path.startsWith("audio/") !== audio || e.path.endsWith("-worklet.js")) continue;
    let source;
    let upstream = e.upstream;
    if (e.path === "audio/gtcrn.wasm") {
      source = join(inputs, "gtcrn/wasm-out/gtcrn.wasm");
      upstream = { project: e.upstream.project, from: "24662ff13f184b96edf91f1568e186341a8ac988, compiled by Stellarr with Emscripten 4.0.10" };
    } else if (e.path.startsWith("bergamot-models/")) {
      const [, direction, filename] = e.path.split("/");
      source = join(inputs, "models", direction, filename);
      (registry[direction] ??= {})[filename.split(".")[0]] = { name: filename, size: e.size, sha256: e.sha256 };
    } else {
      source = join(root, "artifacts/recipes", e.source.recipe, "out", e.source.output);
    }
    await add(e.path, await readFile(source), { id: e.id, upstream, license: e.license }, e);
  }
  if (audio) {
    for (const engine of ["speex", "rnnoise"]) {
      await worklet(engine, join(root, "web/src/stellarr/lib/voice/capture", engine, "processor.ts"));
    }
    await worklet("gtcrn", join(inputs, "gtcrn/worklet/src/gtcrn/workletProcessor.ts"), {
      "@sapphi-red/gtcrn-wasm": join(inputs, "gtcrn/src/index.ts"),
    });
  } else {
    await add("bergamot-models/registry.json", Buffer.from(JSON.stringify(registry, null, 2) + "\n"), {
      id: "translation/registry", upstream: { project, from: "artifacts/manifest.json" }, license: "MIT",
    });
  }
  for (const file of ["package.json", "README.md"]) {
    await copyFile(join(root, "artifacts/npm", name, file), join(staging, file));
  }
  await copyFile(join(root, "artifacts/NOTICES"), join(staging, "NOTICES"));
  await writeFile(join(staging, "manifest.json"), JSON.stringify({ artifacts }, null, 2) + "\n");
  await mkdir(out, { recursive: true });
  execFileSync("npm", ["pack", "--ignore-scripts", "--pack-destination", out], { cwd: staging, stdio: "inherit" });
} finally {
  await rm(staging, { recursive: true, force: true });
}
