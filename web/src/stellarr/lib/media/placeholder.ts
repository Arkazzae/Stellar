/**
 * A picture described in at most 32 bytes, enough for a reader's browser to
 * paint a blurred stand-in while the real image is still on its way.
 *
 * The description is a handful of low-frequency coefficients of a discrete
 * cosine transform (DCT-II) of the picture, taken per channel: luma (Y), two
 * chroma channels (Cb, Cr) and, only when some pixel is not opaque, alpha.
 * The decoder sums those cosines back up on a grid whose longest side is 32
 * pixels. Nothing here touches the DOM or a canvas, so it runs in Node too.
 *
 * Layout. The bytes are one bit stream, most significant bit first, and the
 * last byte is padded with zero bits. The header says what follows, so the
 * only valid length is the one it implies.
 *
 *   header, 12 bits
 *     3 bits  version, 1 (0 and 2..7 are unknown)
 *     1 bit   alpha channel present
 *     1 bit   orientation, 1 = width >= height (landscape or square)
 *     7 bits  ratio code r, longer side / shorter side = 2^(r * 4 / 127),
 *             so 1:1 is 0 and 16:1 is 127 (steps of about 2 percent)
 *   then one block per channel, in the order Y, Cb, Cr, alpha (if present):
 *     DC     the channel's average, unsigned, the first field of the block:
 *              Y      7 bits, 0..1 mapped to 0..127
 *              Cb, Cr 6 bits, -0.5..0.5 mapped to 0..63
 *              alpha  6 bits, 0..1 mapped to 0..63
 *     scale  4 bits s, the largest AC magnitude of the channel is at most
 *            2^((s - 15) / 2), so 15 means 1 and each step is about 1.41x
 *     AC     one field per coefficient, with b bits (Y, Cb, Cr: 4, alpha: 3)
 *            holding q = round(v / scale * h) + h, where h = 2^(b-1) - 1;
 *            the value 2^b - 1 is never written and makes the input invalid
 *
 * Coefficients. Coefficient (i, j) is the cosine with i half-waves across the
 * width and j down the height. A block has i < nx and j < ny, without (0, 0)
 * (the DC), and keeps those with i * ny + j * nx < nx * ny, a triangle that
 * drops the diagonal corner where the least energy lives. They are stored row
 * by row (j outer, i inner). The grid is larger along the longer side:
 *
 *   channel   long side   short side   AC count   bits in block
 *   Y         8           5            25         7 + 4 + 25 * 4 = 111
 *   Cb, Cr    4           3            8          6 + 4 +  8 * 4 =  42 each
 *   alpha     4           3            8          6 + 4 +  8 * 3 =  34
 *
 * "Long" is the width for a landscape header and the height for a portrait one.
 * Without alpha that is 12 + 111 + 84 = 207 bits, 26 bytes; with alpha 241
 * bits, 31 bytes. Both fit in 32 with room that the fixed layout does not use,
 * on purpose: a layout that depends on the picture would need more header.
 *
 * Why these numbers. The eye resolves brightness far finer than colour, so
 * luma gets about three times the coefficients and one more bit for its
 * average; colour needs a good average (a wrong tint is the most visible
 * error in a blurred picture) and only its broad gradients. The average gets
 * more bits than the rest because every other coefficient is relative to it
 * and an error in it shifts the whole picture, while AC values only need to
 * be about right once the per-channel scale spends all 4 bits on the range
 * that channel actually uses: a smooth picture and a busy one both get 15
 * levels across their own range. Alpha is usually flat or an edge, so 3 bits
 * will do. The proportions take 8 bits because the box the placeholder sits
 * in must not jump when the picture arrives; the reader normally also knows
 * the exact size, so a 2 percent error here only changes the blur's shape.
 *
 * Decoding by hand. For a channel with average d, scale S and coefficient
 * fields q, the value at (x, y) of a W by H output is
 *   d + sum over coefficients of ((q - h) / h * S) * cos(pi * i * (x + 0.5) / W)
 *                                                  * cos(pi * j * (y + 0.5) / H)
 * with Y in 0..1 and Cb, Cr in -0.5..0.5, and then
 *   R = Y + 1.402 Cr,  G = Y - 0.344136 Cb - 0.714136 Cr,  B = Y + 1.772 Cb
 * scaled to 0..255. The output has the longer side 32 and the shorter side
 * round(32 / ratio), at least 1.
 */

