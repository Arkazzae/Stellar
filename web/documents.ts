import { readdirSync, rmSync } from "node:fs";
import { join } from "node:path";

import type { Connect, Plugin } from "vite";

import { THEME_SCRIPT } from "./src/stellarr/design/themeScript";
import { INSTANCE_LANGUAGE_PATH, instanceLanguageSource, LOCALE_SCRIPT } from "./src/stellarr/i18n/localeScript";

/*
 * The two documents of the build and which addresses each one answers. The
 * dev and preview servers apply these rules below, and the production server,
 * nginx in the container (infra/nginx.conf), has to apply the same ones:
 * every address of an app gets that app's document, and the app's router
 * shows the page. documents.test.ts holds nginx's rules against these.
 */

/** The web client's document, which answers every address outside /cockpit. */
export const WEB_DOCUMENT = "index.html";

/** Cockpit's document, which answers /cockpit and every address below it. */
export const COCKPIT_DOCUMENT = "cockpit/index.html";

/**
 * Where the build puts its own files, named by their content. Not `assets/`,
 * which the production server sends to the API for uploads.
 */
export const BUILD_DIRECTORY = "static";

/** Where the build writes the pre-paint scripts into the web client's document. */
export const PRE_PAINT_MARKER = "<!-- pre-paint -->";

export type DocumentName = typeof WEB_DOCUMENT | typeof COCKPIT_DOCUMENT;

/**
 * The directories that hold files and never pages: the top-level ones of
 * `public/` (worklets, codecs, models, fonts, sounds, identity art) and the
 * build's own.
 */
export function fileDirectories(publicDir: string): string[] {
  const served = readdirSync(publicDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name);
  return [...new Set([...served, BUILD_DIRECTORY])].sort();
}

/**
 * `pathname` with every run of slashes made one, as nginx's merge_slashes
 * makes `$uri`: `//cockpit//users/` is `/cockpit/users/`. One walk from start
 * to end, since anyone can write the address.
 */
export function mergeSlashes(pathname: string): string {
  let merged = "";
  let start = 0;
  for (let index = 0; index < pathname.length; index += 1) {
    if (pathname[index] === "/" && pathname[index + 1] === "/") {
      merged += pathname.slice(start, index);
      while (pathname[index + 1] === "/") index += 1;
      start = index;
    }
  }
  return merged + pathname.slice(start);
}

/**
 * The document that answers `pathname`, or null when the address names a
 * file, which is served as it is or not at all. A missing file is a 404,
 * never a page: a page in place of a worklet or a model would fail far from
 * the cause. An address names a file when it lies in one of `directories`, or
 * when its last segment has an extension, which no page address has. Runs of
 * slashes count as one, as they do for nginx.
 */
export function documentFor(pathname: string, directories: readonly string[]): DocumentName | null {
  const segments = mergeSlashes(pathname).split("/");
  const first = segments[1] ?? "";
  const last = segments.at(-1) ?? "";
  // `/@vite/client`, `/@fs/…` and the like belong to the dev server.
  if (first.startsWith("@") || directories.includes(first) || last.includes(".")) return null;
  return first === "cockpit" ? COCKPIT_DOCUMENT : WEB_DOCUMENT;
}

/**
 * Takes out of `outDir` the TypeScript declarations the build copied from
 * `publicDir` with the rest of it. One sits beside the voice activity
 * worklet (`audio/vad-processor.d.ts`) for the code that imports the
 * worklet's helpers; a browser has no use for it, so it does not ship.
 * Returns the files it removed, relative to `outDir`.
 */
export function removeDeclarations(publicDir: string, outDir: string): string[] {
  const declarations = readdirSync(publicDir, { recursive: true, encoding: "utf8" }).filter((file) =>
    file.endsWith(".d.ts"),
  );
  for (const file of declarations) rmSync(join(outDir, file), { force: true });
  return declarations;
}

/** Writes the pre-paint scripts into the web client's document at the marker. */
export function writePrePaint(html: string): string {
  if (!html.includes(PRE_PAINT_MARKER)) {
    throw new Error(`The web client's document has no ${PRE_PAINT_MARKER} for the pre-paint scripts.`);
  }
  // `</` would end the script element early; `<\/` means the same in JavaScript.
  const scripts = [LOCALE_SCRIPT, THEME_SCRIPT]
    .map((source) => `<script>${source.replaceAll("</", "<\\/")}</script>`)
    .join("\n    ");
  return html.replace(PRE_PAINT_MARKER, scripts);
}

/**
 * The path of a request's address, read as a path: `//login` is this
 * server's /login and not a host, and `//` is a path too, where reading it
 * against a base would throw. Null for a target that is no path at all.
 */
export function requestPath(url: string | undefined): string | null {
  try {
    return new URL(`http://localhost${url ?? "/"}`).pathname;
  } catch {
    return null;
  }
}

/** Answers every GET of an app address with that app's document. */
function serveDocuments(directories: readonly string[]): Connect.NextHandleFunction {
  return (request, _response, next) => {
    if (request.method !== "GET" && request.method !== "HEAD") return next();
    const pathname = requestPath(request.url);
    if (pathname === null) return next();
    const document = documentFor(pathname, directories);
    if (document !== null) request.url = `/${document}`;
    next();
  };
}

/** Answers the instance's default language, which the container writes in production. */
function serveInstanceLanguage(language: string | undefined): Connect.NextHandleFunction {
  return (request, response, next) => {
    if (requestPath(request.url) !== INSTANCE_LANGUAGE_PATH) return next();
    response.setHeader("Content-Type", "text/javascript; charset=utf-8");
    response.setHeader("Cache-Control", "no-cache");
    response.end(instanceLanguageSource(language));
  };
}

export interface DocumentsOptions {
  /** `STELLARR_LANGUAGE` of the server, which stands in for the container's file. */
  readonly instanceLanguage: string | undefined;
  /** The directory whose top-level folders hold only files. */
  readonly publicDir: string;
}

/**
 * The two documents in the dev and preview servers: the pre-paint scripts in
 * the web client's, each app's document for its addresses, and the instance
 * language file.
 */
export function stellarrDocuments(options: DocumentsOptions): Plugin {
  const directories = fileDirectories(options.publicDir);
  return {
    name: "stellarr-documents",
    transformIndexHtml: {
      order: "pre",
      handler: (html, context) => (context.path === `/${WEB_DOCUMENT}` ? writePrePaint(html) : html),
    },
    configureServer(server) {
      server.middlewares.use(serveInstanceLanguage(options.instanceLanguage));
      // After Vite's own middlewares have served what exists, before its 404.
      return () => server.middlewares.use(serveDocuments(directories));
    },
    configurePreviewServer(server) {
      server.middlewares.use(serveInstanceLanguage(options.instanceLanguage));
      return () => server.middlewares.use(serveDocuments(directories));
    },
    writeBundle(output) {
      if (output.dir !== undefined) removeDeclarations(options.publicDir, output.dir);
    },
  };
}
