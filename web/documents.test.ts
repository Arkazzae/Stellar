import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import type * as Router from "@stellarr/router";
import type { Route } from "@stellarr/router";
import { describe, expect, it, vi } from "vitest";

import {
  BUILD_DIRECTORY,
  COCKPIT_DOCUMENT,
  documentFor,
  fileDirectories,
  mergeSlashes,
  PRE_PAINT_MARKER,
  requestPath,
  WEB_DOCUMENT,
  writePrePaint,
} from "./documents";
import { THEME_SCRIPT } from "./src/stellarr/design/themeScript";
import { SUPPORTED_LOCALES, translateMessage } from "./src/stellarr/i18n";
import { INSTANCE_LANGUAGE_PATH, instanceLanguageSource, LOCALE_SCRIPT } from "./src/stellarr/i18n/localeScript";
import { routes } from "./src/stellarr/routes";

// The route table keeps its paths to itself, so the tables handed to
// defineRoutes are kept here, for the check against infra/nginx.conf.
const tables = vi.hoisted(() => [] as (readonly Route[])[]);
vi.mock("@stellarr/router", async (importOriginal) => {
  const router = await importOriginal<typeof Router>();
  return {
    ...router,
    defineRoutes: (table: Parameters<typeof router.defineRoutes>[0]) => {
      tables.push(table.routes);
      return router.defineRoutes(table);
    },
  };
});

const here = (path: string) => new URL(path, import.meta.url);
const read = (path: string) => readFileSync(here(path), "utf8");
const directories = fileDirectories(fileURLToPath(here("public")));

describe("the addresses each document answers", () => {
  it("gives every address of the web client its document", () => {
    for (const path of ["/", "/login", "/reset-password", "/invite/abc123", "/invite/%E0%A4%A", "/nope", "/login/"]) {
      expect(documentFor(path, directories), path).toBe(WEB_DOCUMENT);
    }
  });

  it("counts runs of slashes as one, as nginx does, in both apps", () => {
    for (const path of ["//", "//login", "//register", "//nope", "///invite//abc123", "//login/"]) {
      expect(documentFor(path, directories), path).toBe(WEB_DOCUMENT);
    }
    for (const path of ["//cockpit", "//cockpit//users", "/cockpit//users", "///cockpit/"]) {
      expect(documentFor(path, directories), path).toBe(COCKPIT_DOCUMENT);
    }
    expect(documentFor("//audio//missing.js", directories)).toBeNull();
    expect(mergeSlashes("//cockpit//users//")).toBe("/cockpit/users/");
    expect(mergeSlashes("/")).toBe("/");
  });

  it("reads a request's address as a path, however many slashes lead it", () => {
    expect(requestPath("//")).toBe("//");
    expect(requestPath("//login?x=1")).toBe("//login");
    expect(requestPath("///cockpit//users")).toBe("///cockpit//users");
    expect(requestPath("/stellarr-instance.js")).toBe("/stellarr-instance.js");
    // A target that makes no address is left to the server's own handling.
    expect(requestPath("@")).toBeNull();
  });

  it("merges a long run of slashes in time linear in its length", () => {
    const run = "/".repeat(300_000);
    const started = performance.now();

    expect(mergeSlashes(`${run}login`)).toBe("/login");
    expect(documentFor(`${run}cockpit${run}users`, directories)).toBe(COCKPIT_DOCUMENT);

    expect(performance.now() - started).toBeLessThan(250);
  });

  it("gives /cockpit and everything below it Cockpit's document", () => {
    for (const path of ["/cockpit", "/cockpit/", "/cockpit/users", "/cockpit/nope/deeper"]) {
      expect(documentFor(path, directories), path).toBe(COCKPIT_DOCUMENT);
    }
    expect(documentFor("/cockpits", directories)).toBe(WEB_DOCUMENT);
  });

  it("never answers a file with a document, so a missing one is a 404", () => {
    for (const path of [
      "/audio/missing-worklet.js",
      "/codecs/missing.wasm",
      "/bergamot-models/plen/missing",
      "/fonts/missing",
      "/sounds/missing",
      "/identity/defaults/missing",
      `/${BUILD_DIRECTORY}/missing`,
      "/missing.js",
      "/cockpit/missing.css",
      "/@vite/client",
    ]) {
      expect(documentFor(path, directories), path).toBeNull();
    }
  });

  it("counts every top-level folder of public/ and the build's own as files", () => {
    expect(directories).toEqual(expect.arrayContaining(["audio", "bergamot-models", "fonts", "identity", "sounds", "static"]));
    expect(BUILD_DIRECTORY).not.toBe("assets");
  });
});

