import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { contrastRatio, flatten, parseHex, type Rgba } from "@/design/colour";
import { PALETTES, WALLPAPER_SHADE_DEFAULT, normaliseWallpaper } from "@/design/themes";
import { isVectorImage, wallpaperSize } from "@/lib/wallpaper/prepareWallpaper";

const STYLES = join(process.cwd(), "src/stellarr/styles");
const wallpaperCss = readFileSync(join(STYLES, "themes/wallpaper.css"), "utf8");

/** How opaque wallpaper.css makes a structural surface, as a fraction. */
function opacityOf(token: string): number {
  const match = new RegExp(`${token}:\\s*color-mix\\(in srgb, var\\([^)]+\\) (\\d+)%, transparent\\)`).exec(wallpaperCss);
  if (match === null) throw new Error(`wallpaper.css sets no opacity for ${token}`);
  return Number(match[1]) / 100;
}

const over = (top: Rgba, alpha: number, bottom: Rgba): Rgba => flatten([top[0], top[1], top[2], alpha], bottom);

/** An OKLCH lightness with no chroma, as sRGB: the light ramp's neutral surfaces. */
function grey(lightness: number): Rgba {
  const linear = lightness ** 3;
  const value = linear <= 0.0031308 ? 12.92 * linear : 1.055 * linear ** (1 / 2.4) - 0.055;
  return [value, value, value, 1];
}

describe("wallpaper readability", () => {
  /*
   * The worst picture there is: a white pixel under a dark theme and a black
   * one under a light theme. At the default shading, small text set in the
   * faint colour keeps the contrast the app holds every surface to, on the
   * channel rail and on the room, for every preset.
   */
  const shade = WALLPAPER_SHADE_DEFAULT / 100;
  const surfaces = { sidebar: opacityOf("--surface-sidebar"), room: opacityOf("--surface-room") };

  it.each(PALETTES)("keeps small text at AA over a white pixel in $label", (palette) => {
    const shaded = over(parseHex(palette.colors.backgrounds.darkest), shade, [1, 1, 1, 1]);
    const sidebar = over(parseHex(palette.colors.backgrounds.secondary), surfaces.sidebar, shaded);
    const room = over(parseHex(palette.colors.backgrounds.primary), surfaces.room, shaded);
    for (const surface of [sidebar, room]) {
      expect(contrastRatio(parseHex(palette.accessibleFaint), surface)).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("keeps small text at AA over a black pixel in the light scheme", () => {
    // recipe.css: canvas 0.975, raised 0.965, base 0.995, faint text 0.5.
    const shaded = over(grey(0.975), shade, [0, 0, 0, 1]);
    for (const [lightness, opacity] of [[0.965, surfaces.sidebar], [0.995, surfaces.room]] as const) {
      expect(contrastRatio(grey(0.5), over(grey(lightness), opacity, shaded))).toBeGreaterThanOrEqual(4.5);
    }
  });
});

describe("floating surfaces over a wallpaper", () => {
  /*
   * What floats paints with tokens a wallpaper never makes see-through. These
   * are the surfaces a person reads over the shell: if one of them took a
   * structural token, the wallpaper and the panels under it would show
   * through its text.
   */
  const SEE_THROUGH = /var\(--surface-(?:canvas|sidebar|room)\)/;
  const FLOATING = [
    ".ctx",
    ".anchored",
    ".am-card",
    ".dock-pop",
    ".emo-pick",
    ".gif-panel",
    ".fmt-pop",
    ".mention-suggestions",
    ".notification-inbox-header",
    ".person-more",
    ".ch-place",
    ".pv-rail",
    ".pv-room",
    ".rail-scroll [data-drag=\"carried\"]",
    ".att-window",
  ];
  const css = readdirSync(join(STYLES, "components"))
    .map((file) => readFileSync(join(STYLES, "components", file), "utf8"))
    .join("\n")
    .replace(/\/\*[\s\S]*?\*\//g, "");

  it.each(FLOATING)("%s never paints with a see-through token", (selector) => {
    const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter(([, selectors]) =>
      (selectors ?? "").split(",").some((one) => one.trim() === selector),
    );
    expect(rules.length).toBeGreaterThan(0);
    for (const [, , body] of rules) {
      for (const declaration of (body ?? "").split(";")) {
        if (/^\s*background/.test(declaration)) expect(declaration).not.toMatch(SEE_THROUGH);
      }
    }
  });

  it("gives the settings window's side the canvas's opaque twin", () => {
    expect(css).toMatch(/--set-side:color-mix\(in oklab, var\(--surface-canvas-solid\)/);
  });
});

describe("wallpaper settings", () => {
  it("clamps what storage or a file gives back and keeps only a placeholder of its size", () => {
    expect(normaliseWallpaper({ shade: 140, blur: -3, placeholder: "url(x)" })).toEqual({ shade: 100, blur: 0 });
    expect(normaliseWallpaper({ shade: "dark" })).toEqual({ shade: WALLPAPER_SHADE_DEFAULT, blur: 0 });
    expect(normaliseWallpaper({ shade: 42.4, blur: 7.6, placeholder: "AbC+/=" })).toEqual({ shade: 42, blur: 8, placeholder: "AbC+/=" });
    expect(normaliseWallpaper(null)).toBeNull();
    expect(normaliseWallpaper("yes")).toBeNull();
  });
});

describe("wallpaper preparation", () => {
  it("fits a picture inside 2560 pixels and never enlarges one", () => {
    expect(wallpaperSize(6000, 4000)).toEqual({ width: 2560, height: 1707 });
    expect(wallpaperSize(1000, 3000)).toEqual({ width: 853, height: 2560 });
    expect(wallpaperSize(1920, 1080)).toEqual({ width: 1920, height: 1080 });
  });

  it("tells an SVG by its type or by how it starts, whatever it is called", () => {
    const text = (value: string) => new TextEncoder().encode(value);
    expect(isVectorImage(text("<svg xmlns='http://www.w3.org/2000/svg'/>"), "image/png")).toBe(true);
    expect(isVectorImage(text(`${String.fromCodePoint(0xfeff)}  <?xml version="1.0"?><svg/>`), "")).toBe(true);
    expect(isVectorImage(text(""), "image/svg+xml")).toBe(true);
    expect(isVectorImage(new Uint8Array([0x89, 0x50, 0x4e, 0x47]), "image/png")).toBe(false);
  });
});
