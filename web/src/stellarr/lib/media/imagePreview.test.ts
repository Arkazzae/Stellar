import { describe, expect, it } from "vitest";
import {
  PREVIEW_MAX_BYTES,
  isAnimatedImage,
  mayHavePreview,
  previewIsWorthSending,
  previewSize,
} from "./imagePreview";

const ascii = (text: string) => [...text].map((char) => char.charCodeAt(0));
const be32 = (value: number) => [0, 0, 0, value];
const PNG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const chunk = (type: string, data: number[]) => [...be32(data.length), ...ascii(type), ...data, 0, 0, 0, 0];
const header = chunk("IHDR", new Array<number>(13).fill(0));

describe("isAnimatedImage", () => {
  it("treats every GIF as animated", () => {
    expect(isAnimatedImage(new Uint8Array(ascii("GIF89a...")))).toBe(true);
  });

  it("finds an animation chunk ahead of the first image data in a PNG", () => {
    const control = chunk("acTL", [0, 0, 0, 2, 0, 0, 0, 0]);
    const animated = [...PNG, ...header, ...control, ...chunk("IDAT", [1, 2])];
    const still = [...PNG, ...header, ...chunk("IDAT", [1, 2]), ...control];
    expect(isAnimatedImage(new Uint8Array(animated))).toBe(true);
    expect(isAnimatedImage(new Uint8Array(still))).toBe(false);
  });

  it("counts a PNG whose head ends before any image data as animated", () => {
    expect(isAnimatedImage(new Uint8Array([...PNG, ...header]))).toBe(true);
  });

  it("reads the animation flag of an extended WebP", () => {
    const extended = (flags: number) =>
      new Uint8Array([
        ...ascii("RIFF"), 0, 0, 0, 0, ...ascii("WEBP"), ...ascii("VP8X"),
        10, 0, 0, 0, flags, 0, 0, 0, 0, 0, 0, 0, 0, 0,
      ]);
    const lossy = new Uint8Array([...ascii("RIFF"), 0, 0, 0, 0, ...ascii("WEBP"), ...ascii("VP8 "), 0, 0, 0, 0]);
    expect(isAnimatedImage(extended(0x02))).toBe(true);
    expect(isAnimatedImage(extended(0x10))).toBe(false);
    expect(isAnimatedImage(lossy)).toBe(false);
  });

  it("leaves a JPEG alone", () => {
    expect(isAnimatedImage(new Uint8Array([0xff, 0xd8, 0xff, 0xe0]))).toBe(false);
  });
});

describe("preview rules", () => {
  it("excludes SVG and GIF from previews and admits other images", () => {
    expect(mayHavePreview("image/svg+xml")).toBe(false);
    expect(mayHavePreview("image/gif")).toBe(false);
    expect(mayHavePreview("application/pdf")).toBe(false);
    expect(mayHavePreview("image/png")).toBe(true);
  });

  it("fits a large picture in the square and never enlarges a small one", () => {
    expect(previewSize(4000, 3000)).toEqual({ width: 1280, height: 960 });
    expect(previewSize(3000, 6000)).toEqual({ width: 640, height: 1280 });
    expect(previewSize(300, 200)).toEqual({ width: 300, height: 200 });
  });

  it("keeps a preview only within 1 MiB and 60% of the original", () => {
    const ten = 10 * PREVIEW_MAX_BYTES;
    expect(previewIsWorthSending(600, 1000)).toBe(true);
    expect(previewIsWorthSending(601, 1000)).toBe(false);
    expect(previewIsWorthSending(PREVIEW_MAX_BYTES, ten)).toBe(true);
    expect(previewIsWorthSending(PREVIEW_MAX_BYTES + 1, ten)).toBe(false);
  });
});
