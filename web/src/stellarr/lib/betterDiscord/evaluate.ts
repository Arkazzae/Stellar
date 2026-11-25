import type { Rgba } from "@/design/colour";

/**
 * Evaluates a theme's stylesheet in a document of its own and reads what it
 * sets: an off-screen iframe, sandboxed with no scripts, whose content
 * security policy refuses every request, holding a skeleton of Discord's
 * root (`theme-dark`, `visual-refresh`, `#app-mount`): a theme with several
 * variants is read as Discord's dark theme shows it. The stylesheet goes in
 * as the text of its one `<style>`, so nothing in it can become markup. The
 * app's own document never sees a rule of it, and the iframe is removed
 * whatever happens.
 *
 * Colours are read through a probe property, so the browser itself resolves
 * `var()` chains, relative colours (`hsl(from …)`), `oklch()` and
 * `color-mix()`; the computed colour is then drawn on a canvas in this
 * document to get sRGB bytes, whatever notation the engine reports it in. A
 * variable that is not set, or not a colour, leaves the probe inheriting its
 * parent's colour, so every colour is read under two different parents and
 * one that follows them is not the theme's.
 */

export interface Evaluation {
  /** Each asked variable the theme sets to a colour, resolved to sRGB. */
  colours: Map<string, Rgba>;
  /** Each asked variable's text as the theme leaves it, after `var()` substitution. */
  values: Map<string, string>;
  /** The same, as <html> and <body> hold them: what a container named `root` or `body` is asked. */
  containers: { root: Map<string, string>; body: Map<string, string> };
  /** Whether the engine evaluated a container style query. */
  styleQueries: boolean;
}

export type DiscordVariant = "dark" | "darker" | "midnight" | "light";

export interface Evaluator {
  evaluate(css: string, colourNames: readonly string[], valueNames: readonly string[], variant?: DiscordVariant): Promise<Evaluation>;
}

function skeleton(variant: DiscordVariant): string {
  const theme = variant === "darker" || variant === "midnight" ? `theme-dark theme-${variant}` : `theme-${variant}`;
  // A theme may repaint a panel with one of Midnight's base colors.
  const midnight = variant === "midnight"
    ? "@layer stellarr-default{.theme-midnight{--background-base-lowest:#000;--background-base-lower:#0b0b0c;--background-base-low:#111214}}"
    : "";
  return `<!doctype html>
<html class="${theme} visual-refresh platform-web full-motion">
<head>
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'">
<style id="canary">:root{color-scheme:${variant === "light" ? "light" : "dark"};container-name:root;--stellarr-canary:on;--saturation-factor:1}@container root style(--stellarr-canary: on){body{--stellarr-canary-seen:yes}}${midnight}</style>
<style id="theme"></style>
</head>
<body class="${theme}">
<div id="app-mount" class="appMount__51fd7 appMount_gEkNtR appMount_ea7e65">
<div id="stellarr-app" class="app__160d8 app_a01fb1 app_YRXZu0 app_QbT-Jy app-2CXKsg visual-refresh">
<div id="stellarr-frame" class="bg__960e4 bg_d4b6c5 bg_Fd0FEH"></div>
<div class="layers__960e4 layers_d4b6c5">
<div id="stellarr-parent">
<div id="stellarr-probe"></div>
<nav id="stellarr-guilds" class="guilds_c48ade guilds_pog62O guilds__2b93a guilds-2JjMmN wrapper-1Rf91z"></nav>
<div id="stellarr-sidebar" class="sidebar_c48ade sidebar_a4d4d9 sidebar-1tnWFu sidebar_pog62O sidebar_cR4ic2"><div id="stellarr-channel-list" class="container__2637a container_H5ozaj channels-Ie2l6A"></div></div>
<div id="stellarr-chat" class="chat_f75fb0 chat_a7d72e chat-2ZfjoI chat_z_gUBg">
  <div class="content_f75fb0">
    <main id="stellarr-chat-content" class="chatContent_f75fb0 chatContent_a7d72e chatContent_z_gUBg chatContent-3KubbW">
      <div id="stellarr-message" class="messageContent_c19a55 markup__75297 markup-eYLPri messageContent-2t3eCI"></div>
    </main>
  </div>
  <div class="channelTextArea_f75fb0 channelTextArea__74017 channelTextArea_z_gUBg inner-zqa7da">
    <div id="stellarr-input" class="themedBackground__74017 themedBackground_2x8-Qn scrollableContainer__74017 scrollableContainer-15eg7h"></div>
  </div>
</div>
</div>
</div>
</div>
</div>
</body>
</html>`;
}