describe("the web client's document", () => {
  const html = writePrePaint(read("index.html"));
  const head = html.slice(0, html.indexOf("</head>"));

  it("runs the instance file, the locale script and the theme script in <head>, in that order", () => {
    const instance = head.indexOf(`<script src="${INSTANCE_LANGUAGE_PATH}"`);
    const locale = head.indexOf(LOCALE_SCRIPT);
    const theme = head.indexOf(THEME_SCRIPT);

    expect(instance).toBeGreaterThan(0);
    expect(locale).toBeGreaterThan(instance);
    expect(theme).toBeGreaterThan(locale);
    expect(html).not.toContain(PRE_PAINT_MARKER);
  });

  it("puts the description before the script that translates it", () => {
    expect(head.indexOf('<meta name="description"')).toBeLessThan(head.indexOf(LOCALE_SCRIPT));
    expect(head).toContain(`content="${translateMessage("en", "app.description")}"`);
  });

  it("refuses a document that lost its marker", () => {
    expect(() => writePrePaint("<head></head>")).toThrow(/pre-paint/);
  });

  it("carries the metadata the app had", () => {
    expect(head).toContain("<title>Stellarr</title>");
    expect(head).toContain('<meta name="robots" content="noindex, nofollow" />');
    expect(head).toContain('<meta name="theme-color" content="#111111" />');
    expect(head).toContain("width=device-width, initial-scale=1, maximum-scale=5, viewport-fit=cover");
    expect(head).toContain('<link rel="icon" href="/src/stellarr/assets/icon.svg"');
  });
});

describe("Cockpit's document", () => {
  const html = read(COCKPIT_DOCUMENT);

  it("carries none of the web client's scripts", () => {
    expect(html).not.toContain(PRE_PAINT_MARKER);
    expect(html).not.toContain(INSTANCE_LANGUAGE_PATH);
    expect(html).toContain('<script type="module" src="/src/cockpit/main.tsx"></script>');
  });

  it("carries the metadata the panel had", () => {
    expect(html).toContain("<title>Stellarr Cockpit</title>");
    expect(html).toContain('<meta name="robots" content="noindex, nofollow" />');
    expect(html).toContain('media="(prefers-color-scheme: dark)" content="#1b1b1f"');
  });
});

interface MapEntry {
  readonly key: string;
  readonly value: string;
}

/** The entries of `map $uri $stellarr_address` in infra/nginx.conf, in their order. */
function addressMap(config: string): MapEntry[] {
  const block = /map \$uri \$stellarr_address \{\n([\s\S]*?)\n\s*\}/.exec(config)?.[1];
  if (block === undefined) throw new Error("infra/nginx.conf has no map $uri $stellarr_address.");
  return [...block.matchAll(/^\s*("[^"]*"|[^\s#]\S*)\s+(\S+);/gm)].map(([, key = "", value = ""]) => ({
    key: key.replace(/^"(.*)"$/, "$1"),
    value,
  }));
}

/** What nginx's map gives `uri`: an exact key first, then the first regex that matches, then the default. */
function lookUp(entries: readonly MapEntry[], uri: string): string | undefined {
  const exact = entries.find(({ key }) => key !== "default" && !key.startsWith("~") && key === uri);
  const regex = entries.find(({ key }) => key.startsWith("~") && new RegExp(key.slice(1)).test(uri));
  return (exact ?? regex ?? entries.find(({ key }) => key === "default"))?.value;
}

/** A route's path with every parameter as `:`, the form both lists are compared in. */
const routeShape = (path: string) => path.replace(/:[A-Za-z_]\w*/g, ":");