/** RGBA pixels, row by row, alpha not premultiplied. */
export interface Pixels {
  width: number;
  height: number;
  data: Uint8ClampedArray;
}

export const PLACEHOLDER_MAX_BYTES = 32;

const VERSION = 1;
const VERSION_BITS = 3;
const RATIO_BITS = 7;
const RATIO_MAX_CODE = (1 << RATIO_BITS) - 1;
/** log2 of the largest proportion that can be stored (16:1). */
const RATIO_LOG2_SPAN = 4;
const SCALE_BITS = 4;
const SCALE_MAX_CODE = (1 << SCALE_BITS) - 1;
const HEADER_BITS = VERSION_BITS + 1 + 1 + RATIO_BITS;

const MAX_INPUT_SIDE = 64;
const OUTPUT_LONG_SIDE = 32;

interface ChannelSpec {
  /** Coefficients along the longer side of the picture, DC included. */
  long: number;
  /** Coefficients along the shorter side of the picture, DC included. */
  short: number;
  dcBits: number;
  acBits: number;
  /** Chroma averages lie in -0.5..0.5, the others in 0..1. */
  signed: boolean;
}

const LUMA: ChannelSpec = { long: 8, short: 5, dcBits: 7, acBits: 4, signed: false };
const CHROMA: ChannelSpec = { long: 4, short: 3, dcBits: 6, acBits: 4, signed: true };
const ALPHA: ChannelSpec = { long: 4, short: 3, dcBits: 6, acBits: 3, signed: false };

const COLOR_CHANNELS = [LUMA, CHROMA, CHROMA];

/** A coefficient position: `i` half-waves across the width, `j` down the height. */
interface Frequency {
  i: number;
  j: number;
}

function channelsFor(hasAlpha: boolean): ChannelSpec[] {
  return hasAlpha ? [...COLOR_CHANNELS, ALPHA] : COLOR_CHANNELS;
}

/** The coefficients of one channel, in storage order, for a given orientation. */
function frequencies(spec: ChannelSpec, landscape: boolean): Frequency[] {
  const nx = landscape ? spec.long : spec.short;
  const ny = landscape ? spec.short : spec.long;
  const list: Frequency[] = [];
  for (let j = 0; j < ny; j++) {
    for (let i = 0; i < nx; i++) {
      if (i === 0 && j === 0) continue;
      if (i * ny + j * nx < nx * ny) list.push({ i, j });
    }
  }
  return list;
}

function blockBits(spec: ChannelSpec, landscape: boolean): number {
  return spec.dcBits + SCALE_BITS + frequencies(spec, landscape).length * spec.acBits;
}

function totalBits(hasAlpha: boolean, landscape: boolean): number {
  let bits = HEADER_BITS;
  for (const spec of channelsFor(hasAlpha)) bits += blockBits(spec, landscape);
  return bits;
}

class BitWriter {
  readonly bytes: Uint8Array;
  private position = 0;

  constructor(bits: number) {
    this.bytes = new Uint8Array(Math.ceil(bits / 8));
  }

  write(value: number, bits: number): void {
    for (let b = bits - 1; b >= 0; b--) {
      if (((value >> b) & 1) === 1) {
        this.bytes[this.position >> 3]! |= 0x80 >> (this.position & 7);
      }
      this.position++;
    }
  }
}

class BitReader {
  private position = 0;
  private readonly bytes: Uint8Array;

  constructor(bytes: Uint8Array) {
    this.bytes = bytes;
  }

  read(bits: number): number {
    let value = 0;
    for (let b = 0; b < bits; b++) {
      const bit = (this.bytes[this.position >> 3]! >> (7 - (this.position & 7))) & 1;
      value = (value << 1) | bit;
      this.position++;
    }
    return value;
  }

  /** True when every bit from here to the end of the data is zero. */
  restIsZero(): boolean {
    for (let p = this.position; p < this.bytes.length * 8; p++) {
      if (((this.bytes[p >> 3]! >> (7 - (p & 7))) & 1) !== 0) return false;
    }
    return true;
  }
}

function scaleOf(code: number): number {
  return 2 ** ((code - SCALE_MAX_CODE) / 2);
}

