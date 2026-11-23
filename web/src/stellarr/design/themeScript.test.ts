import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { makeCustomTheme } from "@/design/customTheme";
import { THEME_SCRIPT } from "@/design/themeScript";
import {
  DEFAULT_THEME,
  PALETTES,
  THEME_STORAGE_KEY,
  applyThemeAttributes,
  getPalette,
  normaliseTheme,
  resolveScheme,
  type PaletteColors,
} from "@/design/themes";

const root = document.documentElement;

/** Dark text on pale, partly see-through surfaces: the derivations' other direction. */
function lightColours(): PaletteColors {
  return {
    accent: "#0a7cff",
    backgrounds: {
      darkest: "#e4e6eb",
      dark: "#eceef2",
      primary: "#fafbfcf0",
      secondary: "#f2f3f5",
      tertiary: "#f6f7f9",
      hover: "#0000000d",
      active: "#0000001a",
      floating: "#ffffff",
    },
    text: { normal: "#111418", muted: "#4b5361", faint: "#c0c4cc" },
    borders: { subtle: "#0000000f", default: "#d9dce1", strong: "#00000033" },
    semantic: { success: "#1a7f37", warning: "#9a6700", danger: "#cf222e", info: "#0969da" },
  };
}

function resetRoot() {
  root.removeAttribute("style");
  for (const attribute of [...root.attributes]) {
    if (attribute.name.startsWith("data-")) root.removeAttribute(attribute.name);
  }
}

function snapshot(element: HTMLElement = root) {
  const variables = Object.fromEntries(
    [...element.style].map((name) => [name, element.style.getPropertyValue(name)]),
  );
  return { data: { ...element.dataset }, variables };
}

/** Runs the text as a browser does when it meets the inline script in <head>. */
function runScript() {
  (0, eval)(THEME_SCRIPT);
}

function prefersDark(dark: boolean) {
  vi.spyOn(window, "matchMedia").mockImplementation(
    (query) =>
      ({ matches: dark && query.includes("dark"), media: query }) as MediaQueryList,
  );
}

/** What the provider would put on <html> for the same stored value. */
function expectedFor(stored: unknown, dark = true) {
  const element = document.createElement("html");
  const theme = normaliseTheme(stored);
  applyThemeAttributes(element, theme, resolveScheme(theme.scheme, dark));
  return snapshot(element);
}

beforeEach(() => {
  localStorage.clear();
  resetRoot();
  prefersDark(true);
});

afterEach(() => {
  vi.restoreAllMocks();
  resetRoot();
});

describe("THEME_SCRIPT", () => {
  it("paints the default theme when nothing is stored", () => {
    runScript();

    expect(snapshot()).toEqual(expectedFor(DEFAULT_THEME));
    expect(root.dataset.theme).toBe(DEFAULT_THEME.palette);
  });

  it.each(PALETTES.map(({ id }) => id))(
    "writes what the provider writes for the %s palette",
    (palette) => {
      const stored = { ...DEFAULT_THEME, palette, accent: "#123456", font: "lexend" };
      localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify(stored));

      runScript();

      expect(snapshot()).toEqual(expectedFor(stored));
    },
  );

  it("takes the default accent of the palette when the stored one is not a colour", () => {
    const stored = { palette: "harbor", accent: "red;" };
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify(stored));

    runScript();

    expect(root.style.getPropertyValue("--palette-accent")).toBe("#1a9fff");
    expect(snapshot()).toEqual(expectedFor(stored));
  });

  it("chooses readable text for a light custom accent", () => {
    const stored = { ...DEFAULT_THEME, accent: "#FFEE00" };
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify(stored));

    runScript();

    expect(root.style.getPropertyValue("--palette-accent")).toBe("#ffee00");
    expect(root.style.getPropertyValue("--palette-on-accent")).toBe("#000000");
  });

  it.each([true, false])(
    "resolves the system scheme from the OS (dark: %s)",
    (dark) => {
      prefersDark(dark);
      const stored = { ...DEFAULT_THEME, scheme: "system" };
      localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify(stored));

      runScript();

      expect(root.dataset.scheme).toBe(dark ? "dark" : "light");
      expect(snapshot()).toEqual(expectedFor(stored, dark));
    },
  );

  it("replaces unknown values axis by axis", () => {
    const stored = {
      palette: "nope",
      scheme: "sepia",
      contrast: 3,
      size: "xl",
      density: "tight",
      font: "comic-sans",
      hasOwnProperty: "constructor",
    };
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify(stored));

    runScript();

    expect(snapshot()).toEqual(expectedFor(stored));
    expect(root.dataset.size).toBe(DEFAULT_THEME.size);
  });

  it.each(["{not json", "null", "42", '"text"'])(
    "falls back to the defaults for stored %s",
    (value) => {
      localStorage.setItem(THEME_STORAGE_KEY, value);

      runScript();

      expect(snapshot()).toEqual(expectedFor(DEFAULT_THEME));
    },
  );

  it.each([
    ["Ink's colours", getPalette("ink").colors],
    ["a light palette with translucent surfaces", lightColours()],
    ["a grey palette", getPalette("evergreen").colors],
  ])("paints a custom palette of %s as the provider does", (_label, colors) => {
    const stored = { ...DEFAULT_THEME, palette: "custom", custom: makeCustomTheme({ name: "x", colors }) };
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify(stored));

    runScript();

    expect(root.dataset.theme).toBe("custom");
    expect(snapshot()).toEqual(expectedFor(stored));
  });

  it("paints the custom palette in the stored accent and with its light tint", () => {
    const stored = { ...DEFAULT_THEME, scheme: "light", palette: "custom", accent: "#ffee00", custom: makeCustomTheme({ name: "x", colors: getPalette("harbor").colors }) };
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify(stored));

    runScript();

    expect(root.style.getPropertyValue("--palette-on-accent")).toBe("#000000");
    expect(root.style.getPropertyValue("--h-neutral")).toBe("254");
    expect(snapshot()).toEqual(expectedFor(stored));
  });

  it.each([
    ["one colour is not hex", { colors: { ...getPalette("ink").colors, text: { ...getPalette("ink").colors.text, muted: "url(https://example.com/x)" } } }],
    ["the derived faint colour is not hex", { accessibleFaint: "url(https://example.com/x)" }],
    ["the light tint is out of range", { neutral: { hue: 12, chroma: 400 } }],
  ])("falls back to the default preset when %s", (_label, overrides) => {
    const custom = { ...makeCustomTheme({ name: "x", colors: getPalette("ink").colors }), ...overrides };
    const stored = { ...DEFAULT_THEME, palette: "custom", custom };
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify(stored));

    runScript();

    expect(root.dataset.theme).toBe(DEFAULT_THEME.palette);
    expect(root.getAttribute("style")).not.toContain("url(");
    expect(snapshot()).toEqual(expectedFor(stored));
  });

  it("still paints the defaults when storage cannot be read", () => {
    vi.spyOn(localStorage, "getItem").mockImplementation(() => {
      throw new DOMException("denied", "SecurityError");
    });

    runScript();

    expect(snapshot()).toEqual(expectedFor(DEFAULT_THEME));
  });
});
