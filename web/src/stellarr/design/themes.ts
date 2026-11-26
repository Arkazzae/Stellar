/**
 * The theme registry.
 *
 * A theme is six independent choices, stored together and applied to `<html>`:
 *
 *   palette   the complete colour vocabulary    -> `data-theme` and CSS variables
 *   accent    an optional personal accent        -> `--palette-accent`
 *   scheme    dark, light, or follow the OS      -> `data-scheme`
 *   contrast  normal or high                     -> `data-contrast`
 *   size      how large message text is          -> `data-size`
 *   density   how much air a message gets        -> `data-density`
 *
 * A preset owns every colour role, not just a hue. Choosing one resets the
 * accent to the colour designed for it, but never changes typography or
 * density.
 *
 * Beside the presets there is one custom palette, the person's own (`custom`),
 * kept while a preset is selected so they can go back to it. Its colours are
 * `#rrggbb` or `#rrggbbaa` and nothing else: they become custom properties
 * that components paint with, and a value such as `url(…)` would make every
 * browser that applies a shared theme fetch an address.
 *
 * The palette data lives here so the runtime, the pre-hydration script, the
 * previews and the tests share one set of values. `recipe.css` maps this
 * vocabulary onto the semantic tokens components use.
 */

export interface PaletteColors {
  accent: string;
  backgrounds: {
    darkest: string;
    dark: string;
    primary: string;
    secondary: string;
    tertiary: string;
    hover: string;
    active: string;
    floating: string;
  };
  text: {
    normal: string;
    muted: string;
    faint: string;
  };
  borders: {
    subtle: string;
    default: string;
    strong: string;
  };
  semantic: {
    success: string;
    warning: string;
    danger: string;
    info: string;
  };
}

interface PaletteDefinition {
  id: string;
  label: string;
  colors: PaletteColors;
  /** Small metadata still needs AA contrast even when the raw "faint" swatch is decorative. */
  accessibleFaint: string;
}

/**
 * The accent of Midnight and Slate, and so of a fresh install. The fallback in
 * `recipe.css` and the fill in `assets/icon.svg` cannot import it and repeat the
 * value; `themeAccent.test.ts` keeps the three in step.
 */
export const STELLARR_ACCENT = "#6f65d1";

