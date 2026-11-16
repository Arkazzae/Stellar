import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
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
  PRE_PAINT_MARKER,
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
