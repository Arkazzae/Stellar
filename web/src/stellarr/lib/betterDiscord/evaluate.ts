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

export type DiscordVariant = "dark" | "midnight" | "light";

export interface Evaluator {
  evaluate(css: string, colourNames: readonly string[], valueNames: readonly string[], variant?: DiscordVariant): Promise<Evaluation>;
}

function skeleton(variant: DiscordVariant): string {
  const theme = variant === "midnight" ? "theme-dark theme-midnight" : `theme-${variant}`;
  return `<!doctype html>
<html class="${theme} visual-refresh platform-web full-motion">
<head>
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'">
<style id="canary">:root{color-scheme:${variant === "light" ? "light" : "dark"};container-name:root;--stellarr-canary:on;--saturation-factor:1}@container root style(--stellarr-canary: on){body{--stellarr-canary-seen:yes}}</style>
<style id="theme"></style>
</head>
<body class="${theme}">
<div id="app-mount" class="appMount__51fd7 appMount_gEkNtR">
<div id="stellarr-app" class="app__160d8 app_a01fb1 visual-refresh">
<div id="stellarr-frame" class="bg__960e4 bg_d4b6c5 bg_Fd0FEH"></div>
<div class="layers__960e4 layers_d4b6c5">
<div id="stellarr-parent">
<div id="stellarr-probe"></div>
<div id="stellarr-sidebar" class="sidebar_c48ade sidebar_a4d4d9 sidebar-1tnWFu container__2637a"></div>
<div id="stellarr-chat" class="chat_f75fb0 chat_a7d72e chat-2ZfjoI chat_z_gUBg">
  <div class="content_f75fb0">
    <div class="chatContent_f75fb0 chatContent_a7d72e">
      <div id="stellarr-message" class="messageContent_c19a55 markup__75297 markup-eYLPri"></div>
    </div>
  </div>
  <div class="channelTextArea_f75fb0 channelTextArea__74017">
    <div id="stellarr-input" class="themedBackground__74017 scrollableContainer__74017 scrollableContainer-15eg7h"></div>
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
        for (const id of ["stellarr-frame", "stellarr-app", "app-mount", "stellarr-chat"]) {
          const element = doc.getElementById(id);
          if (element === null) continue;
          const image = view.getComputedStyle(element).backgroundImage;
          if (image !== "none" && image.includes("rgb(1, ")) {
            values.set("--stellarr-page-image", image);
            break;
          }
        }
        const containers = { root: read(doc.documentElement), body: read(doc.body) };

        const context = canvasContext();
        const colours = new Map<string, Rgba>();
        if (context !== null) {
          for (const name of ["--stellarr-frame", "--stellarr-sidebar", "--stellarr-chat", "--stellarr-input"]) {
            const element = doc.getElementById(name.slice(2));
            if (element === null) continue;
            const rgba = colourToRgba(context, view.getComputedStyle(element).backgroundColor);
            if (rgba !== null && rgba[3] > 0) colours.set(name, rgba);
          }
          const message = doc.getElementById("stellarr-message");
          if (message !== null) {
            const seen = SENTINELS.map((sentinel) => {
              parent.style.color = sentinel;
              return view.getComputedStyle(message).color;
            });
            if (seen[0] !== undefined && seen[0] === seen[1]) {
              const rgba = colourToRgba(context, seen[0]);
              if (rgba !== null) colours.set("--stellarr-message", rgba);
            }
          }
          for (const name of colourNames) {
            if (name.startsWith("--stellarr-")) continue;
            const seen = SENTINELS.map((sentinel) => {
              parent.style.color = sentinel;
              probe.style.color = `var(${name})`;
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
