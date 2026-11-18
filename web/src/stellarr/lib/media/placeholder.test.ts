import { inflateSync } from "node:zlib";
import { describe, expect, it } from "vitest";
import {
  PLACEHOLDER_MAX_BYTES,
  decodePlaceholder,
  encodePlaceholder,
  placeholderDataUrl,
  placeholderFromBase64,
  placeholderToBase64,
  type Pixels,
} from "./placeholder";

type Rgba = [number, number, number, number];

function picture(
  width: number,
  height: number,
  colorAt: (x: number, y: number) => Rgba,
): Pixels {
  const data = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) data.set(colorAt(x, y), (y * width + x) * 4);
  }
  return { width, height, data };
}

/** Deterministic noise, so a failure can be reproduced. */
function random(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 2 ** 32;
  };
}

/** Mean absolute error over the colour channels of two same-sized pictures. */
function meanError(a: Pixels, b: Pixels): number {
  expect([b.width, b.height]).toEqual([a.width, a.height]);
  let sum = 0;
  let n = 0;
  for (let p = 0; p < a.width * a.height; p++) {
    for (let c = 0; c < 3; c++) {
      sum += Math.abs(a.data[p * 4 + c]! - b.data[p * 4 + c]!);
      n++;
    }
  }
  return sum / n;
}

function decoded(pixels: Pixels): Pixels {
  const result = decodePlaceholder(encodePlaceholder(pixels));
  if (result === null) throw new Error("a fresh placeholder must decode");
  return result;
}

function averageOf(pixels: Pixels, x0: number, x1: number): number[] {
  const sum = [0, 0, 0];
  let n = 0;
  for (let y = 0; y < pixels.height; y++) {
    for (let x = x0; x < x1; x++) {
      for (let c = 0; c < 3; c++) sum[c]! += pixels.data[(y * pixels.width + x) * 4 + c]!;
      n++;
    }
  }
  return sum.map((s) => s / n);
}

