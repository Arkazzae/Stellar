// Refresh the application's inventory after installing new pinned package versions.
import { readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";

const web = createRequire(new URL("../../web/package.json", import.meta.url));
const dependencies = web("./package.json").dependencies;
const artifacts = [];
for (const name of ["@stellarr/audio-filters", "@stellarr/translation"]) {
  const metadata = web(name + "/package.json");
  if (dependencies[name] !== metadata.version) throw new Error("Install the pinned version of " + name);
  const root = dirname(web.resolve(name + "/package.json"));
  const manifest = JSON.parse(await readFile(join(root, "manifest.json"), "utf8"));
  for (const entry of manifest.artifacts) artifacts.push({ ...entry, source: { package: name } });
}
const file = new URL("../../artifacts/manifest.json", import.meta.url);
const contents = JSON.stringify({ artifacts }, null, 2) + "\n";
if (process.argv.includes("--check")) {
  if (await readFile(file, "utf8") !== contents) throw new Error("Run node scripts/npm/sync-manifest.mjs");
} else {
  await writeFile(file, contents);
}