/** A page's regex in nginx's map in that form: `~^/invite/[^/]+/?$` is `/invite/:`. */
const nginxShape = (key: string) =>
  key.replace(/^~\^/, "").replace(/\$$/, "").replace(/\/\?$/, "").replaceAll("[^/]+", ":");

/** Addresses of that form, with and without a trailing slash. */
function addressesOf(shape: string): string[] {
  const address = shape.replaceAll(":", "abc123");
  return address === "/" ? [address] : [address, `${address}/`];
}

/** What answers an address that names no file, as documents.ts and the web client's routes decide. */
function expectedAnswer(address: string): string {
  const document = documentFor(address, directories);
  if (document === null) return "file";
  if (document === COCKPIT_DOCUMENT) return "cockpit";
  return routes.claims(address) ? "page" : "missing";
}

describe("infra/nginx.conf, which serves the build in the container", () => {
  const entries = addressMap(read("../infra/nginx.conf"));
  const pages = entries.filter(({ value }) => value === "page").map(({ key }) => nginxShape(key));
  const routePaths = tables.flatMap((table) => table.map(({ path }) => routeShape(path)));

  it("lists the web client's addresses as its route table does", () => {
    expect(tables).toHaveLength(1);
    expect(routePaths.length).toBeGreaterThan(0);
    expect([...pages].sort()).toEqual([...routePaths].sort());
  });

  it("counts as files the folders documents.ts does", () => {
    expect(entries).toContainEqual({ key: `~^/(${directories.join("|")})(/|$)`, value: "file" });
  });

  it("answers every address that names no file as documents.ts and the route table do", () => {
    const addresses = [
      ...[...pages, ...routePaths].flatMap(addressesOf),
      ...directories.flatMap((directory) => [`/${directory}`, `/${directory}/`, `/${directory}/missing`]),
      "/cockpit",
      "/cockpit/",
      "/cockpit/users",
      "/cockpit/nope/deeper",
      "/cockpit/missing.css",
      "/cockpits",
      "/nope",
      "/nope/",
      "/invite",
      "/invite/abc123/more",
      "/invite/abc.123",
      "/login/more",
      "/missing.js",
    ];
    for (const address of addresses) {
      expect(lookUp(entries, address), address).toBe(expectedAnswer(address));
    }
  });

  it("counts an address as a file by a dot in its last segment only", () => {
    const cases: [address: string, answer: string][] = [
      ["/missing.js", "file"],
      ["/web-abc.js.gz", "file"],
      ["/static/web-abc.js.gz", "file"],
      ["/a.b.c", "file"],
      ["/nope/a..b", "file"],
      ["/a.", "file"],
      ["/nope/b.", "file"],
      // A dot in an earlier segment only, or a trailing slash after one.
      ["/v1.2/users", "missing"],
      ["/a.b/", "missing"],
      ["/a.b.c/", "missing"],
      ["/invite/abc.123/", "page"],
    ];
    for (const [address, answer] of cases) {
      expect(lookUp(entries, address), address).toBe(answer);
      expect(expectedAnswer(address), address).toBe(answer);
    }
  });

  // [^/]* after the dot would rescan the rest of the segment from every dot:
  // 50,000 dots before a slash took seven seconds to match.
  // nginx merges the slashes of an address (merge_slashes) before the map
  // sees $uri, and the router merges them before it picks the page, so an
  // address that starts with two slashes gets the page it names.
  it("answers an address with runs of slashes as nginx does once it merged them", () => {
    for (const address of [
      "//",
      "//login",
      "//register",
      "//nope",
      "//login/",
      "///invite//abc123",
      "//cockpit",
      "//cockpit//users",
      "/cockpit//users",
      "//missing.js",
      "//audio//missing",
    ]) {
      expect(lookUp(entries, mergeSlashes(address)), address).toBe(expectedAnswer(address));
    }
    expect(expectedAnswer("//login")).toBe("page");
    expect(expectedAnswer("//nope")).toBe("missing");
    expect(expectedAnswer("//cockpit//users")).toBe("cockpit");
  });

  it("answers a long run of dots before a slash in time linear in its length", () => {
    const address = `/${".".repeat(50_000)}/`;
    const started = performance.now();

    expect(lookUp(entries, address)).toBe("missing");
    expect(expectedAnswer(address)).toBe("missing");

    expect(performance.now() - started).toBeLessThan(250);
  });
});

