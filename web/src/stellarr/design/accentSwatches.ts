import { STELLARR_ACCENT } from "./themes";

/**
 * Accents offered beside the theme's own. The presets' colours first, then
 * hues none of them has, so a swatch row reads as one wheel. Any other colour
 * is still a click away through the picker; these only spare the trip.
 *
 * Apart from `themes.ts` because only the settings that offer them use them,
 * and those load with the signed-in app rather than with the first page.
 */
export const ACCENT_SWATCHES = [
  { id: "indigo", hex: STELLARR_ACCENT },
  { id: "violet", hex: "#9146ff" },
  { id: "pink", hex: "#e93d82" },
  { id: "red", hex: "#e5484d" },
  { id: "orange", hex: "#f76b15" },
  { id: "green", hex: "#1db954" },
  { id: "teal", hex: "#12a594" },
  { id: "blue", hex: "#1a9fff" },
  { id: "sky", hex: "#58a6ff" },
] as const;

export type AccentSwatchId = (typeof ACCENT_SWATCHES)[number]["id"];