export const PALETTES = [
  {
    id: "midnight",
    label: "Midnight",
    colors: {
      accent: STELLARR_ACCENT,
      backgrounds: {
        darkest: "#09090b",
        dark: "#18181b",
        primary: "#222327",
        secondary: "#18181b",
        tertiary: "#1e1f23",
        hover: "#27272a",
        active: "#33343a",
        floating: "#101113",
      },
      text: { normal: "#f3f4f6", muted: "#b9bec9", faint: "#747b89" },
      borders: {
        subtle: "#ffffff0f",
        default: "#292e39",
        strong: "#ffffff33",
      },
      semantic: {
        success: "#3dd68c",
        warning: "#f0b232",
        danger: "#f2555a",
        info: "#6ea8fe",
      },
    },
    accessibleFaint: "#9aa0ad",
  },
  {
    id: "slate",
    label: "Slate",
    colors: {
      accent: STELLARR_ACCENT,
      backgrounds: {
        darkest: "#111214",
        dark: "#1e1f22",
        primary: "#2b2d31",
        secondary: "#1e1f22",
        tertiary: "#232428",
        hover: "#35373c",
        active: "#3f4147",
        floating: "#111214",
      },
      text: { normal: "#f2f3f5", muted: "#b5bac1", faint: "#80848e" },
      borders: {
        subtle: "#ffffff0f",
        default: "#3f4147",
        strong: "#ffffff33",
      },
      semantic: {
        success: "#23a559",
        warning: "#f0b232",
        danger: "#f23f42",
        info: "#00a8fc",
      },
    },
    accessibleFaint: "#afb3bb",
  },
  {
    id: "evergreen",
    label: "Evergreen",
    colors: {
      accent: "#1db954",
      backgrounds: {
        darkest: "#0a0a0a",
        dark: "#121212",
        primary: "#1a1a1a",
        secondary: "#121212",
        tertiary: "#161616",
        hover: "#282828",
        active: "#333333",
        floating: "#0e0e0e",
      },
      text: { normal: "#e8e8e8", muted: "#a7a7a7", faint: "#6a6a6a" },
      borders: {
        subtle: "#ffffff0f",
        default: "#282828",
        strong: "#ffffff2e",
      },
      semantic: {
        success: "#1db954",
        warning: "#f0b232",
        danger: "#e91429",
        info: "#1db954",
      },
    },
    accessibleFaint: "#a0a0a0",
  },
  {
    id: "harbor",
    label: "Harbor",
    colors: {
      accent: "#1a9fff",
      backgrounds: {
        darkest: "#0e1116",
        dark: "#171a21",
        primary: "#1b2838",
        secondary: "#171a21",
        tertiary: "#192330",
        hover: "#2a475e",
        active: "#335573",
        floating: "#111419",
      },
      text: { normal: "#f1f1f1", muted: "#c7d5e0", faint: "#7d8b96" },
      borders: {
        subtle: "#c7d5e014",
        default: "#2a475e",
        strong: "#66c0f461",
      },
      semantic: {
        success: "#75b022",
        warning: "#e5b13a",
        danger: "#d94126",
        info: "#66c0f4",
      },
    },
    accessibleFaint: "#d1d9df",
  },
  {
    id: "amethyst",
    label: "Amethyst",
    colors: {
      accent: "#9146ff",
      backgrounds: {
        darkest: "#0a0a0c",
        dark: "#0e0e10",
        primary: "#18181b",
        secondary: "#0e0e10",
        tertiary: "#141418",
        hover: "#26262c",
        active: "#323239",
        floating: "#0c0c0e",
      },
      text: { normal: "#efeff1", muted: "#adadb8", faint: "#7a7a85" },
      borders: {
        subtle: "#ffffff0f",
        default: "#35353b",
        strong: "#ffffff33",
      },
      semantic: {
        success: "#00f593",
        warning: "#ffb31a",
        danger: "#eb0400",
        info: "#00c8ff",
      },
    },
    accessibleFaint: "#a4a4ae",
  },
  {
    id: "ink",
    label: "Ink",
    colors: {
      accent: "#58a6ff",
      backgrounds: {
        darkest: "#010409",
        dark: "#0d1117",
        primary: "#161b22",
        secondary: "#0d1117",
        tertiary: "#121820",
        hover: "#1f2937",
        active: "#272e38",
        floating: "#090c10",
      },
      text: { normal: "#f0f6fc", muted: "#b1bac4", faint: "#7d8590" },
      borders: {
        subtle: "#f0f6fc14",
        default: "#30363d",
        strong: "#484f58",
      },
      semantic: {
        success: "#3fb950",
        warning: "#d29922",
        danger: "#f85149",
        info: "#58a6ff",
      },
    },
    accessibleFaint: "#a8b1bb",
  },
] as const satisfies readonly PaletteDefinition[];

type Palette = (typeof PALETTES)[number];
export type PaletteId = Palette["id"];

/** The `palette` value that selects the person's own palette. */
export const CUSTOM_PALETTE = "custom";
export type PaletteChoice = PaletteId | typeof CUSTOM_PALETTE;

/** The colour roles of each group of `PaletteColors`, in the order a board shows them. */
export const COLOUR_ROLES = {
  backgrounds: ["darkest", "dark", "primary", "secondary", "tertiary", "hover", "active", "floating"],
  text: ["normal", "muted", "faint"],
  borders: ["subtle", "default", "strong"],
  semantic: ["success", "warning", "danger", "info"],
} as const;

export type ColourGroup = keyof typeof COLOUR_ROLES;

/** The custom property a role is written to: `--palette-background-primary`. */
const VARIABLE_PREFIX: Record<ColourGroup, `--palette-${string}`> = {
  backgrounds: "--palette-background-",
  text: "--palette-text-",
  borders: "--palette-border-",
  semantic: "--palette-",
};

/** Every role of a palette but the accent, with the custom property it is written to. */
export const PALETTE_ROLE_VARIABLES = (Object.keys(COLOUR_ROLES) as ColourGroup[]).flatMap((group) =>
  COLOUR_ROLES[group].map((role) => [group, role, `${VARIABLE_PREFIX[group]}${role}`] as const),
);

export const CUSTOM_NAME_MAX = 64;
export const CUSTOM_SOURCE_MAX = 512;
export const THEME_CSS_MAX = 4096;

/** Bound the saved editor text, including its JSON escapes, for account settings. */
export function fitsThemeCss(value: string): boolean {
  return value.length <= THEME_CSS_MAX && new TextEncoder().encode(JSON.stringify(value)).byteLength <= THEME_CSS_MAX;
}

