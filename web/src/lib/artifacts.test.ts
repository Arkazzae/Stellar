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
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

import { afterAll, describe, expect, it } from "vitest";

interface Artifact {
  id: string;
  path: string;
  size: number;
  sha256: string;
  source: { file?: string; recipe?: string; output?: string };
  upstream: { project: string; from: string; verified?: string };
  license: string;
}

const artifactsRoot = join(process.cwd(), "..", "artifacts");
const readJson = (path: string) => JSON.parse(readFileSync(path, "utf8"));

const manifest: Artifact[] = readJson(join(artifactsRoot, "manifest.json")).artifacts;
const webPackage = readJson(join(process.cwd(), "package.json"));
const resolveFromWeb = createRequire(join(process.cwd(), "package.json"));

/** `npm:<name>@<version>/<file>`, with the name possibly scoped. */
const NPM_FROM = /^npm:((?:@[^/]+\/)?[^@/]+)@([^/]+)\/(.+)$/;

function installedDirectory(name: string): string {
  const directory = (resolveFromWeb.resolve.paths(name) ?? [])
    .map((nodeModules) => join(nodeModules, name))
    .find((candidate) => existsSync(join(candidate, "package.json")));
  if (directory === undefined) throw new Error(`${name} is not installed`);
  return directory;
}

function sha256(bytes: Buffer): string {
  return createHash("sha256").update(bytes).digest("hex");
}

// The worklets and WASM binaries come from artifacts/, but the node classes
// that drive them are bundled from the installed package. A dependency update
// that moves the package on without the committed copies would pair new
// classes with old worklets. When this fails after an update, copy the files
// from node_modules into artifacts/files/ and record their new sha256, size
// and version in artifacts/manifest.json.
describe("artifacts copied from a dependency of the web client", () => {
  const copied = manifest.flatMap((artifact) => {
    const [, name, version, file] = NPM_FROM.exec(artifact.upstream.from) ?? [];
    return name !== undefined && name in webPackage.dependencies
      ? [{ artifact, name, version, file: file as string }]
      : [];
  });

  it("include the noise suppressor's worklets and binaries", () => {
    expect(copied.map(({ name }) => name)).toContain("@sapphi-red/web-noise-suppressor");
  });

  it.each(copied)("$artifact.id matches the installed $name", ({ artifact, name, version, file }) => {
    const directory = installedDirectory(name);
    const installed = readFileSync(join(directory, file));

    expect(readJson(join(directory, "package.json")).version).toBe(version);
    expect(sha256(installed)).toBe(artifact.sha256);
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
