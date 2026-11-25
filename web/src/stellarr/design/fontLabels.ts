import type { FontId } from "./themes";

/**
 * How the picker names and groups each face. Apart from `themes.ts` because
 * only the picker needs it, and it loads with the signed-in app.
 */
export const FONT_LABELS: Record<FontId, { label: string; kind: "sans" | "rounded" | "mono" }> = {
  outfit: { label: "Outfit", kind: "sans" },
  inter: { label: "Inter", kind: "sans" },
  "geist-sans": { label: "Geist", kind: "sans" },
  "dm-sans": { label: "DM Sans", kind: "sans" },
  "plus-jakarta-sans": { label: "Plus Jakarta Sans", kind: "sans" },
  manrope: { label: "Manrope", kind: "sans" },
  figtree: { label: "Figtree", kind: "sans" },
  "ibm-plex-sans": { label: "IBM Plex Sans", kind: "sans" },
  "source-sans-3": { label: "Source Sans 3", kind: "sans" },
  "noto-sans": { label: "Noto Sans", kind: "sans" },
  "open-sans": { label: "Open Sans", kind: "sans" },
  lato: { label: "Lato", kind: "sans" },
  poppins: { label: "Poppins", kind: "sans" },
  montserrat: { label: "Montserrat", kind: "sans" },
  raleway: { label: "Raleway", kind: "sans" },
  "space-grotesk": { label: "Space Grotesk", kind: "sans" },
  sora: { label: "Sora", kind: "sans" },
  lexend: { label: "Lexend", kind: "sans" },
  opendyslexic: { label: "OpenDyslexic", kind: "sans" },
  nunito: { label: "Nunito", kind: "rounded" },
  rubik: { label: "Rubik", kind: "rounded" },
  quicksand: { label: "Quicksand", kind: "rounded" },
  "baloo-2": { label: "Baloo 2", kind: "rounded" },
  "varela-round": { label: "Varela Round", kind: "rounded" },
  comfortaa: { label: "Comfortaa", kind: "rounded" },
  "fira-code": { label: "Fira Code", kind: "mono" },
  "jetbrains-mono": { label: "JetBrains Mono", kind: "mono" },
  "source-code-pro": { label: "Source Code Pro", kind: "mono" },
  "ibm-plex-mono": { label: "IBM Plex Mono", kind: "mono" },
};

/** A family name as a theme writes it ("figtree", "Geist Sans"), matched to one of ours. */
export function matchFont(family: string): FontId | null {
  const wanted = family.trim().replace(/\s+/g, " ").toLowerCase();
  if (wanted === "") return null;
  const entries = Object.entries(FONT_LABELS) as [FontId, { label: string }][];
  const found = entries.find(
    ([id, { label }]) => id === wanted || label.toLowerCase() === wanted || id.replace(/-/g, " ") === wanted,
  );
  return found?.[0] ?? null;
}
