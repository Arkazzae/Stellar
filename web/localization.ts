// Split the translator's files into the pieces each browser entry actually needs.
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { Plugin } from "vite";

const directory = fileURLToPath(new URL("../locales", import.meta.url));
const output = fileURLToPath(new URL("./.locales/", import.meta.url));
type Strings = Record<string, string>;
type Catalog = { meta: Record<string, unknown>; web: Strings; cockpit: Strings; api: Strings };

function readCatalog(code: string): Catalog {
  const file = join(directory, code + ".json");
  let source: Catalog;
  try {
    source = JSON.parse(readFileSync(file, "utf8")) as Catalog;
  } catch (cause) {
    throw new Error(`Cannot read ${file}`, { cause });
  }
  for (const section of ["web", "cockpit", "api"] as const) {
    const entries = Object.entries(source[section]).filter(([key]) => !key.startsWith("_context."));
    for (const [key, text] of entries) {
      if (typeof text !== "string" || !text.trim()) throw new Error(`${code}.${section}.${key} needs text`);
    }
    source[section] = Object.fromEntries(entries);
  }
  Intl.getCanonicalLocales(code);
  if (typeof source.meta?.tag !== "string") throw new Error(`${code}.meta.tag needs a formatting locale`);
  Intl.getCanonicalLocales(source.meta.tag);
  return source;
}

function placeholders(value: string): string {
  return [...new Set(value.match(/\{[^}]+\}/g) ?? [])].sort().join(" ");
}

// Check metadata fields as well as strings, including the copy shown when a catalog fails to load.
function validate(source: Record<string, unknown>, translation: Record<string, unknown>, path: string): void {
  for (const key of new Set([...Object.keys(source), ...Object.keys(translation)])) {
    const expected = source[key];
    const actual = translation[key];
    if (typeof expected === "string" && typeof actual === "string" && actual.trim()) {
      if (placeholders(expected) !== placeholders(actual)) throw new Error(`${path}.${key}: different placeholders from English`);
    } else if (expected && actual && typeof expected === "object" && typeof actual === "object") {
      validate(expected as Record<string, unknown>, actual as Record<string, unknown>, path + "." + key);
    } else {
      throw new Error(`${path}.${key}: missing, extra or invalid catalog entry`);
    }
  }
}

function writeJson(file: string, value: unknown): boolean {
  const content = JSON.stringify(value) + "\n";
  if (existsSync(file) && readFileSync(file, "utf8") === content) return false;
  const temporary = `${file}.${randomUUID()}.tmp`;
  writeFileSync(temporary, content);
  renameSync(temporary, file);
  return true;
}

export function compileLocales(): boolean {
  const files = readdirSync(directory).filter((name) => name.endsWith(".json")).sort();
  const available = files.map((file) => basename(file, ".json"));
  const first = ["en", "pl", "de"].filter((code) => available.includes(code));
  const codes = new Set([...first, ...available]);
  const english = readCatalog("en");
  const meta: Record<string, Catalog["meta"]> = {};
  const cockpit: Record<string, Strings> = {};
  const webDir = join(output, "web");
  const web = new Map<string, Strings>();
  for (const code of codes) {
    if (!/^[a-z]{2,3}(?:-[A-Za-z0-9]{1,8})*$/.test(code)) throw new Error(`Invalid locale filename: ${code}.json`);
    const catalog = code === "en" ? english : readCatalog(code);
    validate(english, catalog, code);
    meta[code] = catalog.meta;
    cockpit[code] = catalog.cockpit;
    web.set(code, catalog.web);
  }
  mkdirSync(webDir, { recursive: true });
  for (const [code, strings] of web) writeJson(join(webDir, code + ".json"), strings);
  const metaChanged = writeJson(join(output, "meta.json"), meta);
  writeJson(join(output, "cockpit.json"), cockpit);
  for (const file of readdirSync(webDir)) {
    if (file.endsWith(".json") && !codes.has(basename(file, ".json"))) rmSync(join(webDir, file), { force: true });
  }
  return metaChanged;
}

/** Regenerate on edits in the dev server and in Vitest watch mode. */
export function watchLocales(): Plugin {
  return {
    name: "stellarr-locales",
    configureServer(server) {
      server.watcher.add(directory);
      server.watcher.on("all", (event, file) => {
        if (!["add", "change", "unlink"].includes(event) || dirname(file) !== directory || !file.endsWith(".json")) return;
        try {
          if (compileLocales()) void server.restart();
        } catch (cause) {
          const error = cause instanceof Error ? cause : new Error(String(cause));
          server.config.logger.error(error.message);
          server.ws.send({ type: "error", err: { message: error.message, stack: error.stack ?? "" } });
        }
      });
    },
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) compileLocales();
