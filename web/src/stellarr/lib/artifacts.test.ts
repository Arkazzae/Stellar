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
  source: { file?: string; recipe?: string; output?: string };
  upstream: { project: string; from: string; integrity?: string; verified?: string };
  license: string;
}

const artifactsRoot = join(process.cwd(), "..", "artifacts");
const readJson = (path: string) => JSON.parse(readFileSync(path, "utf8"));

const manifest: Artifact[] = readJson(join(artifactsRoot, "manifest.json")).artifacts;

function sha256(bytes: Buffer): string {
  return createHash("sha256").update(bytes).digest("hex");
}

// The worklets come from artifacts/, the code that starts them is ours in
// noiseSuppression.ts. Replacing a worklet with another build or version must
// not leave the two speaking past each other: a node constructed under a name
// nothing registered throws, and options the processor does not read leave it
// without a binary.
describe("the noise-suppression worklets", () => {
  const engines = Object.entries(WORKLET_URL).map(([kind, url]) => {
    const source = manifest.find((entry) => `/${entry.path}` === url)?.source.file;
    return {
      kind: kind as keyof typeof PROCESSOR_NAME,
      url,
      code: source === undefined ? "" : readFileSync(join(artifactsRoot, source), "utf8"),
    };
  });

  it.each(engines)("$url registers the processor $kind is started as", ({ kind, code }) => {
    const registered = [...code.matchAll(/registerProcessor\(\s*[`'"]([^`'"]+)[`'"]/g)];
    expect(registered.map(([, name]) => name)).toEqual([PROCESSOR_NAME[kind]]);
  });

  it.each(engines)("$url reads the options and the message it is given", ({ code }) => {
    expect(code).toContain("processorOptions.wasmBinary");
    expect(code).toContain("processorOptions.maxChannels");
    expect(code).toMatch(/\.data\s*===\s*[`'"]destroy[`'"]/);
  });
});

describe("the artifact manifest", () => {
  it("records the size and hash of every committed source file", () => {
    const files = manifest.filter((artifact) => artifact.source.file !== undefined);
    expect(files.length).toBeGreaterThan(0);

    for (const artifact of files) {
      const bytes = readFileSync(join(artifactsRoot, artifact.source.file as string));
      expect({ id: artifact.id, size: bytes.byteLength }).toEqual({
        id: artifact.id,
        size: artifact.size,
      });
      expect({ id: artifact.id, sha256: sha256(bytes) }).toEqual({
        id: artifact.id,
        sha256: artifact.sha256,
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
    const source = join(root, "artifacts", "files", "example.bin");
    const target = join(root, "web", "public", "audio", "example.bin");
    mkdirSync(join(root, "artifacts", "files"), { recursive: true });
    copyFileSync(join(artifactsRoot, "assemble.mjs"), script);
    writeFileSync(source, bytes);
    writeFileSync(
      join(root, "artifacts", "manifest.json"),
      JSON.stringify({
        artifacts: [
          {
            id: "test/example",
            path: "audio/example.bin",
            source: { file: "files/example.bin" },
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
