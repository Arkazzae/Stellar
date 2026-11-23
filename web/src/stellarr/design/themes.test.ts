import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import {
  contrastRatio,
  deriveFaint,
  deriveNeutral,
  flatten,
  parseHex,
  toOklch,
} from "@/design/colour";
import { copyColours, makeCustomTheme } from "@/design/customTheme";
import {
  CUSTOM_NAME_MAX,
  CUSTOM_SOURCE_MAX,
  DEFAULT_THEME,
  PALETTES,
  applyThemeAttributes,
  getPalette,
  normaliseColours,
  normaliseCustomTheme,
  normaliseTheme,
  paletteCssVariables,
  readableTextOn,
} from "@/design/themes";
import { parseBlocks } from "@/design/tokenRamp";
import { appearanceToWire } from "@/lib/platform/clientSettings";

/** `ClientSettings.MaxLength` on the server: what one section may hold. */
const CLIENT_SETTINGS_SECTION_MAX = 8192;

const palettesCss = parseBlocks(
  readFileSync(
    join(process.cwd(), "src/stellarr/styles/themes/palettes.css"),
    "utf8",
  ),
);
const recipeCss = readFileSync(
  join(process.cwd(), "src/stellarr/styles/themes/recipe.css"),
  "utf8",
);
const railCss = readFileSync(
  join(process.cwd(), "src/stellarr/styles/components/rail.css"),
  "utf8",
);
const roomCss = readFileSync(
  join(process.cwd(), "src/stellarr/styles/components/room.css"),
  "utf8",
);
const rosterCss = readFileSync(
  join(process.cwd(), "src/stellarr/styles/components/roster.css"),
  "utf8",
);

