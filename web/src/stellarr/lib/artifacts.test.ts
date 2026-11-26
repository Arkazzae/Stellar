import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

import { afterAll, describe, expect, it } from "vitest";

import { PROCESSOR_NAME, WORKLET_URL } from "./voice/capture/noiseSuppression";

interface Artifact {
  id: string;
  path: string;
  size: number;
  sha256: string;
  source: { package: string };
  upstream: { project: string; from: string; integrity?: string; verified?: string };
  license: string;
}

const artifactsRoot = join(process.cwd(), "..", "artifacts");
const readJson = (path: string) => JSON.parse(readFileSync(path, "utf8"));

const manifest: Artifact[] = readJson(join(artifactsRoot, "manifest.json")).artifacts;

function sha256(bytes: Buffer): string {
  return createHash("sha256").update(bytes).digest("hex");
}

describe("the installed browser packages", () => {
  it.each(Object.entries(WORKLET_URL))("serves the processor registered for %s", (kind, url) => {
    const entry = manifest.find((item) => "/" + item.path === url);
    expect(entry?.source.package).toBe("@stellarr/audio-filters");
    const code = readFileSync(join(process.cwd(), "public", url), "utf8");
    expect(code).toContain(PROCESSOR_NAME[kind as keyof typeof PROCESSOR_NAME]);
    expect(code).toContain("registerProcessor(");
  });

  it("serves the exact bytes recorded for every installed artifact", () => {
    for (const item of manifest) {
      const bytes = readFileSync(join(process.cwd(), "public", item.path));
      expect({ path: item.path, size: bytes.length, sha256: sha256(bytes) }).toEqual({
        path: item.path, size: item.size, sha256: item.sha256,
      });
    }
  });
});

describe("assembling the artifacts", () => {
  const fixtures: string[] = [];

  afterAll(() => {
    for (const fixture of fixtures) rmSync(fixture, { recursive: true, force: true });
  });

  /** A copy of the real script, manifest and source files in a temporary tree. */
  function fixture(bytes: Buffer): { root: string; source: string; target: string } {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "stellarr-artifacts-")));
    fixtures.push(root);

    const script = join(root, "artifacts", "assemble.mjs");
    const pkg = join(root, "node_modules", "@stellarr", "audio-filters");
    const source = join(pkg, "dist", "audio", "example.bin");
    const target = join(root, "web", "public", "audio", "example.bin");
    mkdirSync(join(pkg, "dist", "audio"), { recursive: true });
    mkdirSync(join(root, "artifacts"));
    writeFileSync(join(pkg, "package.json"), JSON.stringify({ name: "@stellarr/audio-filters" }));
    copyFileSync(join(artifactsRoot, "assemble.mjs"), script);
    writeFileSync(source, bytes);
    writeFileSync(
      join(root, "artifacts", "manifest.json"),
      JSON.stringify({
        artifacts: [
          {
            id: "test/example",
            path: "audio/example.bin",
            source: { package: "@stellarr/audio-filters" },
            sha256: sha256(bytes),
            size: bytes.byteLength,
            upstream: { project: "https://example.invalid", from: "a test" },
            license: "MIT",
          },
        ],
      }),
    );
    return { root, source, target };
  }

  function runAssemble(root: string) {
    return spawnSync(process.execPath, [join(root, "artifacts", "assemble.mjs")], {
      cwd: root,
      encoding: "utf8",
    });
  }

  it("copies a matching file to its served path", () => {
    const bytes = Buffer.from("assembled bytes");
    const { root, target } = fixture(bytes);

    const result = runAssemble(root);

    expect(result.status).toBe(0);
    expect(readFileSync(target)).toEqual(bytes);
    expect(result.stdout).toContain("1 checked, 1 written");
  });

  it("fails without writing when the source no longer matches the manifest", () => {
    const { root, source, target } = fixture(Buffer.from("recorded bytes"));
    writeFileSync(source, "edited bytes");

    const result = runAssemble(root);

    expect(result.status).toBe(1);
    expect(result.stderr).toContain("test/example");
    expect(result.stderr).toContain("sha256");
    expect(existsSync(target)).toBe(false);
  });

  it("fails without writing when the source file is missing", () => {
    const { root, source, target } = fixture(Buffer.from("recorded bytes"));
    rmSync(source);

    const result = runAssemble(root);

    expect(result.status).toBe(1);
    expect(result.stderr).toContain("test/example");
    expect(result.stderr).toContain("is missing");
    expect(existsSync(target)).toBe(false);
  });
});