function scaleCodeFor(largest: number): number {
  if (!(largest > 0)) return 0;
  const code = Math.ceil(SCALE_MAX_CODE + 2 * Math.log2(largest) - 1e-9);
  return Math.min(SCALE_MAX_CODE, Math.max(0, code));
}

/** cosines[k][n] = cos(pi * k * (n + 0.5) / size) for k below `count`. */
function cosineTable(count: number, size: number): Float64Array[] {
  const table: Float64Array[] = [];
  for (let k = 0; k < count; k++) {
    const row = new Float64Array(size);
    for (let n = 0; n < size; n++) row[n] = Math.cos((Math.PI * k * (n + 0.5)) / size);
    table.push(row);
  }
  return table;
}

/**
 * At most PLACEHOLDER_MAX_BYTES bytes. Accepts images up to 64 by 64; the
 * caller scales down first.
 */
export function encodePlaceholder(pixels: Pixels): Uint8Array {
  const { width, height, data } = pixels;
  if (
    !Number.isInteger(width) ||
    !Number.isInteger(height) ||
    width < 1 ||
    height < 1 ||
    width > MAX_INPUT_SIDE ||
    height > MAX_INPUT_SIDE ||
    data.length !== width * height * 4
  ) {
    throw new RangeError("placeholder input must be 1 to 64 pixels per side, RGBA");
  }

  const count = width * height;
  let hasAlpha = false;
  let alphaSum = 0;
  for (let p = 0; p < count; p++) {
    const a = data[p * 4 + 3]!;
    if (a < 255) hasAlpha = true;
    alphaSum += a;
  }

  // Colour under fully transparent pixels is arbitrary (often black); letting
  // it into the averages would tint the visible parts. Such pixels borrow the
  // average colour of the rest, weighted by opacity.
  let fallback = [0, 0, 0];
  if (alphaSum > 0) {
    const sums = [0, 0, 0];
    for (let p = 0; p < count; p++) {
      const a = data[p * 4 + 3]!;
      for (let c = 0; c < 3; c++) sums[c]! += data[p * 4 + c]! * a;
    }
    fallback = sums.map((s) => s / alphaSum);
  }

  const planes = [0, 1, 2, 3].map(() => new Float64Array(count));
  for (let p = 0; p < count; p++) {
    const a = data[p * 4 + 3]!;
    const [r, g, b] =
      a === 0
        ? (fallback as [number, number, number])
        : [data[p * 4]!, data[p * 4 + 1]!, data[p * 4 + 2]!];
    const y = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
    planes[0]![p] = y;
    planes[1]![p] = (b / 255 - y) / 1.772;
    planes[2]![p] = (r / 255 - y) / 1.402;
    planes[3]![p] = a / 255;
  }

  const landscape = width >= height;
  const long = Math.max(width, height);
  const short = Math.min(width, height);
  const ratioCode = Math.min(
    RATIO_MAX_CODE,
    Math.round((Math.log2(long / short) / RATIO_LOG2_SPAN) * RATIO_MAX_CODE),
  );

  const writer = new BitWriter(totalBits(hasAlpha, landscape));
  writer.write(VERSION, VERSION_BITS);
  writer.write(hasAlpha ? 1 : 0, 1);
  writer.write(landscape ? 1 : 0, 1);
  writer.write(ratioCode, RATIO_BITS);

  channelsFor(hasAlpha).forEach((spec, index) => {
    const plane = planes[index]!;
    const nx = landscape ? spec.long : spec.short;
    const ny = landscape ? spec.short : spec.long;
    const cosX = cosineTable(nx, width);
    const cosY = cosineTable(ny, height);

    const coefficient = (i: number, j: number): number => {
      let sum = 0;
      for (let y = 0; y < height; y++) {
        const cy = cosY[j]![y]!;
        let row = 0;
        for (let x = 0; x < width; x++) row += plane[y * width + x]! * cosX[i]![x]!;
        sum += row * cy;
      }
      const norm = (i === 0 ? 1 : 2) * (j === 0 ? 1 : 2);
      return (sum * norm) / count;
    };

    const dcMax = (1 << spec.dcBits) - 1;
    const dc = coefficient(0, 0);
    const dcCode = clampInt(Math.round((dc - (spec.signed ? -0.5 : 0)) * dcMax), 0, dcMax);
    writer.write(dcCode, spec.dcBits);

    const list = frequencies(spec, landscape);
    const values = list.map(({ i, j }) => coefficient(i, j));
    let largest = 0;
    for (const v of values) largest = Math.max(largest, Math.abs(v));
    const scaleCode = scaleCodeFor(largest);
    writer.write(scaleCode, SCALE_BITS);

    const h = (1 << (spec.acBits - 1)) - 1;
    const scale = scaleOf(scaleCode);
    for (const v of values) {
      writer.write(clampInt(Math.round((v / scale) * h), -h, h) + h, spec.acBits);
    }
  });

  return writer.bytes;
}