function luminance(hex: string): number {
  const channels = [1, 3, 5].map((offset) => {
    const value = Number.parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return (
    0.2126 * (channels[0] ?? 0) +
    0.7152 * (channels[1] ?? 0) +
    0.0722 * (channels[2] ?? 0)
  );
}

function contrast(a: string, b: string): number {
  const first = luminance(a);
  const second = luminance(b);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

describe("theme registry", () => {
  it("offers the requested complete preset set", () => {
    expect(PALETTES.map(({ label }) => label)).toEqual([
      "Midnight",
      "Slate",
      "Evergreen",
      "Harbor",
      "Amethyst",
      "Ink",
    ]);
  });

  it("has one light-ramp CSS block per registered palette", () => {
    const registered = new Set<string>(PALETTES.map(({ id }) => id));
    for (const id of registered) {
      expect(palettesCss.has(`[data-theme="${id}"]`)).toBe(true);
    }
    for (const selector of palettesCss.keys()) {
      const id = /^\[data-theme="([a-z-]+)"\]$/.exec(selector)?.[1];
      if (id !== undefined) expect(registered.has(id)).toBe(true);
    }
  });

  it("keeps structural panels on explicit surface roles", () => {
    expect(recipeCss).toMatch(
      /--surface-room:\s*var\(--palette-background-primary\)/,
    );
    expect(recipeCss).toMatch(
      /--surface-sidebar:\s*var\(--palette-background-secondary\)/,
    );
    expect(railCss).toContain("background:var(--surface-sidebar)");
    expect(railCss).toContain(
      ".channel.active{background:var(--surface-base);color:var(--text)}",
    );
    expect(roomCss).toContain("background:var(--surface-room)");
    expect(rosterCss).toContain("background:var(--surface-sidebar)");
  });
});

describe.each(PALETTES)("$label palette", (palette) => {
  it("publishes every palette role as a CSS variable", () => {
    const variables = paletteCssVariables(palette);
    expect(Object.keys(variables)).toHaveLength(21);
    expect(variables["--palette-background-darkest"]).toBe(
      palette.colors.backgrounds.darkest,
    );
    expect(variables["--palette-background-floating"]).toBe(
      palette.colors.backgrounds.floating,
    );
    expect(variables["--palette-text-faint"]).toBe(palette.colors.text.faint);
    expect(variables["--palette-info"]).toBe(palette.colors.semantic.info);
  });

  it("keeps small interface text readable across its dark surfaces", () => {
    const foregrounds = [
      palette.colors.text.normal,
      palette.colors.text.muted,
      palette.accessibleFaint,
    ];
    const backgrounds = Object.values(palette.colors.backgrounds);
    const failures = foregrounds.flatMap((foreground) =>
      backgrounds
        .filter((background) => contrast(foreground, background) < 4.5)
        .map(
          (background) =>
            `${foreground} on ${background}: ${contrast(foreground, background).toFixed(2)}:1`,
        ),
    );
    expect(failures).toEqual([]);
  });

  it("chooses a legible foreground for its accent fill", () => {
    const foreground = readableTextOn(palette.colors.accent);
    expect(contrast(foreground, palette.colors.accent)).toBeGreaterThanOrEqual(4.5);
  });
});

describe("Evergreen palette", () => {
  it("preserves the supplied full colour vocabulary", () => {
    const evergreen = PALETTES.find(({ id }) => id === "evergreen");
    expect(evergreen?.colors).toEqual({
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
    });
  });
});

describe("stored preference", () => {
  it("rejects malformed custom accent values", () => {
    expect(
      normaliseTheme({ ...DEFAULT_THEME, palette: "evergreen", accent: "red;" })
        .accent,
    ).toBe("#1db954");
  });
});

/** A custom palette as it is stored: Ink's colours under another name. */
function ownTheme(overrides: Partial<Record<string, unknown>> = {}): Record<string, unknown> {
  return { ...makeCustomTheme({ name: "Nocny", colors: copyColours(getPalette("ink").colors) }), ...overrides };
}

describe("custom palette", () => {
  it("is held to the same hex-only gate the presets pass", () => {
    for (const palette of PALETTES) {
      expect(normaliseColours(palette.colors)).toEqual(palette.colors);
    }
  });

  it.each([
    ["a colour name", "red"],
    ["a function", "rgb(0 0 0)"],
    ["an address", "url(https://example.com/x.png)"],
    ["an address hidden after a colour", "#000000 url(https://example.com/x.png)"],
    ["a variable", "var(--palette-accent)"],
    ["three digits", "#fff"],
    ["a declaration break", "#000000;background:url(x)"],
    ["a number", 0],
  ])("drops the whole palette when one role is %s", (_label, value) => {
    const colors = copyColours(getPalette("ink").colors) as unknown as Record<string, Record<string, unknown>>;
    colors["backgrounds"]!["hover"] = value;
    const theme = normaliseTheme({ ...DEFAULT_THEME, palette: "custom", custom: ownTheme({ colors }) });

    expect(theme.custom).toBeNull();
    expect(theme.palette).toBe(DEFAULT_THEME.palette);
  });

  it.each([
    ["a translucent accent", { colors: { ...getPalette("ink").colors, accent: "#58a6ff80" } }],
    ["no derived faint colour", { accessibleFaint: undefined }],
    ["a translucent derived faint colour", { accessibleFaint: "#a8b1bb80" }],
    ["a tint out of range", { neutral: { hue: 400, chroma: 1 } }],
    ["a tint that is not a number", { neutral: { hue: 200, chroma: "1;" } }],
  ])("drops a palette with %s", (_label, overrides) => {
    expect(normaliseCustomTheme(ownTheme(overrides))).toBeNull();
  });

  it("keeps only the fields and roles it knows", () => {
    const extra = normaliseCustomTheme(ownTheme({ script: "alert(1)", colors: { ...getPalette("ink").colors, extra: "#000000" } }));
    expect(extra).not.toHaveProperty("script");
    expect(extra?.colors).not.toHaveProperty("extra");
  });

  it("caps the words a file brings and keeps only an http(s) source", () => {
    const custom = normaliseCustomTheme(
      ownTheme({
        name: ` ${"n".repeat(200)}${String.fromCodePoint(0x202e)}`,
        author: `${String.fromCodePoint(7)}Ada`,
        source: "javascript:alert(1)",
        basedOn: "nope",
      }),
    );
    expect(custom?.name).toBe("n".repeat(CUSTOM_NAME_MAX));
    expect(custom?.author).toBe("Ada");
    expect(custom).not.toHaveProperty("source");
    expect(custom).not.toHaveProperty("basedOn");
    expect(normaliseCustomTheme(ownTheme({ source: `https://example.com/${"a".repeat(CUSTOM_SOURCE_MAX)}` }))).not.toHaveProperty("source");
    expect(normaliseCustomTheme(ownTheme({ source: "https://github.com/a/b" }))?.source).toBe("https://github.com/a/b");
  });

  it("stays far under the account's limit for a section at its largest", () => {
    // Characters JSON escapes, at every length the gate allows.
    const colors = copyColours(getPalette("harbor").colors) as unknown as Record<string, Record<string, string>>;
    for (const group of ["backgrounds", "text", "borders", "semantic"]) {
      for (const role of Object.keys(colors[group]!)) colors[group]![role] = "#ffffff0f";
    }
    const theme = normaliseTheme({
      ...DEFAULT_THEME,
      palette: "custom",
      custom: ownTheme({
        name: '"\\'.repeat(CUSTOM_NAME_MAX),
        author: '"\\'.repeat(CUSTOM_NAME_MAX),
        source: `https://example.com/${"%22".repeat((CUSTOM_SOURCE_MAX - 20) / 3)}`,
        basedOn: "harbor",
        colors,
        neutral: { hue: 358.123456789, chroma: 19.123456789 },
      }),
    });

    expect(theme.custom?.source).toBeDefined();
    expect(JSON.stringify(appearanceToWire(theme)).length).toBeLessThan(CLIENT_SETTINGS_SECTION_MAX / 4);
  });

  it("is kept while a preset is selected, so the person can go back to it", () => {
    const theme = normaliseTheme({ ...DEFAULT_THEME, palette: "slate", custom: ownTheme() });
    expect(theme.palette).toBe("slate");
    expect(theme.custom?.name).toBe("Nocny");
  });

  it("takes its own accent when the stored one is not a colour", () => {
    const custom = ownTheme({ colors: { ...getPalette("ink").colors, accent: "#ff8800" } });
    expect(normaliseTheme({ palette: "custom", custom, accent: "nope" }).accent).toBe("#ff8800");
  });

  it("writes its light tint inline and takes it back off for a preset", () => {
    const element = document.createElement("html");
    const custom = normaliseCustomTheme(ownTheme());
    applyThemeAttributes(element, { ...DEFAULT_THEME, palette: "custom", custom }, "light");
    expect(element.dataset["theme"]).toBe("custom");
    expect(element.style.getPropertyValue("--h-neutral")).toBe(String(custom?.neutral.hue));

    applyThemeAttributes(element, { ...DEFAULT_THEME, palette: "ink", custom }, "light");
    expect(element.style.getPropertyValue("--h-neutral")).toBe("");
    expect(element.style.getPropertyValue("--c-neutral")).toBe("");
  });
});

describe("derived values", () => {
  it.each(PALETTES)("come close to what $label tunes by hand", (palette) => {
    const derived = makeCustomTheme({ name: palette.label, colors: palette.colors });
    const tuned = palettesCss.get(`[data-theme="${palette.id}"]`);
    const lightness = (hex: string) => toOklch(parseHex(hex))[0];

    // Never under AA on any of its backgrounds, and close to the tuned value.
    for (const background of Object.values(palette.colors.backgrounds)) {
      expect(contrastRatio(parseHex(derived.accessibleFaint), flatten(parseHex(background), [0, 0, 0, 1]))).toBeGreaterThanOrEqual(4.5);
    }
    expect(Math.abs(lightness(derived.accessibleFaint) - lightness(palette.accessibleFaint))).toBeLessThan(0.1);
    expect(Math.abs(derived.neutral.hue - Number(tuned?.get("h-neutral")))).toBeLessThanOrEqual(6);
    expect(Math.abs(derived.neutral.chroma - Number(tuned?.get("c-neutral")))).toBeLessThan(1);
  });

  it("leave a faint colour that already reads alone and darken one on a light palette", () => {
    expect(deriveFaint("#ffffff", "#ffffff", ["#000000"])).toBe("#ffffff");
    const onLight = deriveFaint("#dddddd", "#111111", ["#ffffff", "#f4f4f4"]);
    expect(contrastRatio(parseHex(onLight), parseHex("#f4f4f4"))).toBeGreaterThanOrEqual(4.5);
  });

  it("read a grey background as untinted", () => {
    expect(deriveNeutral("#1a1a1a", "#0a0a0a")).toEqual({ hue: 0, chroma: 0 });
  });
});
