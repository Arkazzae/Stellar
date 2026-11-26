import { describe, expect, it } from "vitest";

import { forkPreset, withColour } from "@/design/customTheme";
import {
  THEME_FILE_MAX_BYTES,
  readThemeFile,
  themeFileName,
  themeFileOf,
  themeFileText,
} from "@/design/themeFile";
import { DEFAULT_THEME, getPalette, normaliseTheme } from "@/design/themes";

/** A file read back as loosely as a hand-edited one. */
type Loose = { colors: Record<string, Record<string, unknown> | undefined>; accent: unknown };

const OWN = {
  ...withColour(forkPreset("ink"), "borders", "subtle", "#ff00ff40"),
  name: "Port",
  author: "Ada",
  source: "https://example.com/port",
};

describe("theme file", () => {
  it("brings back the palette, accent and words it was saved with, and nothing personal", () => {
    const saved = { ...DEFAULT_THEME, palette: "custom" as const, custom: OWN, accent: "#ff3366", size: "lg" as const, contrast: "high" as const, font: "figtree" as const };
    const text = themeFileText(themeFileOf(saved));
    expect(JSON.parse(text)).not.toHaveProperty("size");
    expect(JSON.parse(text)).not.toHaveProperty("font");
    expect(JSON.parse(text)).not.toHaveProperty("contrast");

    const read = readThemeFile(text);
    expect(read.ok).toBe(true);
    if (!read.ok) return;
    // Which preset it came from is this device's history, not part of the
    // theme, and the one accent a file carries is the palette's own colour too.
    const { basedOn: _basedOn, ...rest } = OWN;
    const shared = { ...rest, colors: { ...rest.colors, accent: "#ff3366" } };
    expect(read.patch).toEqual({ palette: "custom", custom: shared, accent: "#ff3366", scheme: "dark" });
    expect(normaliseTheme({ ...DEFAULT_THEME, ...read.patch }).custom).toEqual(shared);
  });

  it("carries a wallpaper as bytes with its shading, never as an address to use", async () => {
    const bytes = new Uint8Array([0x52, 0x49, 0x46, 0x46, 1, 2, 3, 250]);
    const image = `data:image/webp;base64,${btoa(String.fromCharCode(...bytes))}`;
    const theme = { ...DEFAULT_THEME, wallpaper: { shade: 65, blur: 4, placeholder: "AAAA" } };
    const file = themeFileOf(theme, image);
    expect(file.wallpaper).toEqual({ image, shade: 65, blur: 4 });

    const read = readThemeFile(themeFileText(file));
    if (!read.ok || read.wallpaper === undefined) throw new Error("the wallpaper did not come back");
    expect(read.patch).not.toHaveProperty("wallpaper");
    expect({ shade: read.wallpaper.shade, blur: read.wallpaper.blur }).toEqual({ shade: 65, blur: 4 });
    expect(read.wallpaper.image.type).toBe("image/webp");
    expect(new Uint8Array(await read.wallpaper.image.arrayBuffer())).toEqual(bytes);
  });

  it.each([
    ["a vector picture", "data:image/svg+xml;base64,PHN2Zy8+"],
    ["an address", "https://example.com/wallpaper.jpg"],
    ["markup after the picture", "data:image/png;base64,AAAA\");background:url(x"],
  ])("leaves out a wallpaper that is %s and keeps the rest", (_label, image) => {
    const file = { ...themeFileOf(DEFAULT_THEME), wallpaper: { image, shade: 50, blur: 0 } };
    const read = readThemeFile(JSON.stringify(file));
    expect(read.ok).toBe(true);
    expect(read).not.toHaveProperty("wallpaper");
  });

  it("reads older files while ignoring their font choice", () => {
    const file = { ...themeFileOf(DEFAULT_THEME), font: "figtree" };
    const read = readThemeFile(JSON.stringify(file));
    expect(read.ok).toBe(true);
    if (read.ok) expect(read.patch).not.toHaveProperty("font");
  });

  it("writes a preset under its own name", () => {
    const file = themeFileOf({ ...DEFAULT_THEME, palette: "harbor", accent: "#1a9fff" });
    expect(file).toMatchObject({ stellarrTheme: 1, name: "Harbor", accent: "#1a9fff" });
    expect(file.colors.backgrounds).toEqual(getPalette("harbor").colors.backgrounds);
  });

  it.each([
    ["text that is not JSON", "{oops"],
    ["JSON that is not ours", JSON.stringify({ name: "x", colors: {} })],
    ["a file of another kind", JSON.stringify({ stellarrTheme: "yes" })],
  ])("refuses %s as not a theme file", (_label, text) => {
    expect(readThemeFile(text)).toEqual({ ok: false, problem: "notOurs" });
  });

  it.each([
    ["a colour with an address", (file: Loose) => (file.colors.backgrounds!.primary = "url(https://example.com/x)")],
    ["a named colour", (file: Loose) => (file.colors.text!.normal = "white")],
    ["a translucent accent", (file: Loose) => (file.accent = "#ff336680")],
    ["a missing group", (file: Loose) => delete file.colors.semantic],
  ])("refuses %s and changes nothing", (_label, spoil) => {
    const file = themeFileOf({ ...DEFAULT_THEME, palette: "custom", custom: OWN }) as unknown as Loose;
    spoil(file);
    expect(readThemeFile(JSON.stringify(file))).toEqual({ ok: false, problem: "colours" });
  });

  it("refuses a file too large to be a theme before reading it", () => {
    expect(readThemeFile(" ".repeat(THEME_FILE_MAX_BYTES + 1))).toEqual({ ok: false, problem: "tooLarge" });
  });

  it("names the file after the theme, without what a file system refuses", () => {
    expect(themeFileName('Nocny / port:  wersja "2"', "Motyw")).toBe("Nocny port wersja 2.stellarr-theme.json");
    expect(themeFileName("..", "Motyw")).toBe("Motyw.stellarr-theme.json");
  });
});
