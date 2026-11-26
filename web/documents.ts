import { readdirSync } from "node:fs";

import type { Connect, Plugin, Rollup } from "vite";

import { THEME_SCRIPT } from "./src/stellarr/design/themeScript";
import { mapLocales, SUPPORTED_LOCALES, type SupportedLocale } from "./src/i18n/locales";
import { LANGUAGE_SETTINGS_KEY } from "./src/stellarr/i18n/defaultLocale";
import {
  INSTANCE_LANGUAGE_PATH,
  instanceLanguageSource,
  LANGUAGE_HINT_KEY,
  LOCALE_SCRIPT,
} from "./src/stellarr/i18n/localeScript";
import { WORKLETS } from "./worklets";

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
 * `public/` (worklets, codecs, models, fonts, sounds, identity and mascot art) and the
 * build's own, among them the one it writes the app's worklets to.
 */
export function fileDirectories(publicDir: string): string[] {
  const served = readdirSync(publicDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name);
  const worklets = Object.values(WORKLETS).map((file) => file.split("/")[0] ?? file);
  return [...new Set([...served, BUILD_DIRECTORY, ...worklets])].sort();
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

/**
 * Where the build writes the two preload scripts: the first ahead of the
 * instance file, the second after the locale script.
 */
export const PRELOAD_MARKER = "<!-- preload -->";
export const PRELOAD_CHOSEN_MARKER = "<!-- preload-chosen -->";

/** Each language's catalog, by its path in the source; each is a chunk of its own. */
export const CATALOG_MODULES = mapLocales((locale) => `.locales/web/${locale}.json`);

/** The files the document fetches ahead of the entry's own requests for them. */
export interface PreloadPlan {
  /** Per language, its catalog's chunk and whatever it imports that the entry does not. */
  catalogs: Record<SupportedLocale, string[]>;
}

/** A chunk and every chunk it imports, directly or not. */
export function staticClosure(chunks: ReadonlyMap<string, Rollup.OutputChunk>, fileName: string, into = new Set<string>()): Set<string> {
  if (into.has(fileName)) return into;
  into.add(fileName);
  for (const imported of chunks.get(fileName)?.imports ?? []) staticClosure(chunks, imported, into);
  return into;
}

/**
 * Reads from the bundle which files hold the catalogs, leaving out what the
 * entry already loads. It refuses a build in which a catalog is part of the
 * entry's own files: then the split this plan exists for has been lost, and
 * the entry carries both catalogs again.
 */
export function preloadPlan(bundle: Rollup.OutputBundle, entry: string, base = "/"): PreloadPlan {
  const chunks = new Map(
    Object.values(bundle)
      .filter((output): output is Rollup.OutputChunk => output.type === "chunk")
      .map((chunk) => [chunk.fileName, chunk]),
  );
  const loaded = staticClosure(chunks, entry);
  const filesOf = (path: string): string[] => {
    const chunk = [...chunks.values()].find((candidate) => candidate.facadeModuleId?.endsWith(`/${path}`) === true);
    if (chunk === undefined) throw new Error(`The build has no chunk of its own for ${path}.`);
    if (loaded.has(chunk.fileName)) {
      throw new Error(`${path} is in ${chunk.fileName}, which the entry loads anyway; it should be a chunk of its own.`);
    }
    return [...staticClosure(chunks, chunk.fileName)].filter((file) => !loaded.has(file)).map((file) => `${base}${file}`);
  };
  const catalogs = mapLocales((locale) => filesOf(CATALOG_MODULES[locale]));
  const files = Object.values(catalogs).flat();
  if (new Set(files).size !== files.length) {
    throw new Error("Language catalogs share a chunk, so a page would fetch more than its language.");
  }
  return { catalogs };
}

/**
 * Adds a `modulepreload` link for `href` unless one is there already. The
 * entry's own `import()` of the module then takes the one already fetched.
 */
const PRELOAD_FUNCTION =
  'function preload(href){if(document.querySelector(\'link[rel="modulepreload"][href="\'+href+\'"]\'))return;' +
  'var link=document.createElement("link");link.rel="modulepreload";link.setAttribute("crossorigin","");link.href=href;document.head.appendChild(link)}';

/**
 * Runs first in `<head>`, ahead of the instance file, which blocks the parser
 * for a round trip while the browser already fetches the entry. When this
 * browser can tell the page's language already, from the person's stored
 * choice or else the hint the locale script left on the last load, it asks
 * for that catalog now, next to the entry.
 *
 * The guess is the locale script's own answer except on the one load after an
 * operator changes the instance default; then the next script fetches the
 * right catalog too.
 */
export function earlyPreloadScript(plan: PreloadPlan): string {
  return `(function(){
var CATALOGS=${JSON.stringify(plan.catalogs)},SUPPORTED=${JSON.stringify(SUPPORTED_LOCALES)};
${PRELOAD_FUNCTION}
function pick(value){return SUPPORTED.indexOf(value)<0?null:value}
var guess=null;
try{var raw=localStorage.getItem(${JSON.stringify(LANGUAGE_SETTINGS_KEY)});if(raw)guess=pick(JSON.parse(raw).locale);if(!guess)guess=pick(localStorage.getItem(${JSON.stringify(LANGUAGE_HINT_KEY)}))}catch(e){}
if(guess)CATALOGS[guess].forEach(preload);
})()`;
}

/**
 * Runs after the locale script and asks for the catalog of the language it
 * chose, which on a browser's first load could not be told in advance. When
 * the first script guessed right, the link is there and nothing more is asked.
 */
export function chosenPreloadScript(plan: PreloadPlan): string {
  return `(function(){
var CATALOGS=${JSON.stringify(plan.catalogs)};
${PRELOAD_FUNCTION}
(CATALOGS[document.documentElement.lang]||[]).forEach(preload);
})()`;
}

const inline = (source: string) => `<script>${source.replaceAll("</", "<\\/")}</script>`;

/**
 * Writes the two preload scripts at their markers. The dev server has no
 * bundle to read and serves modules one by one anyway, so it gets none
 * (`plan` null).
 */
export function writePreload(html: string, plan: PreloadPlan | null): string {
  for (const marker of [PRELOAD_MARKER, PRELOAD_CHOSEN_MARKER]) {
    if (!html.includes(marker)) {
      throw new Error(`The web client's document has no ${marker} for the preload scripts.`);
    }
  }
  return html
    .replace(PRELOAD_MARKER, plan === null ? "" : inline(earlyPreloadScript(plan)))
    .replace(PRELOAD_CHOSEN_MARKER, plan === null ? "" : inline(chosenPreloadScript(plan)));
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
 * language file. Once the build knows its files, the web client's document
 * also gets the preload scripts.
 */
export function stellarrDocuments(options: DocumentsOptions): Plugin[] {
  const directories = fileDirectories(options.publicDir);
  let base = "/";
  return [
    {
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
    },
    {
      name: "stellarr-preload",
      configResolved(config) {
        base = config.base;
      },
      transformIndexHtml: {
        // After the bundle is written, when the chunks' names are known.
        order: "post",
        handler: (html, context) => {
          if (context.path !== `/${WEB_DOCUMENT}`) return html;
          const { bundle, chunk } = context;
          return writePreload(html, bundle === undefined || chunk === undefined ? null : preloadPlan(bundle, chunk.fileName, base));
        },
      },
    },
  ];
}
