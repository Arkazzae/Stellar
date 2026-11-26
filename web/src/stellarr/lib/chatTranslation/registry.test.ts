import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

interface ServedFile {
  path: string;
  size: number;
  sha256: string;
}

const readJson = (path: string) => JSON.parse(readFileSync(join(process.cwd(), path), "utf8"));

const registry: Record<string, Record<string, { name: string; size: number; sha256: string }>> =
  readJson("public/bergamot-models/registry.json");
const manifest: ServedFile[] = readJson("../artifacts/manifest.json").artifacts;

const byPath = (files: ServedFile[]) =>
  files
    .map(({ path, size, sha256 }) => ({ path, size, sha256 }))
    .sort((a, b) => a.path.localeCompare(b.path));

// The registry is what the client reads to find a pair's files and the total
// it shows progress against; the manifest is what decides which bytes are
// served. Disagreeing on a name, size or hash, they would send a download to
// a file the image lacks or cache bytes the manifest never approved.
describe("the model registry", () => {
  it("names exactly the model files the artifact manifest serves, at their sizes and hashes", () => {
    const listed = Object.entries(registry).flatMap(([pair, files]) =>
      Object.values(files).map((file) => ({
        path: `bergamot-models/${pair}/${file.name}`,
        size: file.size,
        sha256: file.sha256,
      })),
    );
    const served = manifest.filter((entry) => entry.path.startsWith("bergamot-models/") && entry.path !== "bergamot-models/registry.json");

    expect(byPath(listed)).toEqual(byPath(served));
  });
});
