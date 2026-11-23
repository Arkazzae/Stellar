import { matchFont } from "@/design/fontLabels";
import type { FontId, PaletteColors } from "@/design/themes";

import type { Evaluator } from "./evaluate";
import { DISCORD_VARIABLES, SHADING_VARIABLE, mapDiscordTheme, type RoleSource } from "./roles";
import {
  inlineImports,
  readMeta,
  styleQueryNames,
  unwrapStyleQueries,
  wallpaperAddressOf,
  withoutAddresses,
  type FetchStylesheet,
  type Inlined,
  type ThemeMeta,
} from "./text";

/** Where a theme's face is set, in the order a theme's own choice wins. */
const FONT_VARIABLES = ["--font", "--font-primary", "--font-display", "--font-headline"] as const;

export interface Conversion {
  meta: ThemeMeta;
  /** A full palette, only when the theme's backgrounds and text came across. */
  palette?: PaletteColors;
  accent?: string;
  /** Every role of the palette, with the Discord variable it came from or none. */
  roles: RoleSource[];
  /** The core roles that did not come across, when there is no palette. */
  missingCore: string[];
  /** The backgrounds are the theme's shading over its picture, not colours it set. */
  backgroundsFromShading?: boolean;
  /** The face the theme asks for, and ours when we have it. */
  font?: { family: string; id: FontId | null };
  /** A picture the theme puts behind Discord, which is fetched only if the person agrees. */
  wallpaper?: { url: string; host: string; shade?: number };
  /** How the engine dealt with container style queries. */
  styleQueries: "native" | "unwrapped" | "none";
  /** Style-query blocks that could not be decided without the engine. */
  undecided: number;
  skipped: Inlined["skipped"];
  fetched: string[];
}

export interface ConvertOptions {
  fetchSheet: FetchStylesheet;
  evaluator: Evaluator;
  /** Decide style queries here even where the engine can: to check the two agree. */
  forceUnwrap?: boolean;
}

/** The first family of a `font-family` list, without quotes. */
export function firstFamily(value: string): string {
  return (value.split(",")[0] ?? "").trim().replace(/^["']|["']$/g, "").trim();
}

/**
 * A BetterDiscord theme as what Stellarr can take from it. The stylesheet is
 * read with its imports, stripped of every address, and evaluated in a
 * document of its own (`evaluate.ts`); Discord's variables are then read
 * back as colours and mapped onto our roles (`roles.ts`). Where the engine
 * cannot evaluate a container style query, the blocks the theme gates its
 * colours behind are decided against the values the theme itself declares
 * and evaluated again.
 */
export async function convertTheme(text: string, from: string | null, options: ConvertOptions): Promise<Conversion> {
  const meta = readMeta(text);
  const inlined = await inlineImports(text, from, options.fetchSheet);
  const picture = wallpaperAddressOf(inlined.css);
  const clean = withoutAddresses(inlined.css);
  const queried = styleQueryNames(clean);
  const valueNames = [...queried, ...FONT_VARIABLES];

  let evaluation = await options.evaluator.evaluate(clean, DISCORD_VARIABLES, valueNames);
  let styleQueries: Conversion["styleQueries"] = queried.length === 0 ? "none" : "native";
  let undecided = 0;
  if (queried.length > 0 && (!evaluation.styleQueries || options.forceUnwrap === true)) {
    const first = evaluation;
    const unwrapped = unwrapStyleQueries(clean, (name, container) =>
      (container === "body" ? first.containers.body : first.containers.root).get(name) ?? first.values.get(name),
    );
    undecided = unwrapped.undecided;
    evaluation = await options.evaluator.evaluate(unwrapped.css, DISCORD_VARIABLES, valueNames);
    styleQueries = "unwrapped";
  }

  const mapped = mapDiscordTheme(evaluation.colours, picture !== null);
  const declared = FONT_VARIABLES.map((name) => evaluation.values.get(name)).find((value) => value !== undefined && firstFamily(value) !== "");
  const family = declared === undefined ? inlined.googleFonts[0] : firstFamily(declared);
  const shading = evaluation.colours.get(SHADING_VARIABLE);

  return {
    meta,
    ...(mapped.colors === undefined ? {} : { palette: mapped.colors }),
    ...(mapped.accent === undefined ? {} : { accent: mapped.accent }),
    roles: mapped.roles,
    missingCore: mapped.missingCore,
    ...(mapped.backgroundsFromShading === true ? { backgroundsFromShading: true } : {}),
    ...(family === undefined ? {} : { font: { family, id: matchFont(family) } }),
    ...(picture === null
      ? {}
      : {
          wallpaper: {
            url: picture,
            host: new URL(picture).host,
            ...(shading === undefined ? {} : { shade: Math.round(shading[3] * 100) }),
          },
        }),
    styleQueries,
    undecided,
    skipped: inlined.skipped,
    fetched: inlined.fetched,
  };
}

/** What may be fetched for a theme, and how much of it. */
export const FETCH_TIMEOUT_MS = 10_000;

/**
 * Reads an address the way a theme's parts are read: https only, without
 * cookies or a referrer, within a time and a size, and failing on anything
 * but a success.
 */
export async function fetchCapped(url: string, maxBytes: number): Promise<Blob> {
  if (new URL(url).protocol !== "https:") throw new Error("Only https addresses are read.");
  const response = await fetch(url, {
    credentials: "omit",
    referrerPolicy: "no-referrer",
    mode: "cors",
    signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
  });
  if (!response.ok || response.body === null) throw new Error(`${url} answered ${response.status}.`);
  const declared = Number(response.headers.get("content-length") ?? "0");
  if (declared > maxBytes) throw new Error(`${url} is larger than ${maxBytes} bytes.`);
  const reader = response.body.getReader();
  const parts: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > maxBytes) {
      await reader.cancel();
      throw new Error(`${url} is larger than ${maxBytes} bytes.`);
    }
    parts.push(value);
  }
  return new Blob(parts as BlobPart[], { type: response.headers.get("content-type") ?? "" });
}

/**
 * An address a person pasted, as the stylesheet to read: a GitHub page of a
 * file becomes the file itself. betterdiscord.app does not let another site
 * read its downloads, which is said rather than tried.
 */
export function stylesheetAddress(input: string): { url: string } | { problem: "notHttps" | "betterDiscordSite" | "invalid" } {
  let url: URL;
  try {
    url = new URL(input.trim());
  } catch {
    return { problem: "invalid" };
  }
  if (url.protocol !== "https:") return { problem: "notHttps" };
  if (url.hostname === "betterdiscord.app" || url.hostname.endsWith(".betterdiscord.app")) {
    return { problem: "betterDiscordSite" };
  }
  const blob = /^\/([^/]+)\/([^/]+)\/blob\/(.+)$/.exec(url.pathname);
  if (url.hostname === "github.com" && blob !== null) {
    return { url: `https://raw.githubusercontent.com/${blob[1]}/${blob[2]}/${blob[3]}` };
  }
  return { url: url.href };
}
