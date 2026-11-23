import { deriveFaint, deriveNeutral } from "./colour";
import { COLOUR_ROLES, type CustomTheme, type PaletteColors } from "./themes";

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
