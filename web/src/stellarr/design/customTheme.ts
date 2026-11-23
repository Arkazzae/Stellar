import { deriveFaint, deriveNeutral } from "./colour";
import {
  COLOUR_ROLES,
  getPalette,
  type ColourGroup,
  type CustomTheme,
  type PaletteColors,
  type PaletteId,
} from "./themes";

/** What a person or a file decides about a custom palette; the rest is derived. */
export type CustomThemeFields = Omit<CustomTheme, "accessibleFaint" | "neutral">;

/**
 * A custom palette with its derived values worked out from its colours. Every
 * writer of a custom palette goes through here, so the stored values always
 * belong to the colours stored with them.
 */
export function makeCustomTheme(fields: CustomThemeFields): CustomTheme {
  const { colors } = fields;
  return {
    ...fields,
    accessibleFaint: deriveFaint(
      colors.text.faint,
      colors.text.normal,
      COLOUR_ROLES.backgrounds.map((role) => colors.backgrounds[role]),
    ),
    neutral: deriveNeutral(colors.backgrounds.primary, colors.backgrounds.darkest),
  };
}

/** What a person or a file decided about a custom palette, without what was derived. */
export function fieldsOf({ name, author, source, basedOn, colors }: CustomTheme): CustomThemeFields {
  return {
    name,
    ...(author === undefined ? {} : { author }),
    ...(source === undefined ? {} : { source }),
    ...(basedOn === undefined ? {} : { basedOn }),
    colors,
  };
}

/** The palette with one role changed, its derived values worked out again. */
export function withColour(
  custom: CustomTheme,
  group: ColourGroup,
  role: string,
  value: string,
): CustomTheme {
  const colors = copyColours(custom.colors);
  (colors[group] as Record<string, string>)[role] = value;
  return makeCustomTheme({ ...fieldsOf(custom), colors });
}

/**
 * A preset made into the person's own palette, ready for its first change,
 * under a name of its own so it is not taken for the preset beside it.
 */
export function forkPreset(id: PaletteId, name: string = getPalette(id).label): CustomTheme {
  const preset = getPalette(id);
  return makeCustomTheme({ name, basedOn: preset.id, colors: copyColours(preset.colors) });
}

/** A deep copy of a palette's colours, to edit without touching the original. */
export function copyColours(colors: PaletteColors): PaletteColors {
  return {
    accent: colors.accent,
    backgrounds: { ...colors.backgrounds },
    text: { ...colors.text },
    borders: { ...colors.borders },
    semantic: { ...colors.semantic },
  };
}
