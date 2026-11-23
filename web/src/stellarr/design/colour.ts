/**
 * The colour arithmetic a custom theme needs: reading `#rrggbb` and
 * `#rrggbbaa`, relative luminance and contrast as WCAG defines them, and
 * OKLCH hue and chroma. Channels are numbers from 0 to 1 in sRGB's own
 * encoding.
 *
 * Only the code that sets a custom palette's colours uses it (the board, the
 * theme file, the converter), never the first page: the values it derives are
 * stored with the palette (`customTheme.ts`).
 */

export type Rgba = readonly [number, number, number, number];

export function parseHex(hex: string): Rgba {
  const channel = (offset: number) => Number.parseInt(hex.slice(offset, offset + 2), 16) / 255;
  return [channel(1), channel(3), channel(5), hex.length === 9 ? channel(7) : 1];
}

const byte = (value: number) =>
  Math.round(Math.min(1, Math.max(0, value)) * 255)
    .toString(16)
    .padStart(2, "0");

/** `#rrggbb` when opaque, `#rrggbbaa` when not. */
export function toHex([red, green, blue, alpha = 1]: readonly number[]): string {
  const opaque = `#${byte(red ?? 0)}${byte(green ?? 0)}${byte(blue ?? 0)}`;
  return alpha >= 1 ? opaque : `${opaque}${byte(alpha)}`;
}

/** `top` painted over an opaque `bottom`, as the browser composites it. */
export function flatten(top: Rgba, bottom: Rgba): Rgba {
  const alpha = top[3];
  return [
    top[0] * alpha + bottom[0] * (1 - alpha),
    top[1] * alpha + bottom[1] * (1 - alpha),
    top[2] * alpha + bottom[2] * (1 - alpha),
    1,
  ];
}

const linear = (channel: number) =>
  channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;

export function relativeLuminance([red, green, blue]: Rgba): number {
  return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue);
}

export function contrastRatio(a: Rgba, b: Rgba): number {
  const first = relativeLuminance(a);
  const second = relativeLuminance(b);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

/** Lightness, chroma and hue in degrees, of an opaque colour. */
export function toOklch([red, green, blue]: Rgba): [number, number, number] {
  const r = linear(red);
  const g = linear(green);
  const b = linear(blue);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    Math.hypot(a, bb),
    ((Math.atan2(bb, a) * 180) / Math.PI + 360) % 360,
  ];
}

/** The contrast small interface text has to keep, WCAG's AA for normal text. */
export const SMALL_TEXT_CONTRAST = 4.5;

/**
 * The faint text a palette's small copy is set in: its own faint colour,
 * moved toward its normal text in steps of 1/255 until it reaches AA on the
 * background closest to it, the lightest for light text and the darkest for
 * dark text, so it reads on every surface, hover and selection included. That
 * is the rule the presets' hand-tuned values are held to. A faint colour that
 * already passes stays as it is; a palette whose normal text cannot pass gets
 * its normal text. The first background is the canvas the others are
 * flattened onto.
 */
export function deriveFaint(faint: string, normal: string, backgrounds: readonly string[]): string {
  const canvas = flatten(parseHex(backgrounds[0] ?? "#000000"), [0, 0, 0, 1]);
  const surfaces = backgrounds.map((background) => flatten(parseHex(background), canvas));
  const rising = relativeLuminance(flatten(parseHex(normal), canvas)) > relativeLuminance(canvas);
  const nearest = surfaces.reduce((a, b) => {
    const [first, second] = [relativeLuminance(a), relativeLuminance(b)];
    return (rising ? second > first : second < first) ? b : a;
  });
  const start = flatten(parseHex(faint), nearest);
  const end = flatten(parseHex(normal), nearest);
  for (let step = 0; step <= 255; step += 1) {
    const mix = step / 255;
    const candidate = toHex(start.map((channel, index) => channel + ((end[index] ?? 0) - channel) * mix));
    if (contrastRatio(parseHex(candidate), nearest) >= SMALL_TEXT_CONTRAST) return candidate;
  }
  return toHex(end);
}

/** How much chroma one step of `--c-neutral` stands for in the light ramp. */
export const NEUTRAL_CHROMA_STEP = 0.008;
/** Below this the primary background reads as grey and the light ramp is too. */
export const NEUTRAL_CHROMA_FLOOR = 0.002;

/**
 * The tint of the light counterpart: `--h-neutral` and `--c-neutral` from the
 * hue and chroma of the dark palette's primary background, which is what
 * `palettes.css` tunes by hand for each preset.
 */
export function deriveNeutral(primary: string, darkest: string): { hue: number; chroma: number } {
  const [, chroma, hue] = toOklch(flatten(parseHex(primary), flatten(parseHex(darkest), [0, 0, 0, 1])));
  if (chroma < NEUTRAL_CHROMA_FLOOR) return { hue: 0, chroma: 0 };
  return { hue: Math.round(hue) % 360, chroma: Math.round((chroma / NEUTRAL_CHROMA_STEP) * 10) / 10 };
}
