import { createHash } from "node:crypto";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative, sep } from "node:path";

import type { Plugin } from "vite";

import { WEB_DOCUMENT } from "./documents";

/*
 * The release id: what a running page compares to learn that the server has
 * moved on. It is one hash over every file the build writes, so it changes
 * when any served file does and is the same for identical output, and it is
 * published twice. /release.json is the question a running page asks, past
 * every cache; nginx serves it under the `no-cache` rule of every file that
 * keeps its name (infra/nginx.conf), and the page fetches it with
 * `cache: "no-store"` besides. The web client's document carries it in a
 * <meta>, which is how a loaded page learns which release it is itself. An id
 * baked into a chunk would change that chunk, and a chunk's name is its content.
 */

/** Where the build writes the release file, from the web root. */
export const RELEASE_FILE = "release.json";

/** The address a page asks for it, which `lib/platform/release.ts` repeats. */
export const RELEASE_PATH = `/${RELEASE_FILE}`;

/** The name of the <meta> the web client's document carries its release in. */
export const RELEASE_META = "stellarr-release";

/**
 * One id for a set of files, named by their paths from the web root. The
 * order they are given in does not matter: the same files always give the same
 * id, and a changed byte, a renamed file or a file added or removed gives
 * another.
 */
export function releaseId(files: ReadonlyMap<string, Uint8Array>): string {
  const hash = createHash("sha256");
  for (const path of [...files.keys()].sort()) {
    const bytes = files.get(path) ?? new Uint8Array();
    // The lengths keep one file's tail from passing for the next one's head.
    hash.update(`${path.length}:${path}${bytes.length}:`);
    hash.update(bytes);
  }
  return hash.digest("hex").slice(0, 20);
}

/** The document with the release it belongs to written into its <head>. */
export function stampDocument(html: string, id: string): string {
  if (!html.includes("</head>")) throw new Error(`The web client's document has no </head> to write the release into.`);
  return html.replace("</head>", `  <meta name="${RELEASE_META}" content="${id}" />\n  </head>`);
}

/** The contents of /release.json. */
export function releaseFile(id: string): string {
  return `${JSON.stringify({ id })}\n`;
}

/** Every file below `directory`, by its path from there with forward slashes. */
function readTree(directory: string): Map<string, Uint8Array> {
  const files = new Map<string, Uint8Array>();
  for (const entry of readdirSync(directory, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile()) continue;
    const path = join(entry.parentPath, entry.name);
    files.set(relative(directory, path).split(sep).join("/"), readFileSync(path));
  }
  return files;
}

/**
 * Writes the release into a finished build: the id of everything in the
 * output directory, in the web client's document and in /release.json. It
 * runs once the build has written its files, public/ among them, since the
 * id has to cover all of them; the document is hashed as the build wrote it,
 * before the id is in it.
 */
export function stellarrRelease(): Plugin {
  let outDir = "";
  return {
    name: "stellarr-release",
    apply: "build",
    configResolved(config) {
      outDir = join(config.root, config.build.outDir);
    },
    closeBundle() {
      const files = readTree(outDir);
      const document = files.get(WEB_DOCUMENT);
      if (document === undefined) return;
      const id = releaseId(files);
      writeFileSync(join(outDir, WEB_DOCUMENT), stampDocument(Buffer.from(document).toString("utf8"), id));
      writeFileSync(join(outDir, RELEASE_FILE), releaseFile(id));
    },
  };
}
