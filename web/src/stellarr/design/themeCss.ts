import { copyColours, makeCustomTheme } from "./customTheme";
import {
  CUSTOM_PALETTE,
  PALETTE_ROLE_VARIABLES,
  PALETTES,
  fitsThemeCss,
  getPalette,
  normaliseColour,
  normaliseHex,
  resolvePalette,
  type ColourGroup,
  type CustomTheme,
  type ThemePreference,
} from "./themes";

const ACCENT = "--palette-accent";
const roles = new Map<string, { group: ColourGroup; role: string }>(
  PALETTE_ROLE_VARIABLES.map(([group, role, variable]) => [variable, { group, role }]),
);

export function themeCssSource(theme: ThemePreference): string {
  if (theme.custom?.css !== undefined) return theme.custom.css;
  const palette = theme.custom ?? resolvePalette(theme);
  const accent = theme.palette === CUSTOM_PALETTE || theme.custom === null ? theme.accent : palette.colors.accent;
  const declarations = PALETTE_ROLE_VARIABLES.map(([group, role, variable]) => {
    const colors: Record<string, string> = palette.colors[group];
    return `  ${variable}: ${colors[role]};`;
  });
  return [":root {", `  ${ACCENT}: ${accent};`, ...declarations, "}", ""].join("\n");
}

type ParsedThemeCss = { custom: CustomTheme } | { problem: "size" | "syntax" | "property" | "colour"; property?: string };

/** Read the documented token declarations without adding a stylesheet to the page. */
export function parseThemeCss(source: string, theme: ThemePreference, name: string): ParsedThemeCss {
  if (!fitsThemeCss(source)) return { problem: "size" };
  const clean = source.replace(/\/\*[\s\S]*?\*\//g, "").trim();
  const block = /^:root\s*\{([^{}]*)\}$/.exec(clean)?.[1];
  const declarations = block?.split(";").map((part) => part.trim()).filter(Boolean);
  if (!declarations?.length) return { problem: "syntax" };

  const basedOn = theme.custom?.basedOn ?? (theme.palette === CUSTOM_PALETTE ? PALETTES[0].id : theme.palette);
  const colors = copyColours(getPalette(basedOn).colors);
  for (const declaration of declarations) {
    const [, property, value] = /^(--[a-zA-Z0-9_-]+)\s*:\s*(.+)$/.exec(declaration) ?? [];
    if (property === undefined || value === undefined) return { problem: "syntax" };
    const role = roles.get(property);
    if (role === undefined && property !== ACCENT) return { problem: "property", property };
    const expanded = /^#[\da-f]{3,4}$/i.test(value)
      ? "#" + Array.from(value.slice(1), (digit) => digit.repeat(2)).join("")
      : value;
    const colour = property === ACCENT ? normaliseHex(expanded) : normaliseColour(expanded);
    if (colour === null) return { problem: "colour", property };
    if (role === undefined) colors.accent = colour;
    else (colors[role.group] as Record<string, string>)[role.role] = colour;
  }
  return { custom: makeCustomTheme({ name, basedOn, colors, css: source }) };
}