/** What paints a palette: its colours and the values derived from them. */
export interface ResolvedPalette {
  colors: PaletteColors;
  /** The faint colour small copy is set in, AA on every background. */
  accessibleFaint: string;
  /** The light ramp's tint; a preset's comes from `palettes.css` instead. */
  neutral?: LightTint;
}

/** `--h-neutral` and `--c-neutral`: the hue and the strength of the light ramp's tint. */
export interface LightTint {
  hue: number;
  chroma: number;
}

/**
 * The person's own palette: a preset they changed, a file, or a converted
 * theme. Its derived values are worked out once, when its colours are set
 * (`customTheme.ts`), and kept with them, so the first page and the
 * pre-paint script apply them without the arithmetic.
 */
export interface CustomTheme extends ResolvedPalette {
  name: string;
  /** Source text from the CSS editor, kept for editing. Only the parsed colours are painted. */
  css?: string;
  author?: string;
  /** Where it came from, an http(s) address. */
  source?: string;
  /** The preset it was forked from, which discarding it goes back to. */
  basedOn?: PaletteId;
  neutral: LightTint;
}

/** The largest `--c-neutral` a custom palette may ask for; the presets stay under 4. */
export const LIGHT_TINT_MAX = 20;

export const SIZES = [{ id: "sm" }, { id: "md" }, { id: "lg" }] as const;

export type SizePreference = (typeof SIZES)[number]["id"];

export const DENSITIES = [{ id: "cozy" }, { id: "compact" }] as const;

export type DensityPreference = (typeof DENSITIES)[number]["id"];

/** `system` follows `prefers-color-scheme` and keeps following it. */
export const SCHEMES = ["system", "dark", "light"] as const;
export type SchemePreference = (typeof SCHEMES)[number];
export type ResolvedScheme = "dark" | "light";

export const CONTRASTS = ["normal", "high"] as const;
export type ContrastPreference = (typeof CONTRASTS)[number];

export interface ThemePreference {
  palette: PaletteChoice;
  accent: string;
  /** The person's own palette, kept while a preset is selected. */
  custom: CustomTheme | null;
  scheme: SchemePreference;
  contrast: ContrastPreference;
  size: SizePreference;
  density: DensityPreference;
  /**
   * How the account's wallpaper is shown, or null for none. The picture
   * itself is on the account (`wallpaperUrl`), far too large for a settings
   * section; this carries only what is cheap to sync.
   */
  wallpaper: WallpaperSettings | null;
}

export interface WallpaperSettings {
  /** How far the scheme's background covers the picture, in percent. */
  shade: number;
  /** How much the picture is blurred, in pixels. */
  blur: number;
  /** The picture in at most 32 bytes, base64 (`lib/media/placeholder.ts`), for before it arrives. */
  placeholder?: string;
}

/**
 * The shading a new wallpaper starts at. Together with the structural
 * surfaces' opacity in `wallpaper.css` it keeps small text at AA over a white
 * pixel in the dark scheme and a black one in the light scheme, for every
 * preset (`wallpaper.test.ts`).
 */
export const WALLPAPER_SHADE_DEFAULT = 80;
export const WALLPAPER_BLUR_MAX = 24;

export const DEFAULT_THEME: ThemePreference = {
  palette: "midnight",
  accent: PALETTES[0].colors.accent,
  custom: null,
  scheme: "dark",
  contrast: "normal",
  size: "md",
  density: "cozy",
  wallpaper: null,
};

/** One key holding one JSON object, so a partial write can never split the axes. */
export const THEME_STORAGE_KEY = "stellarr.theme";

const PALETTE_IDS: readonly PaletteId[] = PALETTES.map(({ id }) => id);
const SIZE_IDS: readonly SizePreference[] = SIZES.map(({ id }) => id);
const DENSITY_IDS: readonly DensityPreference[] = DENSITIES.map(({ id }) => id);

export type ThemeCssVariables = Record<`--${string}`, string>;

