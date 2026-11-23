import { afterEach, describe, expect, it, vi } from "vitest";

import { parseHex } from "@/design/colour";

import { convertTheme, stylesheetAddress } from "./convert";
import { browserEvaluator, type Evaluation, type Evaluator } from "./evaluate";
import { DISCORD_ROLES, mapDiscordTheme } from "./roles";
import {
  inlineImports,
  readMeta,
  styleQueryNames,
  unwrapStyleQueries,
  wallpaperAddressOf,
  withoutAddresses,
} from "./text";

/*
 * Small handwritten themes, one per pattern the real ones use: a palette on
 * `:root`, Discord's variables on `.theme-dark`, a build behind an import,
 * colours gated by `@container root style(--colors: on)`, relative colours,
 * and a picture behind it all.
 */
const MIDNIGHT_LIKE = `/**
 * @name nocturne
 * @author ada
 * @version 1.2.0
 * @source https://github.com/ada/nocturne
 */
@import url('https://ada.github.io/nocturne/build.css');
:root { --colors: on; --bg-4: hsl(220, 15%, 10%); --text-3: hsl(220, 20%, 60%); }
body { --font: 'figtree'; }`;

const BUILD = `@import url('https://fonts.googleapis.com/css2?family=Figtree:wght@400;700&display=swap');
:root { container-name: root; }
@container root style(--colors: on) {
  body, .theme-dark:not(.custom-user-profile-theme) {
    --background-base-low: var(--bg-4);
    --text-default: var(--text-3);
  }
}
@font-face { font-family: X; src: url(https://ada.github.io/x.woff2); }
.icon { background: url("https://ada.github.io/i.svg") no-repeat, image-set("a.png" 1x, "b.png" 2x); }`;

const CLEARVISION_LIKE = `:root {
  --main-color: #2780e6;
  --background-image: url(https://clearvision.github.io/images/sapphire.jpg);
  --background-shading: rgba(0, 0, 0, 0.4);
}
.theme-dark { --text-brand: hsl(from var(--main-color) h s l); }`;

function fetcher(sheets: Record<string, string>) {
  return vi.fn(async (url: string) => {
    const sheet = sheets[url];
    if (sheet === undefined) throw new Error(`no ${url}`);
    return sheet;
  });
}