describe("a placeholder of a picture", () => {
  it("brings a solid colour back within 6 levels per channel", () => {
    for (const color of [
      [200, 30, 40],
      [0, 0, 0],
      [255, 255, 255],
      [10, 220, 120],
      [30, 60, 250],
    ] as const) {
      const source = picture(32, 32, () => [...color, 255]);
      const result = decoded(source);
      for (let p = 0; p < result.width * result.height; p++) {
        for (let c = 0; c < 3; c++) {
          expect(Math.abs(result.data[p * 4 + c]! - color[c]!)).toBeLessThanOrEqual(6);
        }
        expect(result.data[p * 4 + 3]).toBe(255);
      }
    }
  });

  it("brings gradients back with a mean absolute error under 10 levels", () => {
    const horizontal = picture(32, 32, (x) => [x * 8, 255 - x * 8, 90, 255]);
    const vertical = picture(32, 32, (_x, y) => [40, y * 8, 255 - y * 8, 255]);

    expect(meanError(horizontal, decoded(horizontal))).toBeLessThan(10);
    expect(meanError(vertical, decoded(vertical))).toBeLessThan(10);
  });

  it("keeps the average colour of each of two flat halves within 14 levels", () => {
    const left: Rgba = [220, 40, 40, 255];
    const right: Rgba = [30, 70, 200, 255];
    const source = picture(32, 32, (x) => (x < 16 ? left : right));
    const result = decoded(source);

    // The edge blurs, so the halves are judged away from the seam.
    const leftAverage = averageOf(result, 0, 8);
    const rightAverage = averageOf(result, 24, 32);
    for (let c = 0; c < 3; c++) {
      expect(Math.abs(leftAverage[c]! - left[c]!)).toBeLessThan(14);
      expect(Math.abs(rightAverage[c]! - right[c]!)).toBeLessThan(14);
    }
  });

  it.each([
    ["1:1", 40, 40, 32, 32],
    ["16:9", 64, 36, 32, 18],
    ["9:16", 36, 64, 18, 32],
    ["4:1", 64, 16, 32, 8],
  ])("keeps the proportions of %s", (_name, width, height, outWidth, outHeight) => {
    const result = decoded(picture(width, height, (x, y) => [x * 4, y * 4, 128, 255]));

    expect([result.width, result.height]).toEqual([outWidth, outHeight]);
    expect(result.data.length).toBe(outWidth * outHeight * 4);
  });

  it("keeps half transparency, and spends no bytes on alpha when opaque", () => {
    const translucent = picture(32, 32, () => [90, 160, 40, 128]);
    const opaque = picture(32, 32, () => [90, 160, 40, 255]);

    const result = decoded(translucent);
    for (let p = 0; p < result.width * result.height; p++) {
      expect(Math.abs(result.data[p * 4 + 3]! - 128)).toBeLessThanOrEqual(3);
    }
    expect(encodePlaceholder(opaque).length).toBeLessThan(encodePlaceholder(translucent).length);
    expect(decoded(opaque).data.every((v, i) => i % 4 !== 3 || v === 255)).toBe(true);
  });

  it("does not let the colour under transparent pixels tint the visible ones", () => {
    const source = picture(32, 32, (x) => (x < 16 ? [200, 50, 50, 255] : [0, 0, 0, 0]));
    const result = decoded(source);

    expect(result.data[(16 * 32 + 2) * 4]).toBeGreaterThan(185);
    expect(result.data[(16 * 32 + 2) * 4 + 3]).toBeGreaterThan(230);
    expect(result.data[(16 * 32 + 29) * 4 + 3]).toBeLessThan(25);
  });

  it("never exceeds 32 bytes, for noise, extremes and transparency alike", () => {
    const noise = random(7);
    const sizes: [number, number][] = [[1, 1], [64, 64], [64, 1], [1, 64], [64, 16], [17, 63], [32, 32]];
    for (const [width, height] of sizes) {
      const inputs = [
        picture(width, height, () => [noise() * 256, noise() * 256, noise() * 256, 255]),
        picture(width, height, () => [noise() * 256, noise() * 256, noise() * 256, noise() * 256]),
        picture(width, height, (x, y) => ((x + y) % 2 === 0 ? [255, 255, 255, 255] : [0, 0, 0, 255])),
        picture(width, height, () => [255, 0, 255, 0]),
      ];
      for (const input of inputs) {
        const bytes = encodePlaceholder(input);
        expect(bytes.length).toBeLessThanOrEqual(PLACEHOLDER_MAX_BYTES);
        expect(decodePlaceholder(bytes)).not.toBeNull();
      }
    }
  });
});

describe("decoding bytes that are not a placeholder", () => {
  const valid = encodePlaceholder(picture(32, 32, (x, y) => [x * 8, y * 8, 100, 255]));
  const validWithAlpha = encodePlaceholder(picture(32, 20, (x, y) => [x * 8, y * 8, 100, 200]));

  it("gives null for every truncation and every extension of a valid one", () => {
    for (const bytes of [valid, validWithAlpha]) {
      for (let length = 0; length < bytes.length; length++) {
        expect(decodePlaceholder(bytes.slice(0, length))).toBeNull();
      }
      expect(decodePlaceholder(new Uint8Array([...bytes, 0]))).toBeNull();
    }
  });

  it("gives null for an unknown version", () => {
    for (const version of [0, 2, 3, 7]) {
      const changed = valid.slice();
      changed[0] = (changed[0]! & 0x1f) | (version << 5);
      expect(decodePlaceholder(changed)).toBeNull();
    }
  });

  it("gives null, without throwing, for an empty array and for random bytes", () => {
    expect(decodePlaceholder(new Uint8Array(0))).toBeNull();

    const next = random(11);
    for (let n = 0; n < 500; n++) {
      const bytes = Uint8Array.from({ length: Math.floor(next() * 40) }, () => next() * 256);
      expect(() => decodePlaceholder(bytes)).not.toThrow();
    }
    // Random bytes that happen to carry a plausible header must still be refused
    // or decode to a well-formed picture, never anything in between.
    for (let n = 0; n < 500; n++) {
      const bytes = Uint8Array.from({ length: 26 }, () => next() * 256);
      bytes[0] = (bytes[0]! & 0x1f) | 0x20;
      const result = decodePlaceholder(bytes);
      if (result !== null) {
        expect(result.data.length).toBe(result.width * result.height * 4);
      }
    }
  });
});

