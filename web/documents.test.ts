import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

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
import { translateMessage } from "./src/stellarr/i18n";
import { INSTANCE_LANGUAGE_PATH, LOCALE_SCRIPT } from "./src/stellarr/i18n/localeScript";

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