describe("the instance language file the container writes", () => {
  const entrypoint = read("../infra/entrypoint.sh");
  const writer = /^write_instance_language\(\) \{\n[\s\S]*?\n\}\n/m.exec(entrypoint)?.[0];

  /** Runs the entrypoint's writer under bash and returns the file it wrote. */
  function written(language: string | undefined): string {
    if (writer === undefined) throw new Error("infra/entrypoint.sh has no write_instance_language.");
    const directory = mkdtempSync(join(tmpdir(), "stellarr-instance-"));
    try {
      const file = join(directory, "stellarr-instance.js");
      const env = { ...process.env };
      delete env.STELLARR_LANGUAGE;
      if (language !== undefined) env.STELLARR_LANGUAGE = language;
      execFileSync("bash", ["-c", `${writer}\nwrite_instance_language "$1"`, "bash", file], { env, stdio: "ignore" });
      return readFileSync(file, "utf8");
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }

  it("is written where nginx serves it from", () => {
    expect(entrypoint).toContain(`write_instance_language /run/stellarr${INSTANCE_LANGUAGE_PATH}\n`);
    expect(read("../infra/nginx.conf")).toMatch(
      new RegExp(`location = ${INSTANCE_LANGUAGE_PATH.replaceAll(".", "\\.")} \\{\\s*root /run/stellarr;`),
    );
  });

  it.each([...SUPPORTED_LOCALES, undefined, "", "de", "EN", " pl", 'pl";alert(1);"', "</script>"])(
    "holds the bytes of instanceLanguageSource for STELLARR_LANGUAGE=%j",
    (language) => {
      expect(written(language)).toBe(instanceLanguageSource(language));
    },
  );
});

/** The files of a source tree, test code left out. */
function sourceFiles(directory: string): string[] {
  return readdirSync(here(directory), { recursive: true, encoding: "utf8" })
    .filter((file) => /\.(ts|tsx|css)$/.test(file) && !/\.test\.tsx?$/.test(file) && !/(^|\/)test\//.test(file))
    .map((file) => `${directory}/${file}`);
}

describe("the files the apps name by address", () => {
  // A root-relative address in a string, a url() or an attribute, with the
  // extension of a file. The same sweep over the built files found /icon.svg,
  // which Next served from src/app/ and the static build does not have.
  const ADDRESS = /["'`(]\/[\w@./-]+\.(?:svg|png|jpe?g|webp|gif|ico|m?js|css|wasm|json|wav|mp3|ogg|opus|woff2?|ttf|bin|spm|txt|webmanifest|map|html)\b/g;
  const manifest = JSON.parse(read("../artifacts/manifest.json")) as { artifacts: { path: string }[] };
  const assembled = new Set(manifest.artifacts.map(({ path }) => `/${path}`));
  const sources = [
    ...sourceFiles("src"),
    ...["stellarr-ui", "stellarr-icons", "stellarr-router"].flatMap((name) => sourceFiles(`../packages/${name}/src`)),
    "index.html",
    "cockpit/index.html",
    "public/bergamot-worker.js",
    "public/audio/vad-processor.js",
  ];
  const named = new Map<string, string>();
  for (const source of sources) {
    for (const [match] of read(source).matchAll(ADDRESS)) named.set(match.slice(1), source);
  }

  /** Whether the instance serves `address`: a file of public/ or of the artifacts, the instance file, or a source the build turns into a file of its own. */
  function served(address: string): boolean {
    if (address === INSTANCE_LANGUAGE_PATH) return true;
    if (address.startsWith("/src/")) return existsSync(here(`.${address}`));
    return assembled.has(address) || existsSync(here(`public${address}`));
  }

  it("finds the addresses the apps load files from", () => {
    expect([...named.keys()]).toEqual(
      expect.arrayContaining(["/audio/vad-processor.js", "/sounds/calling.wav", "/fonts/outfit-100-900-latin.woff2"]),
    );
  });

  it("names only files the build or the instance serves", () => {
    const missing = [...named].filter(([address]) => !served(address)).map(([address, source]) => `${address} in ${source}`);
    expect(missing).toEqual([]);
  });
});