function clampInt(value: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, value));
}

/**
 * A small picture (longest side about 32 px) in the proportions of the
 * original, or null for bytes it does not understand. Never throws.
 */
export function decodePlaceholder(bytes: Uint8Array): Pixels | null {
  try {
    return decode(bytes);
  } catch {
    return null;
  }
}

function decode(bytes: Uint8Array): Pixels | null {
  if (!(bytes instanceof Uint8Array) || bytes.length * 8 < HEADER_BITS) return null;

  const reader = new BitReader(bytes);
  if (reader.read(VERSION_BITS) !== VERSION) return null;
  const hasAlpha = reader.read(1) === 1;
  const landscape = reader.read(1) === 1;
  const ratioCode = reader.read(RATIO_BITS);

  if (bytes.length !== Math.ceil(totalBits(hasAlpha, landscape) / 8)) return null;

  const ratio = 2 ** ((ratioCode / RATIO_MAX_CODE) * RATIO_LOG2_SPAN);
  const shortSide = Math.max(1, Math.round(OUTPUT_LONG_SIDE / ratio));
  const width = landscape ? OUTPUT_LONG_SIDE : shortSide;
  const height = landscape ? shortSide : OUTPUT_LONG_SIDE;
  const count = width * height;

  const channels = channelsFor(hasAlpha);
  const planes: Float64Array[] = [];
  for (const spec of channels) {
    const nx = landscape ? spec.long : spec.short;
    const ny = landscape ? spec.short : spec.long;
    const cosX = cosineTable(nx, width);
    const cosY = cosineTable(ny, height);

    const dcMax = (1 << spec.dcBits) - 1;
    const dc = reader.read(spec.dcBits) / dcMax + (spec.signed ? -0.5 : 0);
    const scale = scaleOf(reader.read(SCALE_BITS));
    const h = (1 << (spec.acBits - 1)) - 1;
    const invalid = (1 << spec.acBits) - 1;

    const plane = new Float64Array(count).fill(dc);
    for (const { i, j } of frequencies(spec, landscape)) {
      const q = reader.read(spec.acBits);
      if (q === invalid) return null;
      const value = ((q - h) / h) * scale;
      if (value === 0) continue;
      for (let y = 0; y < height; y++) {
        const weight = value * cosY[j]![y]!;
        for (let x = 0; x < width; x++) plane[y * width + x]! += weight * cosX[i]![x]!;
      }
    }
    planes.push(plane);
  }
  if (!reader.restIsZero()) return null;

  const data = new Uint8ClampedArray(count * 4);
  for (let p = 0; p < count; p++) {
    const y = planes[0]![p]!;
    const cb = planes[1]![p]!;
    const cr = planes[2]![p]!;
    data[p * 4] = (y + 1.402 * cr) * 255;
    data[p * 4 + 1] = (y - 0.344136 * cb - 0.714136 * cr) * 255;
    data[p * 4 + 2] = (y + 1.772 * cb) * 255;
    data[p * 4 + 3] = hasAlpha ? planes[3]![p]! * 255 : 255;
  }
  return { width, height, data };
}

const BASE64_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** The text form the bytes travel in: standard padded base64. */
export function placeholderToBase64(bytes: Uint8Array): string {
  let text = "";
  for (let p = 0; p < bytes.length; p += 3) {
    const a = bytes[p]!;
    const b = bytes[p + 1];
    const c = bytes[p + 2];
    const word = (a << 16) | ((b ?? 0) << 8) | (c ?? 0);
    text += BASE64_ALPHABET[(word >> 18) & 63]!;
    text += BASE64_ALPHABET[(word >> 12) & 63]!;
    text += b === undefined ? "=" : BASE64_ALPHABET[(word >> 6) & 63]!;
    text += c === undefined ? "=" : BASE64_ALPHABET[word & 63]!;
  }
  return text;
}

