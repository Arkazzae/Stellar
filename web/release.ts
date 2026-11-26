import { createHash } from "node:crypto";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";

import type { Plugin, Rollup } from "vite";

import { BUILD_DIRECTORY, fileDirectories, WEB_DOCUMENT } from "./documents";

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
 *
 * The file also lists the web client's shell, which the service worker keeps
 * a copy of (public/stellarr-worker.js), and the folders that hold only
 * files, with which the worker tells the web client's addresses from the
 * rest the way `documentFor` does.
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

/** What the service worker needs to know of a release. */
export interface ReleaseManifest {
  id: string;
  /** The files of the web client's shell, by address, its document first. */
  shell: string[];
  /** The folders that hold only files and never pages; see `fileDirectories`. */
  directories: string[];
}

/** The contents of /release.json. */
export function releaseFile(manifest: ReleaseManifest): string {
  return `${JSON.stringify(manifest)}\n`;
}

/** How a chunk of the build reaches the others and its styles. */
export interface ChunkLinks {
  imports: readonly string[];
  dynamicImports: readonly string[];
  css: readonly string[];
}

/** The links of each chunk of a bundle, by file name. */
export function chunkLinks(bundle: Rollup.OutputBundle): Map<string, ChunkLinks> {
  const links = new Map<string, ChunkLinks>();
  for (const output of Object.values(bundle)) {
    if (output.type !== "chunk") continue;
    links.set(output.fileName, {
      imports: output.imports,
      dynamicImports: output.dynamicImports,
      css: [...(output.viteMetadata?.importedCss ?? [])],
    });
  }
  return links;
}

/**
 * An address in the build's own folders, in the text of a file that names one.
 */
const BUILD_ADDRESS = new RegExp(`/(?:${BUILD_DIRECTORY}|fonts)/[\\w.-]+`, "g");

/** The files whose text may name others: documents, scripts and styles. */
const TEXT_FILE = new RegExp("[.](?:html|js|css)$");

/**
 * The files of the web client's shell: its document and what the document and
 * the signed-in app load from /static/ and /fonts/. That is the document's
 * own scripts and styles, every chunk they import, on demand or not, and the
 * images and fonts that any of these name. Cockpit's own files are not
 * among them, for the web client's document leads nowhere near them. Nor is
 * anything outside those two folders: the worklets, codecs, models and
 * sounds keep going to the network.
 *
 * `read` gives the text of a file of the build, or null when it has none.
 */
export function shellFiles(chunks: ReadonlyMap<string, ChunkLinks>, read: (file: string) => string | null): string[] {
  const shell = new Set([WEB_DOCUMENT]);
  const pending = [WEB_DOCUMENT];
  const visit = (file: string) => {
    if (shell.has(file) || read(file) === null) return;
    shell.add(file);
    pending.push(file);
  };
  for (let file = pending.pop(); file !== undefined; file = pending.pop()) {
    const links = chunks.get(file);
    for (const linked of [...(links?.imports ?? []), ...(links?.dynamicImports ?? []), ...(links?.css ?? [])]) visit(linked);
    if (!TEXT_FILE.test(file)) continue;
    for (const [address] of (read(file) ?? "").matchAll(BUILD_ADDRESS)) visit(address.slice(1));
  }
  const others = [...shell].filter((file) => file !== WEB_DOCUMENT).sort();
  return [WEB_DOCUMENT, ...others].map((file) => `/${file}`);
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
 * output directory, in the web client's document and in /release.json, with
 * the shell's files. It runs once the build has written its files, public/
 * among them, since the id has to cover all of them; the document is hashed
 * as the build wrote it, before the id is in it.
 */
export function stellarrRelease(options: { publicDir: string }): Plugin {
  let outDir = "";
  let chunks = new Map<string, ChunkLinks>();
  return {
    name: "stellarr-release",
    apply: "build",
    configResolved(config) {
      outDir = resolve(config.root, config.build.outDir);
    },
    generateBundle(_options, bundle) {
      chunks = chunkLinks(bundle);
    },
    closeBundle() {
      const files = readTree(outDir);
      const document = files.get(WEB_DOCUMENT);
      if (document === undefined) return;
      const id = releaseId(files);
      const read = (file: string) => (files.has(file) ? Buffer.from(files.get(file) ?? []).toString("utf8") : null);
      const shell = shellFiles(chunks, read);
      const directories = fileDirectories(options.publicDir);
      writeFileSync(join(outDir, WEB_DOCUMENT), stampDocument(Buffer.from(document).toString("utf8"), id));
      writeFileSync(join(outDir, RELEASE_FILE), releaseFile({ id, shell, directories }));
    },
  };
}
