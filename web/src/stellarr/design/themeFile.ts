import { makeCustomTheme } from "./customTheme";
import {
  CUSTOM_PALETTE,
  FONTS,
  getPalette,
  normaliseColours,
  normaliseCustomTheme,
  normaliseHex,
  resolvePalette,
  type CustomTheme,
  type FontId,
  type PaletteColors,
  type PaletteId,
  type ThemePreference,
} from "./themes";

/**
 * Stellarr's own theme file, `<name>.stellarr-theme.json`:
 *
 *   { "stellarrTheme": 1, "name": …, "author"?: …, "source"?: …,
 *     "colors": { backgrounds, text, borders, semantic }, "accent": …,
 *     "font"?: … }
 *
 * `stellarrTheme` only says the file is ours. A file carries what a theme
 * looks like and nothing personal: size, density, contrast and reduced motion
 * are the reader's, so they are never written or read. Reading goes through
 * the same normaliser as storage and the account.
 */

export const THEME_FILE_MARK = "stellarrTheme";
export const THEME_FILE_SUFFIX = ".stellarr-theme.json";
/** Far above a palette's kilobyte or two; a larger file is not one of ours. */
export const THEME_FILE_MAX_BYTES = 64 * 1024;

export interface ThemeFile {
  [THEME_FILE_MARK]: 1;
  name: string;
  author?: string;
  source?: string;
  colors: Omit<PaletteColors, "accent">;
  accent: string;
  /** One of `FONTS`; a face this build does not have is left out on reading. */
  font?: FontId;
}

/** The palette and accent in use, as a file. A preset is written under its own name. */
export function themeFileOf(theme: ThemePreference): ThemeFile {
  const custom = theme.palette === CUSTOM_PALETTE ? theme.custom : null;
  const { backgrounds, text, borders, semantic } = resolvePalette(theme).colors;
  return {
    [THEME_FILE_MARK]: 1,
    name: custom?.name ?? getPalette(theme.palette as PaletteId).label,
    ...(custom?.author === undefined ? {} : { author: custom.author }),
    ...(custom?.source === undefined ? {} : { source: custom.source }),
    colors: { backgrounds, text, borders, semantic },
    accent: theme.accent,
    font: theme.font,
  };
}

export function themeFileText(file: ThemeFile): string {
  return `${JSON.stringify(file, null, 2)}\n`;
}

/** The name with what a file system will not take left out, and the suffix. */
export function themeFileName(name: string, fallback: string): string {
  const safe = name
    .replace(/[\p{Cc}\\/:*?"<>|]/gu, "")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/^\.+/, "");
  return `${safe === "" ? fallback : safe}${THEME_FILE_SUFFIX}`;
}

export type ThemeFileProblem = "notOurs" | "colours" | "tooLarge";

export type ThemeFileRead =
  | { ok: true; custom: CustomTheme; patch: Partial<ThemePreference> }
  | { ok: false; problem: ThemeFileProblem };

const field = (value: unknown, key: string): unknown =>
  typeof value === "object" && value !== null ? (value as Record<string, unknown>)[key] : undefined;

/**
 * A file's theme as a patch to the preference: its palette as the person's
 * own, selected, its accent, and the dark scheme, since a custom palette is a
 * dark palette. Nothing else is taken from it.
 */
export function readThemeFile(text: string): ThemeFileRead {
  if (text.length > THEME_FILE_MAX_BYTES) return { ok: false, problem: "tooLarge" };
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { ok: false, problem: "notOurs" };
  }
  if (field(parsed, THEME_FILE_MARK) !== 1) return { ok: false, problem: "notOurs" };

  const accent = normaliseHex(field(parsed, "accent"));
  const colors = normaliseColours({ ...(field(parsed, "colors") as object | undefined), accent });
  if (accent === null || colors === null) return { ok: false, problem: "colours" };

  const custom = normaliseCustomTheme(
    makeCustomTheme({
      name: field(parsed, "name") as string,
      author: field(parsed, "author") as string,
      source: field(parsed, "source") as string,
      colors,
    }),
  );
  if (custom === null) return { ok: false, problem: "colours" };
  const font = FONTS.find((face) => face === field(parsed, "font"));
  return {
    ok: true,
    custom,
    patch: { palette: CUSTOM_PALETTE, custom, accent, scheme: "dark", ...(font === undefined ? {} : { font }) },
  };
}
