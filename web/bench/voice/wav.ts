/**
 * RIFF WAVE in and out, without a package.
 *
 * Reads integer PCM (8, 16, 24 and 32 bit) and IEEE float (32 and 64 bit),
 * plain or WAVE_FORMAT_EXTENSIBLE. Several channels are averaged into one,
 * since the detector only ever sees one channel. Writes 16-bit PCM or 32-bit
 * float, for listening to the fixtures.
 */

export interface DecodedWav {
  sampleRate: number;
  channels: number;
  bitsPerSample: number;
  format: "pcm" | "float";
  samples: Float32Array;
}

const PCM = 1;
const FLOAT = 3;
const EXTENSIBLE = 0xfffe;

function text(view: DataView, offset: number): string {
  return String.fromCharCode(
    view.getUint8(offset),
    view.getUint8(offset + 1),
    view.getUint8(offset + 2),
    view.getUint8(offset + 3),
  );
}

export function decodeWav(bytes: Uint8Array): DecodedWav {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes.byteLength < 12 || text(view, 0) !== "RIFF" || text(view, 8) !== "WAVE") {
    throw new Error("not a RIFF WAVE file");
  }
  let offset = 12;
  let formatTag = 0;
  let channels = 0;
  let sampleRate = 0;
  let bitsPerSample = 0;
  let data: { start: number; length: number } | null = null;
  while (offset + 8 <= bytes.byteLength) {
    const id = text(view, offset);
    const size = view.getUint32(offset + 4, true);
    const body = offset + 8;
    if (id === "fmt ") {
      formatTag = view.getUint16(body, true);
      channels = view.getUint16(body + 2, true);
      sampleRate = view.getUint32(body + 4, true);
      bitsPerSample = view.getUint16(body + 14, true);
      if (formatTag === EXTENSIBLE && size >= 40) {
        // The first two bytes of the sub-format GUID carry the real tag.
        formatTag = view.getUint16(body + 24, true);
      }
    } else if (id === "data") {
      // A streaming writer may leave the size at its maximum; trust the file.
      data = { start: body, length: Math.min(size, bytes.byteLength - body) };
    }
    offset = body + size + (size % 2);
  }
  if (data === null) throw new Error("no data chunk");
  if (channels < 1 || sampleRate < 1) throw new Error("no usable fmt chunk");

  const bytesPerSample = bitsPerSample / 8;
  const frames = Math.floor(data.length / (bytesPerSample * channels));
  const samples = new Float32Array(frames);
  const read = sampleReader(view, formatTag, bitsPerSample);
  for (let frame = 0; frame < frames; frame += 1) {
    let sum = 0;
    for (let channel = 0; channel < channels; channel += 1) {
      sum += read(data.start + (frame * channels + channel) * bytesPerSample);
    }
    samples[frame] = sum / channels;
  }
  return {
    sampleRate,
    channels,
    bitsPerSample,
    format: formatTag === FLOAT ? "float" : "pcm",
    samples,
  };
}

function sampleReader(
  view: DataView,
  formatTag: number,
  bits: number,
): (offset: number) => number {
  if (formatTag === FLOAT && bits === 32) return (at) => view.getFloat32(at, true);
  if (formatTag === FLOAT && bits === 64) return (at) => view.getFloat64(at, true);
  if (formatTag !== PCM) throw new Error(`unsupported WAVE format ${formatTag}/${bits}`);
  switch (bits) {
    case 8:
      return (at) => (view.getUint8(at) - 128) / 128;
    case 16:
      return (at) => view.getInt16(at, true) / 32_768;
    case 24:
      return (at) => {
        const value =
          view.getUint8(at) | (view.getUint8(at + 1) << 8) | (view.getInt8(at + 2) << 16);
        return value / 8_388_608;
      };
    case 32:
      return (at) => view.getInt32(at, true) / 2_147_483_648;
    default:
      throw new Error(`unsupported PCM width ${bits}`);
  }
}

export function encodeWav(
  samples: Float32Array,
  sampleRate: number,
  format: "pcm16" | "float32" = "pcm16",
): Uint8Array {
  const bytesPerSample = format === "pcm16" ? 2 : 4;
  const dataLength = samples.length * bytesPerSample;
  const bytes = new Uint8Array(44 + dataLength);
  const view = new DataView(bytes.buffer);
  const write = (offset: number, value: string) => {
    for (let i = 0; i < 4; i += 1) view.setUint8(offset + i, value.charCodeAt(i));
  };
  write(0, "RIFF");
  view.setUint32(4, 36 + dataLength, true);
  write(8, "WAVE");
  write(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, format === "pcm16" ? PCM : FLOAT, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * bytesPerSample, true);
  view.setUint16(32, bytesPerSample, true);
  view.setUint16(34, bytesPerSample * 8, true);
  write(36, "data");
  view.setUint32(40, dataLength, true);
  for (let i = 0; i < samples.length; i += 1) {
    const value = samples[i] ?? 0;
    if (format === "pcm16") {
      const clamped = Math.max(-1, Math.min(1, value));
      view.setInt16(44 + i * 2, Math.round(clamped * 32_767), true);
    } else {
      view.setFloat32(44 + i * 4, value, true);
    }
  }
  return bytes;
}