/** An accent: opaque `#rrggbb`, lower case. */
export function normaliseHex(value: unknown): string | null {
  if (typeof value !== "string" || !/^#[\da-f]{6}$/i.test(value)) return null;
  return value.toLowerCase();
}

/** `#rrggbb` or `#rrggbbaa`, nothing else: no names, no functions, no `url(`. */
const HEX_COLOUR = /^#[\da-f]{6}(?:[\da-f]{2})?$/i;

/** A colour of a custom palette: `#rrggbb` or `#rrggbbaa`, lower case. */
export function normaliseColour(value: unknown): string | null {
  return typeof value === "string" && HEX_COLOUR.test(value) ? value.toLowerCase() : null;
}

export function getPalette(id: PaletteId): Palette {
  return PALETTES.find((palette) => palette.id === id) ?? PALETTES[0];
}

function relativeLuminance(hex: string): number {
  const [red = 0, green = 0, blue = 0] = [1, 3, 5].map((offset) => {
    const channel = Number.parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return channel <= 0.04045
      ? channel / 12.92
      : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

/** Picks black or white for controls filled with the user's accent. */
export function readableTextOn(hex: string): "#000000" | "#ffffff" {
  const luminance = relativeLuminance(hex);
  const contrastWithBlack = (luminance + 0.05) / 0.05;
  const contrastWithWhite = 1.05 / (luminance + 0.05);
  return contrastWithBlack >= contrastWithWhite ? "#000000" : "#ffffff";
}

/** The palette a theme paints with: a preset, or the person's own. */
export function resolvePalette(theme: Pick<ThemePreference, "palette" | "custom">): ResolvedPalette {
  if (theme.palette !== CUSTOM_PALETTE) return getPalette(theme.palette);
  return theme.custom ?? PALETTES[0];
}

export function paletteCssVariables(
  palette: ResolvedPalette,
  accent: string = palette.colors.accent,
): ThemeCssVariables {
  const variables: ThemeCssVariables = {
    "--palette-accent": accent,
    "--palette-on-accent": readableTextOn(accent),
    "--palette-text-faint-accessible": palette.accessibleFaint,
  };
  for (const [group, role, name] of PALETTE_ROLE_VARIABLES) {
    variables[name] = (palette.colors[group] as Record<string, string>)[role] ?? "";
  }
  if (palette.neutral !== undefined) {
    variables["--palette-on-background"] = readableTextOn(palette.colors.backgrounds.primary);
    variables["--palette-on-danger"] = readableTextOn(palette.colors.semantic.danger);
    variables["--h-neutral"] = String(palette.neutral.hue);
    variables["--c-neutral"] = String(palette.neutral.chroma);
  }
  return variables;
}

function oneOf<T extends string>(
  allowed: readonly T[],
  value: unknown,
  fallback: T,
): T {
  return allowed.find((candidate) => candidate === value) ?? fallback;
}

const field = (value: unknown, key: string): unknown =>
  typeof value === "object" && value !== null ? (value as Record<string, unknown>)[key] : undefined;

/**
 * Words a person or a file supplied, as one line: control and direction
 * characters removed, trimmed, and cut at `max` characters.
 */
export function normaliseText(value: unknown, max: number): string {
  if (typeof value !== "string") return "";
  const line = value
    .slice(0, max * 4)
    .replace(/[\p{Cc}\p{Bidi_Control}]/gu, "")
    .trim();
  return Array.from(line).slice(0, max).join("").trim();
}

function normaliseSource(value: unknown): string | undefined {
  if (typeof value !== "string" || value.length > CUSTOM_SOURCE_MAX) return undefined;
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Every role of a palette, or null when one is missing or is not a hex
 * colour. All or nothing: half a palette painted over the rest of another
 * is not the palette anyone made.
 */
export function normaliseColours(value: unknown): PaletteColors | null {
  const accent = normaliseHex(field(value, "accent"));
  if (accent === null) return null;
  const colours: Record<string, unknown> = { accent };
  for (const group of Object.keys(COLOUR_ROLES) as ColourGroup[]) {
    const source = field(value, group);
    const roles: Record<string, string> = {};
    for (const role of COLOUR_ROLES[group]) {
      const colour = normaliseColour(field(source, role));
      if (colour === null) return null;
      roles[role] = colour;
    }
    colours[group] = roles;
  }
  return colours as unknown as PaletteColors;
}

/** A whole number of degrees, or a tint strength, as the light ramp takes it. */
function tintNumber(value: unknown, max: number): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= max ? value : null;
}

/**
 * The person's own palette as storage, the account or a file gives it back,
 * or null when its colours or derived values do not hold. Only known fields
 * are read and a new object is built, so nothing else the input carries
 * comes along.
 */
export function normaliseCustomTheme(value: unknown): CustomTheme | null {
  const colors = normaliseColours(field(value, "colors"));
  const accessibleFaint = normaliseHex(field(value, "accessibleFaint"));
  const neutral = field(value, "neutral");
  const hue = tintNumber(field(neutral, "hue"), 359);
  const chroma = tintNumber(field(neutral, "chroma"), LIGHT_TINT_MAX);
  if (colors === null || accessibleFaint === null || hue === null || chroma === null) return null;
  const author = normaliseText(field(value, "author"), CUSTOM_NAME_MAX);
  const source = normaliseSource(field(value, "source"));
  const basedOn = PALETTE_IDS.find((id) => id === field(value, "basedOn"));
  const css = field(value, "css");
  return {
    name: normaliseText(field(value, "name"), CUSTOM_NAME_MAX),
    ...(typeof css === "string" && fitsThemeCss(css) ? { css } : {}),
    ...(author === "" ? {} : { author }),
    ...(source === undefined ? {} : { source }),
    ...(basedOn === undefined ? {} : { basedOn }),
    colors,
    accessibleFaint,
    neutral: { hue, chroma },
  };
}

/** A whole number from `min` to `max`, or `fallback` for anything else. */
function wholeNumber(value: unknown, max: number, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.round(Math.min(max, Math.max(0, value))) : fallback;
}

/** Any object says a wallpaper is set; its fields fall back one by one. */
export function normaliseWallpaper(value: unknown): WallpaperSettings | null {
  if (typeof value !== "object" || value === null) return null;
  const placeholder = field(value, "placeholder");
  return {
    shade: wholeNumber(field(value, "shade"), 100, WALLPAPER_SHADE_DEFAULT),
    blur: wholeNumber(field(value, "blur"), WALLPAPER_BLUR_MAX, 0),
    // Base64 of at most 32 bytes.
    ...(typeof placeholder === "string" && /^[\w+/=-]{4,44}$/.test(placeholder) ? { placeholder } : {}),
  };
}

/**
 * Narrows anything read back from storage, the account or a file to a
 * usable preference. Each axis falls back on its own instead of the whole
 * object being discarded, so a bad or missing value costs only its own axis.
 * The provider passes every write through here too.
 */
export function normaliseTheme(input: unknown): ThemePreference {
  if (typeof input !== "object" || input === null) return DEFAULT_THEME;
  const raw = input as Partial<Record<keyof ThemePreference, unknown>>;

  const custom = normaliseCustomTheme(raw.custom);
  const palette: PaletteChoice =
    raw.palette === CUSTOM_PALETTE && custom !== null
      ? CUSTOM_PALETTE
      : oneOf(PALETTE_IDS, raw.palette, PALETTES[0].id);
  return {
    palette,
    accent: normaliseHex(raw.accent) ?? resolvePalette({ palette, custom }).colors.accent,
    custom,
    scheme: oneOf(SCHEMES, raw.scheme, DEFAULT_THEME.scheme),
    contrast: oneOf(CONTRASTS, raw.contrast, DEFAULT_THEME.contrast),
    size: oneOf(SIZE_IDS, raw.size, DEFAULT_THEME.size),
    density: oneOf(DENSITY_IDS, raw.density, DEFAULT_THEME.density),
    wallpaper: normaliseWallpaper(raw.wallpaper),
  };
}

export function resolveScheme(
  preference: SchemePreference,
  prefersDark: boolean,
): ResolvedScheme {
  if (preference === "system") return prefersDark ? "dark" : "light";
  return preference;
}

/**
 * Writes every axis onto an element. The provider calls it at runtime; the
 * pre-hydration script produces the same result on `<html>` before first paint.
 *
 * A custom palette writes its light-ramp tint inline, since `palettes.css`
 * keys the presets' by `data-theme`; going back to a preset removes it again,
 * or the inline value would outrank the preset's.
 */
export function applyThemeAttributes(
  element: HTMLElement,
  theme: ThemePreference,
  resolved: ResolvedScheme,
): void {
  const { dataset, style } = element;
  dataset.theme = theme.palette;
  dataset.scheme = resolved;
  dataset.contrast = theme.contrast;
  dataset.size = theme.size;
  dataset.density = theme.density;

  const palette = resolvePalette(theme);
  const accent = normaliseHex(theme.accent) ?? palette.colors.accent;
  style.removeProperty("--palette-on-background");
  style.removeProperty("--palette-on-danger");
  style.removeProperty("--h-neutral");
  style.removeProperty("--c-neutral");
  for (const [name, value] of Object.entries(
    paletteCssVariables(palette, accent),
  )) {
    style.setProperty(name, value);
  }
}
