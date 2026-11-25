import { matchFont } from "@/design/fontLabels";
import type { FontId, PaletteColors } from "@/design/themes";

import type { DiscordVariant, Evaluator } from "./evaluate";
import { DISCORD_VARIABLES, SHADING_VARIABLES, mapDiscordTheme, type MappedTheme, type RoleSource } from "./roles";
import {
  inlineImports,
  readMeta,
  styleQueryNames,
  unwrapStyleQueries,
  wallpaperValues,
  type FetchStylesheet,
  type Inlined,
  type ThemeMeta,
} from "./text";

/** Where a theme's face is set, in the order a theme's own choice wins. */
const FONT_VARIABLES = ["--font", "--app-font", "--font-primary", "--font-display", "--font-headline", "--dplus-font-ui", "--main-font", "--stellarr-font"] as const;

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
  wallpaper?: { url: string; host?: string; shade?: number };
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

function hasDeclaredSurface(theme: MappedTheme): boolean {
  return theme.roles.some(({ group, role, from }) =>
    group === "backgrounds" && ["primary", "darkest", "secondary"].includes(role) &&
    from !== undefined && from !== "--custom-theme-base-color" && !from.startsWith("--stellarr-"),
  );
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
  const wallpaper = wallpaperValues(inlined.css);
  const clean = wallpaper.css;
  const queried = styleQueryNames(clean);
  const valueNames = [...queried, ...FONT_VARIABLES, ...wallpaper.names];

  const evaluate = async (variant: DiscordVariant) => {
    let evaluation = await options.evaluator.evaluate(clean, DISCORD_VARIABLES, valueNames, variant);
    let styleQueries: Conversion["styleQueries"] = queried.length === 0 ? "none" : "native";
    let undecided = 0;
    if (queried.length > 0 && (!evaluation.styleQueries || options.forceUnwrap === true)) {
      const first = evaluation;
      const unwrapped = unwrapStyleQueries(clean, (name, container) =>
        (container === "body" ? first.containers.body : first.containers.root).get(name) ?? first.values.get(name),
      );
      undecided = unwrapped.undecided;
      evaluation = await options.evaluator.evaluate(unwrapped.css, DISCORD_VARIABLES, valueNames, variant);
      styleQueries = "unwrapped";
    }
    const picture = wallpaper.read(evaluation.values);
    return { evaluation, picture, mapped: mapDiscordTheme(evaluation.colours, picture !== null), styleQueries, undecided };
  };

  let result = await evaluate("dark");
  for (const variant of ["darker", "midnight", "light"] as const) {
    if (result.picture !== null || hasDeclaredSurface(result.mapped)) break;
    if (!clean.includes(`.theme-${variant}`)) continue;
    const candidate = await evaluate(variant);
    if (candidate.mapped.colors !== undefined && (result.mapped.colors === undefined || hasDeclaredSurface(candidate.mapped))) {
      result = candidate;
    }
  }
  const { evaluation, picture, mapped, styleQueries, undecided } = result;
  const stack = FONT_VARIABLES.map((name) => evaluation.values.get(name))
    .find((value) => value !== undefined && firstFamily(value) !== "");
  const families = stack === undefined ? inlined.googleFonts : stack.split(",").map(firstFamily);
  const family = families.find((name) => matchFont(name) !== null) ?? families[0];
  const shading = SHADING_VARIABLES.map((name) => evaluation.colours.get(name)).find((value) => value !== undefined);

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
            ...(picture.startsWith("https://") ? { host: new URL(picture).host } : {}),
            ...(shading === undefined ? {} : { shade: Math.round(shading[3] * 100) }),
          },
        }),
    styleQueries,
    undecided,
    skipped: inlined.skipped,
    fetched: inlined.fetched,
  };
}
