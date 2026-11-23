import { makeCustomTheme } from "./customTheme";
import {
  CUSTOM_PALETTE,
  FONTS,
  getPalette,
  normaliseColours,
  normaliseCustomTheme,
  normaliseHex,
  normaliseWallpaper,
  resolvePalette,
  type CustomTheme,
  type FontId,
  type PaletteColors,
  type PaletteId,
  type ThemePreference,
  type WallpaperSettings,
} from "./themes";

/**
 * Stellarr's own theme file, `<name>.stellarr-theme.json`:
 *
 *   { "stellarrTheme": 1, "name": …, "author"?: …, "source"?: …,
 *     "colors": { backgrounds, text, borders, semantic }, "accent": …,
 *     "font"?: …,
 *     "wallpaper"?: { "image": "data:image/webp;base64,…", "shade": …, "blur": … } }
 *
 * `stellarrTheme` only says the file is ours. A file carries what a theme
 * looks like and nothing personal: size, density, contrast and reduced motion
 * are the reader's, so they are never written or read. Reading goes through
 * the same normaliser as storage and the account.
 *
 * An embedded wallpaper is only ever bytes to decode: it goes through the
 * same decoding, re-encoding and cap as a picked file before it is uploaded,
 * and the address in the file is never put into a style.
 */

export const THEME_FILE_MARK = "stellarrTheme";
export const THEME_FILE_SUFFIX = ".stellarr-theme.json";
/**
 * Room for a palette and an embedded wallpaper: the account takes at most
 * 8 MB of picture, which base64 makes about 11.
 */
export const THEME_FILE_MAX_BYTES = 12 * 1024 * 1024;

export interface ThemeFile {
  [THEME_FILE_MARK]: 1;
  name: string;
  author?: string;
  source?: string;
  colors: Omit<PaletteColors, "accent">;
  accent: string;
  /** One of `FONTS`; a face this build does not have is left out on reading. */
  font?: FontId;
  wallpaper?: { image: string; shade: number; blur: number };
}

/**
 * The palette, accent and face in use, as a file, with the wallpaper when its
 * picture is given as a data address. A preset is written under its own name.
 */
export function themeFileOf(theme: ThemePreference, wallpaperImage?: string): ThemeFile {
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
    ...(wallpaperImage === undefined || theme.wallpaper === null
      ? {}
      : { wallpaper: { image: wallpaperImage, shade: theme.wallpaper.shade, blur: theme.wallpaper.blur } }),
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

/** A wallpaper a file carried: bytes to prepare and upload, and how it is shown. */
export interface FileWallpaper extends Pick<WallpaperSettings, "shade" | "blur"> {
  image: Blob;
}

export type ThemeFileRead =
  | { ok: true; custom: CustomTheme; patch: Partial<ThemePreference>; wallpaper?: FileWallpaper }
  | { ok: false; problem: ThemeFileProblem };

const field = (value: unknown, key: string): unknown =>
  typeof value === "object" && value !== null ? (value as Record<string, unknown>)[key] : undefined;

/** A raster picture as a base64 data address, and nothing else. */
const IMAGE_DATA = /^data:(image\/(?:webp|jpeg|png));base64,([A-Za-z0-9+/]+={0,2})$/;

/** The bytes of an embedded picture, or null when it is anything but a raster data address. */
export function imageFromDataAddress(value: unknown): Blob | null {
  if (typeof value !== "string") return null;
  const match = IMAGE_DATA.exec(value);
  if (match === null) return null;
  try {
    const text = atob(match[2] ?? "");
    const bytes = new Uint8Array(text.length);
    for (let index = 0; index < text.length; index += 1) bytes[index] = text.charCodeAt(index);
    return new Blob([bytes], { type: match[1] });
  } catch {
    return null;
  }
}

/**
 * A file's theme as a patch to the preference: its palette as the person's
 * own, selected, its accent and face, and the dark scheme, since a custom
 * palette is a dark palette. A wallpaper comes back apart, as bytes, since
 * it has to be prepared and uploaded before it is anything.
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
  const image = imageFromDataAddress(field(field(parsed, "wallpaper"), "image"));
  const settings = normaliseWallpaper(field(parsed, "wallpaper"));
  return {
    ok: true,
    custom,
    patch: { palette: CUSTOM_PALETTE, custom, accent, scheme: "dark", ...(font === undefined ? {} : { font }) },
    ...(image === null || settings === null ? {} : { wallpaper: { image, shade: settings.shade, blur: settings.blur } }),
  };
}