/** Two colours no theme would set, for telling an inherited colour from a set one. */
const SENTINELS = ["rgb(1, 2, 3)", "rgb(4, 5, 6)"] as const;

/** A computed colour as sRGB bytes 0–1, through a 1×1 canvas. */
export function colourToRgba(context: CanvasRenderingContext2D, colour: string): Rgba | null {
  context.clearRect(0, 0, 1, 1);
  context.fillStyle = "#010203";
  context.fillStyle = colour;
  // A string the canvas cannot parse leaves the fill as it was.
  if (context.fillStyle === "#010203" && !/^(#010203|rgba?\(\s*1,\s*2,\s*3)/i.test(colour)) return null;
  context.fillRect(0, 0, 1, 1);
  const [r = 0, g = 0, b = 0, a = 0] = context.getImageData(0, 0, 1, 1).data;
  return [r / 255, g / 255, b / 255, a / 255];
}

const MIDNIGHT_BASE = new Set(["--background-base-lowest", "--background-base-lower", "--background-base-low"]);

function gradientColour(context: CanvasRenderingContext2D, image: string): Rgba | null {
  if (!image.startsWith("linear-gradient(") || /rgb\(1, \d+, \d+\)/.test(image)) return null;
  // CSSOM has already resolved var() and the stop colors. Take a middle
  // visible stop as the solid equivalent of the gradient.
  const stops = image.match(/(?:rgba?|hsla?|oklch|oklab|lch|lab|color)\([^()]*\)/gi) ?? [];
  const colours = stops.map((stop) => colourToRgba(context, stop)).filter((value): value is Rgba => value !== null && value[3] >= 0.5);
  return colours[Math.floor(colours.length / 2)] ?? null;
}

function canvasContext(): CanvasRenderingContext2D | null {
  const canvas = document.createElement("canvas");
  canvas.width = 1;
  canvas.height = 1;
  return canvas.getContext("2d", { willReadFrequently: true });
}

function loaded(frame: HTMLIFrameElement): Promise<Document> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("The theme's document did not load.")), 10_000);
    frame.addEventListener(
      "load",
      () => {
        clearTimeout(timer);
        const doc = frame.contentDocument;
        if (doc === null) reject(new Error("The theme's document cannot be read."));
        else resolve(doc);
      },
      { once: true },
    );
  });
}