/**
 * Null for anything that is not canonical padded base64 (wrong length, a
 * character outside the alphabet, padding anywhere but the end, nonzero
 * leftover bits) or that decodes to more than PLACEHOLDER_MAX_BYTES bytes.
 */
export function placeholderFromBase64(text: string): Uint8Array | null {
  if (typeof text !== "string" || text.length % 4 !== 0) return null;
  const padding = text.endsWith("==") ? 2 : text.endsWith("=") ? 1 : 0;
  const size = (text.length / 4) * 3 - padding;
  if (size > PLACEHOLDER_MAX_BYTES) return null;

  const out = new Uint8Array(size);
  let produced = 0;
  for (let p = 0; p < text.length; p += 4) {
    let word = 0;
    for (let k = 0; k < 4; k++) {
      const isPad = p + k >= text.length - padding;
      const sextet = isPad ? 0 : BASE64_ALPHABET.indexOf(text[p + k]!);
      if (sextet < 0) return null;
      word = (word << 6) | sextet;
    }
    const group = [(word >> 16) & 255, (word >> 8) & 255, word & 255];
    for (const byte of group) {
      if (produced < size) out[produced++] = byte;
      else if (byte !== 0) return null;
    }
  }
  return out;
}

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const STORED_BLOCK_MAX = 0xffff;

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) crc = CRC_TABLE[(crc ^ byte) & 0xff]! ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function adler32(bytes: Uint8Array): number {
  let a = 1;
  let b = 0;
  for (const byte of bytes) {
    a = (a + byte) % 65521;
    b = (b + a) % 65521;
  }
  return ((b << 16) | a) >>> 0;
}

function uint32(value: number): number[] {
  return [(value >>> 24) & 255, (value >>> 16) & 255, (value >>> 8) & 255, value & 255];
}

function chunk(type: string, body: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + body.length);
  out.set(uint32(body.length), 0);
  for (let k = 0; k < 4; k++) out[4 + k] = type.charCodeAt(k);
  out.set(body, 8);
  out.set(uint32(crc32(out.subarray(4, 8 + body.length))), 8 + body.length);
  return out;
}

/** zlib stream made only of stored (uncompressed) deflate blocks. */
function zlibStored(raw: Uint8Array): Uint8Array {
  const blocks = Math.max(1, Math.ceil(raw.length / STORED_BLOCK_MAX));
  const out = new Uint8Array(2 + raw.length + blocks * 5 + 4);
  out[0] = 0x78;
  out[1] = 0x01;
  let at = 2;
  for (let b = 0; b < blocks; b++) {
    const part = raw.subarray(b * STORED_BLOCK_MAX, (b + 1) * STORED_BLOCK_MAX);
    const last = b === blocks - 1;
    out[at++] = last ? 1 : 0;
    out[at++] = part.length & 255;
    out[at++] = part.length >> 8;
    out[at++] = ~part.length & 255;
    out[at++] = (~part.length >> 8) & 255;
    out.set(part, at);
    at += part.length;
  }
  out.set(uint32(adler32(raw)), at);
  return out;
}

function encodePng(pixels: Pixels): Uint8Array {
  const { width, height, data } = pixels;
  const stride = width * 4;
  const scanlines = new Uint8Array(height * (stride + 1));
  for (let y = 0; y < height; y++) {
    // Filter type 0 (none) in front of every row.
    scanlines.set(data.subarray(y * stride, (y + 1) * stride), y * (stride + 1) + 1);
  }

  const header = new Uint8Array([...uint32(width), ...uint32(height), 8, 6, 0, 0, 0]);
  const parts = [
    new Uint8Array(PNG_SIGNATURE),
    chunk("IHDR", header),
    chunk("IDAT", zlibStored(scanlines)),
    chunk("IEND", new Uint8Array(0)),
  ];
  const png = new Uint8Array(parts.reduce((n, part) => n + part.length, 0));
  let at = 0;
  for (const part of parts) {
    png.set(part, at);
    at += part.length;
  }
  return png;
}

/**
 * A `data:image/png;base64,…` URL of the decoded picture, or null for bytes it
 * does not understand.
 */
export function placeholderDataUrl(bytes: Uint8Array): string | null {
  const pixels = decodePlaceholder(bytes);
  if (pixels === null) return null;
  const png = encodePng(pixels);
  return `data:image/png;base64,${placeholderToBase64(png)}`;
}
