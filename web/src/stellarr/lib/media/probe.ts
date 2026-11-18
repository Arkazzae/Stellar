import { encodePlaceholder, placeholderToBase64, type Pixels } from "./placeholder";
import {
  ANIMATION_HEAD_BYTES,
  PREVIEW_JPEG_QUALITY,
  PREVIEW_WEBP_QUALITY,
  isAnimatedImage,
  mayHavePreview,
  previewIsWorthSending,
  previewSize,
} from "./imagePreview";

/** Metadata measured from the exact local file before it is uploaded. */
export interface UploadMediaMetadata {
  durationSeconds?: number;
  width?: number;
  height?: number;
  /** A bounded JPEG frame generated from a local video. */
  poster?: Blob;
  /** A bounded still copy of a local image, WebP or JPEG, for small views. */
  preview?: Blob;
  /** The picture in a few bytes as base64, for a blurred stand-in while it loads. */
  placeholder?: string;
  /** Marks audio captured in the composer rather than uploaded as a file. */
  voiceMessage?: boolean;
}

/** Budget for the whole probe; a file that is not readable by then is uploaded as it is. */
const PROBE_TIMEOUT_MS = 10_000;
const FRAME_TIMEOUT_MS = 3_000;
const POSTER_MAX_WIDTH = 960;
const POSTER_MAX_HEIGHT = 540;
const POSTER_JPEG_QUALITY = 0.82;
/** The placeholder is described from a drawing no larger than this on its longer side. */
const PLACEHOLDER_MAX_EDGE = 32;

/**
 * Lets the browser's own decoders inspect a file about to be uploaded. What
 * this browser cannot decode yields no optional metadata, never an error, and
 * the file still uploads.
 */
export async function probeMedia(file: File): Promise<UploadMediaMetadata> {
  if (file.type.startsWith("image/")) return probeImage(file);
  if (file.type.startsWith("video/")) return probePlayable(file, "video");
  if (file.type.startsWith("audio/")) return probePlayable(file, "audio");
  return {};
}

async function probeImage(file: File): Promise<UploadMediaMetadata> {
  if (typeof createImageBitmap !== "function") return {};
  try {
    // The declared orientation is applied, so a phone photo's size and preview
    // are those of the picture as it is meant to be seen.
    const bitmap = await createImageBitmap(file, { imageOrientation: "from-image" });
    try {
      const { width, height } = bitmap;
      const placeholder = makePlaceholder(bitmap);
      const preview = await withinBudget(makePreview(bitmap, file));
      return {
        width,
        height,
        ...(preview === undefined ? {} : { preview }),
        ...(placeholder === undefined ? {} : { placeholder }),
      };
    } finally {
      bitmap.close();
    }
  } catch {
    return {};
  }
}

/** Resolves undefined when the work fails or outlasts the probe's budget. */
function withinBudget<T>(work: Promise<T | undefined>): Promise<T | undefined> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(undefined), PROBE_TIMEOUT_MS);
    const settle = (value: T | undefined) => {
      clearTimeout(timer);
      resolve(value);
    };
    work.then(settle, () => settle(undefined));
  });
}

/** Where a preview is drawn and encoded: an offscreen canvas where there is one. */
interface Surface {
  encode(type: string, quality: number): Promise<Blob | null>;
  /** The drawn pixels, not premultiplied. */
  read(): Pixels;
}

function surfaceFor(bitmap: ImageBitmap, width: number, height: number): Surface | null {
  if (typeof OffscreenCanvas === "function") {
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext("2d");
    if (context === null) return null;
    draw(context, bitmap, width, height);
    return {
      encode: (type, quality) => canvas.convertToBlob({ type, quality }),
      read: () => context.getImageData(0, 0, width, height),
    };
  }
  if (typeof document === "undefined") return null;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (context === null) return null;
  draw(context, bitmap, width, height);
  return {
    encode: (type, quality) => new Promise((resolve) => canvas.toBlob(resolve, type, quality)),
    read: () => context.getImageData(0, 0, width, height),
  };
}

function draw(
  context: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D,
  bitmap: ImageBitmap,
  width: number,
  height: number,
): void {
  context.imageSmoothingEnabled = true;
  context.imageSmoothingQuality = "high";
  context.drawImage(bitmap, 0, 0, width, height);
}

/**
 * The picture described in a few bytes, from the first frame of an animation
 * too. Undefined when nothing could be drawn: the upload goes on without it.
 */
function makePlaceholder(bitmap: ImageBitmap): string | undefined {
  try {
    if (bitmap.width <= 0 || bitmap.height <= 0) return undefined;
    const scale = Math.min(1, PLACEHOLDER_MAX_EDGE / Math.max(bitmap.width, bitmap.height));
    const width = Math.max(1, Math.round(bitmap.width * scale));
    const height = Math.max(1, Math.round(bitmap.height * scale));
    const surface = surfaceFor(bitmap, width, height);
    if (surface === null) return undefined;
    return placeholderToBase64(encodePlaceholder(surface.read()));
  } catch {
    return undefined;
  }
}