describe("the text form", () => {
  it("round-trips through base64, as standard padded base64", () => {
    const bytes = encodePlaceholder(picture(32, 18, (x, y) => [x * 8, y * 14, 60, 255]));
    const text = placeholderToBase64(bytes);

    expect(text).toBe(Buffer.from(bytes).toString("base64"));
    expect(placeholderFromBase64(text)).toEqual(bytes);
    for (let length = 0; length <= 6; length++) {
      const part = bytes.slice(0, length);
      expect(placeholderFromBase64(placeholderToBase64(part))).toEqual(part);
    }
  });

  it("gives null for malformed base64", () => {
    for (const text of [
      "AAA",
      "AAAAA",
      "AA=A",
      "A===",
      "AA==AAAA",
      "AAA*",
      "AAB=",
      "AAAA AAAA",
      "-_-_",
      "A".repeat(48),
    ]) {
      expect(placeholderFromBase64(text)).toBeNull();
    }
  });
});

describe("the data URL", () => {
  const source = picture(32, 18, (x, y) => [x * 8, y * 14, 200 - x * 4, y < 9 ? 255 : 160]);
  const bytes = encodePlaceholder(source);
  const url = placeholderDataUrl(bytes);

  function pngOf(dataUrl: string | null): Buffer {
    expect(dataUrl).toMatch(/^data:image\/png;base64,/);
    return Buffer.from(dataUrl!.slice("data:image/png;base64,".length), "base64");
  }

  /** Bitwise CRC-32, written apart from the module's table-driven one. */
  function referenceCrc(bytes: Uint8Array): number {
    let crc = 0xffffffff;
    for (const byte of bytes) {
      crc ^= byte;
      for (let k = 0; k < 8; k++) crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
    return (crc ^ 0xffffffff) >>> 0;
  }

  it("is null for bytes that are not a placeholder", () => {
    expect(placeholderDataUrl(new Uint8Array(0))).toBeNull();
  });

  it("is a PNG with the right signature, dimensions and chunk checksums", () => {
    const png = pngOf(url);
    const decodedPixels = decodePlaceholder(bytes)!;

    expect([...png.subarray(0, 8)]).toEqual([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    expect(png.readUInt32BE(16)).toBe(decodedPixels.width);
    expect(png.readUInt32BE(20)).toBe(decodedPixels.height);
    expect([png[24], png[25]]).toEqual([8, 6]);

    const types: string[] = [];
    let at = 8;
    while (at < png.length) {
      const length = png.readUInt32BE(at);
      const type = png.toString("latin1", at + 4, at + 8);
      types.push(type);
      const covered = png.subarray(at + 4, at + 8 + length);
      expect(png.readUInt32BE(at + 8 + length)).toBe(referenceCrc(covered));
      at += 12 + length;
    }
    expect(at).toBe(png.length);
    expect(types).toEqual(["IHDR", "IDAT", "IEND"]);
  });

  it("holds exactly the scanlines of the decoded picture in its IDAT", () => {
    const png = pngOf(url);
    const { width, height, data } = decodePlaceholder(bytes)!;
    const idatLength = png.readUInt32BE(33);
    const idat = png.subarray(41, 41 + idatLength);

    const expected = Buffer.alloc(height * (width * 4 + 1));
    for (let y = 0; y < height; y++) {
      Buffer.from(data.subarray(y * width * 4, (y + 1) * width * 4)).copy(
        expected,
        y * (width * 4 + 1) + 1,
      );
    }
    expect(inflateSync(idat).equals(expected)).toBe(true);
  });
});