/** The evaluator that uses this browser. */
export function browserEvaluator(): Evaluator {
  return {
    async evaluate(css, colourNames, valueNames, variant = "dark") {
      const frame = document.createElement("iframe");
      frame.setAttribute("sandbox", "allow-same-origin");
      frame.setAttribute("aria-hidden", "true");
      frame.tabIndex = -1;
      frame.style.cssText = "position:fixed;left:-10000px;top:0;width:1280px;height:800px;border:0;visibility:hidden";
      frame.srcdoc = skeleton(variant);
      const ready = loaded(frame);
      document.body.append(frame);
      try {
        const doc = await ready;
        const sheet = doc.getElementById("theme");
        const probe = doc.getElementById("stellarr-probe");
        const parent = doc.getElementById("stellarr-parent");
        const view = doc.defaultView;
        if (sheet === null || probe === null || parent === null || view === null) {
          throw new Error("The theme's document is not the one it was given.");
        }
        const message = doc.getElementById("stellarr-message");
        const initialFont = view.getComputedStyle(message ?? doc.body).fontFamily;
        sheet.textContent = css;

        const styleQueries = view.getComputedStyle(doc.body).getPropertyValue("--stellarr-canary-seen").trim() === "yes";
        const read = (element: Element) => {
          const style = view.getComputedStyle(element);
          const found = new Map<string, string>();
          for (const name of valueNames) {
            const value = style.getPropertyValue(name).trim();
            if (value !== "") found.set(name, value);
          }
          return found;
        };
        const values = read(probe);
        const family = view.getComputedStyle(message ?? doc.body).fontFamily;
        if (family !== initialFont && family !== "") values.set("--stellarr-font", family);

        const backdropStyles: CSSStyleDeclaration[] = [];
        for (const id of ["stellarr-frame", "stellarr-app", "app-mount", "stellarr-chat-content", "stellarr-chat"]) {
          const element = doc.getElementById(id);
          if (element === null) continue;
          backdropStyles.push(view.getComputedStyle(element));
          // Older themes paint the app wallpaper on a generated layer.
          for (const pseudo of ["::before", "::after"]) {
            const style = view.getComputedStyle(element, pseudo);
            if (style.content !== "none" && style.content !== "normal") backdropStyles.push(style);
          }
        }
        backdropStyles.push(view.getComputedStyle(doc.body));
        const backdrop = backdropStyles.find((style) => /rgb\(1, \d+, \d+\)/.test(style.backgroundImage));
        if (backdrop !== undefined) values.set("--stellarr-page-image", backdrop.backgroundImage);
        const containers = { root: read(doc.documentElement), body: read(doc.body) };

        const context = canvasContext();
        const colours = new Map<string, Rgba>();
        if (context !== null) {
          const surfaces = {
            "--stellarr-frame": ["stellarr-frame", "stellarr-guilds", "stellarr-app", "app-mount"],
            "--stellarr-sidebar": ["stellarr-channel-list", "stellarr-sidebar"],
            "--stellarr-chat": ["stellarr-chat-content", "stellarr-chat"],
            "--stellarr-input": ["stellarr-input"],
          };
          for (const [name, ids] of Object.entries(surfaces)) {
            for (const id of ids) {
              const element = doc.getElementById(id);
              if (element === null) continue;
              const style = view.getComputedStyle(element);
              const rgba = colourToRgba(context, style.backgroundColor);
              const paint = rgba !== null && rgba[3] > 0 ? rgba : gradientColour(context, style.backgroundImage);
              if (paint === null) continue;
              colours.set(name, paint);
              break;
            }
          }
          const body = view.getComputedStyle(doc.body);
          const bodyColour = colourToRgba(context, body.backgroundColor);
          const bodyGradient = gradientColour(context, body.backgroundImage);
          const bodyPaint = bodyColour !== null && bodyColour[3] > 0 ? bodyColour : bodyGradient;
          if (bodyPaint !== null && !colours.has("--stellarr-frame")) colours.set("--stellarr-frame", bodyPaint);
          if (bodyGradient !== null) colours.set("--stellarr-backdrop", bodyGradient);

          if (message !== null) {
            const seen = SENTINELS.map((sentinel) => {
              parent.style.setProperty("color", sentinel, "important");
              return view.getComputedStyle(message).color;
            });
            if (seen[0] !== undefined && seen[0] === seen[1]) {
              const rgba = colourToRgba(context, seen[0]);
              if (rgba !== null) colours.set("--stellarr-message", rgba);
            }
          }
          for (const name of colourNames) {
            if (name.startsWith("--stellarr-")) continue;
            if (variant === "midnight" && MIDNIGHT_BASE.has(name) && !new RegExp(`${name}\\s*:`).test(css)) continue;
            const seen = SENTINELS.map((sentinel) => {
              parent.style.setProperty("color", sentinel, "important");
              probe.style.setProperty("color", `var(${name})`, "important");
              return view.getComputedStyle(probe).color;
            });
            if (seen[0] === undefined || seen[0] !== seen[1]) continue;
            const rgba = colourToRgba(context, seen[0]);
            if (rgba !== null) colours.set(name, rgba);
          }
        }
        return { colours, values, containers, styleQueries };
      } finally {
        frame.remove();
      }
    },
  };
}