/**
 * The reduced copy of an image, or undefined when the original should be used
 * as it is: an animation, a vector, a file that is already small, or a browser
 * that cannot encode a copy that keeps what the original has.
 */
async function makePreview(bitmap: ImageBitmap, file: File): Promise<Blob | undefined> {
  if (!mayHavePreview(file.type) || bitmap.width <= 0 || bitmap.height <= 0) return undefined;
  const head = new Uint8Array(await file.slice(0, ANIMATION_HEAD_BYTES).arrayBuffer());
  if (isAnimatedImage(head)) return undefined;

  const size = previewSize(bitmap.width, bitmap.height);
  const surface = surfaceFor(bitmap, size.width, size.height);
  if (surface === null) return undefined;

  let encoded = await surface.encode("image/webp", PREVIEW_WEBP_QUALITY);
  // A browser that cannot encode WebP quietly answers with a PNG. JPEG is the
  // fallback only for a JPEG original, since it cannot hold transparency.
  if (encoded?.type !== "image/webp") {
    if (file.type !== "image/jpeg") return undefined;
    encoded = await surface.encode("image/jpeg", PREVIEW_JPEG_QUALITY);
    if (encoded?.type !== "image/jpeg") return undefined;
  }
  return previewIsWorthSending(encoded.size, file.size) ? encoded : undefined;
}

async function probePlayable(
  file: File,
  kind: "audio" | "video",
): Promise<UploadMediaMetadata> {
  if (typeof document === "undefined") return {};

  const video = kind === "video" ? document.createElement("video") : null;
  const element: HTMLMediaElement = video ?? document.createElement("audio");
  if (video) {
    // A poster needs one decoded frame on top of the container metadata.
    video.preload = "auto";
    video.muted = true;
    video.playsInline = true;
  } else {
    element.preload = "metadata";
  }

  const url = URL.createObjectURL(file);
  try {
    return await new Promise<UploadMediaMetadata>((resolve) => {
      const timer = setTimeout(() => resolve({}), PROBE_TIMEOUT_MS);
      const settle = (metadata: UploadMediaMetadata) => {
        clearTimeout(timer);
        resolve(metadata);
      };
      element.onerror = () => settle({});
      element.onloadedmetadata = () => {
        measure(element, video).then(settle, () => settle({}));
      };
      element.src = url;
      element.load();
    });
  } finally {
    element.removeAttribute("src");
    element.load();
    URL.revokeObjectURL(url);
  }
}

async function measure(
  element: HTMLMediaElement,
  video: HTMLVideoElement | null,
): Promise<UploadMediaMetadata> {
  const metadata: UploadMediaMetadata = {};
  if (Number.isFinite(element.duration) && element.duration > 0) {
    metadata.durationSeconds = element.duration;
  }
  if (video === null) return metadata;

  if (video.videoWidth > 0 && video.videoHeight > 0) {
    metadata.width = video.videoWidth;
    metadata.height = video.videoHeight;
  }
  const poster = await capturePoster(video);
  if (poster !== undefined) metadata.poster = poster;
  return metadata;
}

async function capturePoster(video: HTMLVideoElement): Promise<Blob | undefined> {
  if (
    video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA &&
    !(await frameLoaded(video))
  ) {
    return undefined;
  }
  const { videoWidth, videoHeight } = video;
  if (videoWidth <= 0 || videoHeight <= 0) return undefined;

  const scale = Math.min(1, POSTER_MAX_WIDTH / videoWidth, POSTER_MAX_HEIGHT / videoHeight);
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(videoWidth * scale));
  canvas.height = Math.max(1, Math.round(videoHeight * scale));
  const context = canvas.getContext("2d");
  if (context === null) return undefined;

  context.drawImage(video, 0, 0, canvas.width, canvas.height);
  return new Promise((resolve) => {
    canvas.toBlob((blob) => resolve(blob ?? undefined), "image/jpeg", POSTER_JPEG_QUALITY);
  });
}

/** Resolves true once the first frame is decoded, false if that takes too long. */
function frameLoaded(video: HTMLVideoElement): Promise<boolean> {
  return new Promise((resolve) => {
    const onLoaded = () => finish(true);
    const finish = (loaded: boolean) => {
      clearTimeout(timer);
      video.removeEventListener("loadeddata", onLoaded);
      resolve(loaded);
    };
    const timer = setTimeout(() => finish(false), FRAME_TIMEOUT_MS);
    video.addEventListener("loadeddata", onLoaded, { once: true });
  });
}