describe("reading a theme's text", () => {
  it("reads the META header", () => {
    expect(readMeta(MIDNIGHT_LIKE)).toEqual({
      name: "nocturne",
      author: "ada",
      version: "1.2.0",
      source: "https://github.com/ada/nocturne",
    });
    expect(readMeta("body { color: red }")).toEqual({});
  });

  it("writes chained imports in place, keeps Google Fonts' families and never fetches them", async () => {
    const fetchSheet = fetcher({
      "https://ada.github.io/nocturne/build.css": `@import "parts/colours.css" screen;\n${BUILD}`,
      "https://ada.github.io/nocturne/parts/colours.css": ":root { --accent-2: #46aec5; }",
    });

    const inlined = await inlineImports(MIDNIGHT_LIKE, "https://raw.githubusercontent.com/ada/nocturne/main/n.theme.css", fetchSheet);

    expect(fetchSheet).toHaveBeenCalledTimes(2);
    expect(inlined.css).toContain("@media screen {\n:root { --accent-2: #46aec5; }\n}");
    expect(inlined.css).toContain("--background-base-low: var(--bg-4)");
    expect(inlined.css).not.toMatch(/@import/);
    expect(inlined.googleFonts).toEqual(["Figtree"]);
    expect(inlined.fetched).toHaveLength(3);
  });

  it("stops at the depth and size it is given and leaves out what is not https", async () => {
    const fetchSheet = fetcher({
      "https://a.example/1.css": '@import "2.css";',
      "https://a.example/2.css": '@import "3.css";',
      "https://a.example/3.css": "x{}",
    });
    const deep = await inlineImports('@import "https://a.example/1.css"; @import "http://a.example/plain.css";', null, fetchSheet, {
      depth: 2,
      total: 1000,
    });
    expect(deep.skipped).toEqual([
      { url: "https://a.example/3.css", reason: "depth" },
      { url: "http://a.example/plain.css", reason: "insecure" },
    ]);

    const big = await inlineImports('@import "https://a.example/3.css";', null, fetcher({ "https://a.example/3.css": "y".repeat(500) }), {
      depth: 4,
      total: 100,
    });
    expect(big.skipped).toEqual([{ url: "https://a.example/3.css", reason: "size" }]);
  });

  it("takes out everything that would make a request before anything evaluates it", () => {
    const clean = withoutAddresses(`${BUILD}\n@import url(https://x.example/a.css);\n:root{--background-image:url( 'https://x.example/b.png' )}`);
    expect(clean).not.toMatch(/url\(|@font-face|@import|image-set|https:/);
    expect(clean).toContain("--background-base-low: var(--bg-4)");
  });

  it("finds the picture a theme puts behind Discord, whatever it calls it, and only a raster one", () => {
    expect(wallpaperAddressOf(CLEARVISION_LIKE)).toBe("https://clearvision.github.io/images/sapphire.jpg");
    expect(wallpaperAddressOf(":root{--app-bg: url(https://i.ibb.co/x/alps.png)}")).toBe("https://i.ibb.co/x/alps.png");
    expect(wallpaperAddressOf(":root{--background-image-url: url('')} --dms-icon-svg-url: url(https://a.example/moon.svg)")).toBeNull();
  });

  it("decides style queries against the values the theme declares", () => {
    const css = `@container root style(--colors: on) { a { --x: 1 } }
@container body style(--blur: on) and style(--size: big) { b { --y: 2 } }
@container body not style(--blur: on) { c { --z: 3 } }
@container (min-width: 400px) { d { --w: 4 } }`;
    const values: Record<string, string | undefined> = { "root:--colors": "on", "body:--blur": "on", "body:--size": "small" };
    const declared = (name: string, container: string | null) => values[`${container}:${name}`];

    const { css: unwrapped, undecided } = unwrapStyleQueries(css, declared);

    expect(unwrapped).toContain("a { --x: 1 }");
    expect(unwrapped).not.toContain("--y");
    expect(unwrapped).not.toContain("--z");
    expect(unwrapped).toContain("@container (min-width: 400px)");
    expect(undecided).toBe(1);
    expect(styleQueryNames(css)).toEqual(["--colors", "--blur", "--size"]);
  });
});

describe("Discord's variables as our palette", () => {
  const rgba = (hex: string) => parseHex(hex);

  it("reads the 2025 names and the older ones alike", () => {
    const now = mapDiscordTheme(new Map([
      ["--background-base-lowest", rgba("#0a0b0d")],
      ["--background-base-low", rgba("#16181d")],
      ["--text-default", rgba("#8592ad")],
      ["--brand-500", rgba("#46aec5")],
    ]));
    const before = mapDiscordTheme(new Map([
      ["--background-tertiary", rgba("#0a0b0d")],
      ["--background-primary", rgba("#16181d")],
      ["--text-normal", rgba("#8592ad")],
      ["--brand-experiment", rgba("#46aec5")],
    ]));
    expect(now.colors?.backgrounds.primary).toBe("#16181d");
    expect(before.colors?.backgrounds).toEqual(now.colors?.backgrounds);
    expect(before.colors?.text.normal).toBe("#8592ad");
    expect(before.accent).toBe("#46aec5");
    expect(now.roles.find((role) => role.role === "primary")?.from).toBe("--background-base-low");
  });

  it("flattens a translucent background onto the theme's darker one and keeps a border's alpha", () => {
    const mapped = mapDiscordTheme(new Map([
      ["--background-base-lowest", rgba("#000000")],
      ["--background-base-low", rgba("#ffffff80")],
      ["--text-default", rgba("#eeeeee")],
      ["--border-subtle", rgba("#ffffff14")],
    ]));
    expect(mapped.colors?.backgrounds.primary).toBe("#808080");
    expect(mapped.colors?.borders.subtle).toBe("#ffffff14");
  });

  it("makes no palette without the theme's backgrounds and text, and offers only what came across", () => {
    const mapped = mapDiscordTheme(new Map([["--text-brand", rgba("#2780e6")], ["--text-default", rgba("#dddddd")]]));
    expect(mapped.colors).toBeUndefined();
    expect(mapped.accent).toBe("#2780e6");
    expect(mapped.missingCore).toEqual(["backgrounds.primary", "backgrounds.darkest"]);
  });

  it("takes a picture theme's shading for its panels, and says so", () => {
    const resolved = new Map([["--text-default", rgba("#dddddd")], ["--background-shading", parseHex("#00000066")]]);
    expect(mapDiscordTheme(resolved).colors).toBeUndefined();
    const mapped = mapDiscordTheme(resolved, true);
    expect(mapped.backgroundsFromShading).toBe(true);
    expect(mapped.colors?.backgrounds.primary).toBe("#000000");
    expect(mapped.roles.find((role) => role.role === "primary")?.from).toBe("--background-shading");

    // One that names no shading: the black under its picture is ours, not read from it.
    const unshaded = mapDiscordTheme(new Map([["--text-default", rgba("#dddddd")]]), true);
    expect(unshaded.colors?.backgrounds.primary).toBe("#000000");
    expect(unshaded.roles.filter((role) => role.group === "backgrounds" && role.from !== undefined)).toEqual([]);
  });

  it("checks against midnight's remap: each role reads the variable midnight says paints it", () => {
    expect(DISCORD_ROLES.backgrounds.primary[0]).toBe("--background-base-low");
    expect(DISCORD_ROLES.text.normal[0]).toBe("--text-default");
    expect(DISCORD_ROLES.text.faint[0]).toBe("--text-muted");
    expect(DISCORD_ROLES.backgrounds.hover[0]).toBe("--background-mod-subtle");
  });
});

/** An evaluator that answers from fixed colours and records what it was given. */
function stubEvaluator(answers: (css: string) => Partial<Evaluation>): Evaluator & { seen: string[] } {
  const seen: string[] = [];
  return {
    seen,
    async evaluate(css) {
      seen.push(css);
      const answer = answers(css);
      return {
        colours: answer.colours ?? new Map(),
        values: answer.values ?? new Map(),
        containers: answer.containers ?? { root: new Map(), body: new Map() },
        styleQueries: answer.styleQueries ?? true,
      };
    },
  };
}

describe("converting a theme", () => {
  const sheets = {
    "https://raw.githubusercontent.com/ada/nocturne/main/n.theme.css": MIDNIGHT_LIKE,
    "https://ada.github.io/nocturne/build.css": BUILD,
  };
  const colours = new Map([
    ["--background-base-low", parseHex("#16181d")],
    ["--text-default", parseHex("#8592ad")],
    ["--background-base-lowest", parseHex("#0a0b0d")],
  ]);

  it("evaluates a stylesheet without addresses and maps what it sets", async () => {
    const evaluator = stubEvaluator(() => ({ colours, values: new Map([["--font", "'figtree'"]]) }));

    const conversion = await convertTheme(MIDNIGHT_LIKE, "https://raw.githubusercontent.com/ada/nocturne/main/n.theme.css", {
      fetchSheet: fetcher(sheets),
      evaluator,
    });

    expect(evaluator.seen).toHaveLength(1);
    expect(evaluator.seen[0]).not.toMatch(/url\(|https:/);
    expect(conversion.palette?.backgrounds.primary).toBe("#16181d");
    expect(conversion.font).toEqual({ family: "figtree", id: "figtree" });
    expect(conversion.styleQueries).toBe("native");
    expect(conversion.meta.name).toBe("nocturne");
  });

  it("decides the style queries itself where the engine cannot, from the theme's own values", async () => {
    const evaluator = stubEvaluator((css) =>
      css.includes("@container root")
        ? { styleQueries: false, containers: { root: new Map([["--colors", "on"]]), body: new Map() } }
        : { colours, styleQueries: false },
    );

    const conversion = await convertTheme(MIDNIGHT_LIKE, "https://raw.githubusercontent.com/ada/nocturne/main/n.theme.css", {
      fetchSheet: fetcher(sheets),
      evaluator,
    });

    expect(evaluator.seen).toHaveLength(2);
    expect(evaluator.seen[1]).not.toContain("@container root");
    expect(evaluator.seen[1]).toContain("--background-base-low: var(--bg-4)");
    expect(conversion.styleQueries).toBe("unwrapped");
    expect(conversion.palette).toBeDefined();
  });

  it("names a picture theme's wallpaper and host and its shading, without fetching it", async () => {
    const fetchSheet = fetcher({});
    const evaluator = stubEvaluator(() => ({
      colours: new Map([["--text-default", parseHex("#dcdcde")], ["--text-brand", parseHex("#2780e6")], ["--background-shading", parseHex("#00000066")]]),
    }));

    const conversion = await convertTheme(CLEARVISION_LIKE, null, { fetchSheet, evaluator });

    expect(fetchSheet).not.toHaveBeenCalled();
    expect(conversion.wallpaper).toEqual({ url: "https://clearvision.github.io/images/sapphire.jpg", host: "clearvision.github.io", shade: 40 });
    expect(conversion.backgroundsFromShading).toBe(true);
    expect(conversion.accent).toBe("#2780e6");
  });

  it("reads a GitHub page as the file and says plainly that betterdiscord.app cannot be read", () => {
    expect(stylesheetAddress("https://github.com/ada/nocturne/blob/main/themes/n.theme.css")).toEqual({
      url: "https://raw.githubusercontent.com/ada/nocturne/main/themes/n.theme.css",
    });
    expect(stylesheetAddress("https://betterdiscord.app/download?id=174")).toEqual({ problem: "betterDiscordSite" });
    expect(stylesheetAddress("http://ada.github.io/n.css")).toEqual({ problem: "notHttps" });
  });
});

describe("the browser evaluator", () => {
  afterEach(() => {
    document.querySelectorAll("iframe").forEach((frame) => frame.remove());
  });

  it("never adds a style to the app's document and leaves no frame behind", async () => {
    const before = document.styleSheets.length;
    const styles = document.querySelectorAll("style").length;

    await browserEvaluator()
      .evaluate(":root{--background-base-low:#123456} body{background:red}", ["--background-base-low"], [])
      .catch(() => undefined);

    expect(document.styleSheets.length).toBe(before);
    expect(document.querySelectorAll("style")).toHaveLength(styles);
    expect(document.querySelector("iframe")).toBeNull();
    expect(getComputedStyle(document.body).backgroundColor).not.toBe("rgb(255, 0, 0)");
  });
});
