import { ANIMATION_HEAD_BYTES, isAnimatedImage } from "@/lib/media/imagePreview";
import { makePlaceholder, surfaceFor } from "@/lib/media/probe";

/**
 * A picture made ready to be a wallpaper: decoded by the browser, fitted to
 * WALLPAPER_MAX_EDGE on its longer side and encoded again, as WebP where the
 * browser can and JPEG where it cannot. Drawing it through a canvas leaves
 * behind everything the file carried besides its pixels, a phone photo's
 * location included, and anything a decoder would act on: what is uploaded
 * is only ever what this browser drew.
 *
 * Every way in passes here: a file picked from disk, a wallpaper embedded in
 * a theme file, and the one a converted theme names.
 */

/** Sharp on a 1440p screen, and a few hundred kilobytes as WebP. */
export const WALLPAPER_MAX_EDGE = 2560;
/** What is read at all; a larger file is refused before it is decoded. */
export const WALLPAPER_INPUT_MAX_BYTES = 25 * 1024 * 1024;
export const WALLPAPER_WEBP_QUALITY = 0.86;
export const WALLPAPER_JPEG_QUALITY = 0.86;

export type WallpaperProblem = "tooLarge" | "vector" | "undecodable";

export interface PreparedWallpaper {
  file: File;
  width: number;
  height: number;
  /** The picture in a few bytes, base64, for the synced theme. */
  placeholder?: string;
  /** The file was an animation and only its first frame was kept. */
  firstFrameOnly: boolean;
}

/** The size the wallpaper is drawn at: inside the square, never larger than the original. */
export function wallpaperSize(width: number, height: number): { width: number; height: number } {
  const scale = Math.min(1, WALLPAPER_MAX_EDGE / width, WALLPAPER_MAX_EDGE / height);
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

/**
 * SVG, by its type or by how it starts: a vector picture is a document that
 * can name other files and run its own animation, not a photo to stretch.
 */
export function isVectorImage(head: Uint8Array, type: string): boolean {
  if (type === "image/svg+xml") return true;
  // trimStart removes a byte order mark too.
  const text = new TextDecoder().decode(head.subarray(0, 512)).trimStart().toLowerCase();
  return text.startsWith("<svg") || (text.startsWith("<?xml") && text.includes("<svg"));
}

export async function prepareWallpaper(input: Blob): Promise<PreparedWallpaper | { problem: WallpaperProblem }> {
  if (input.size > WALLPAPER_INPUT_MAX_BYTES) return { problem: "tooLarge" };
  const head = new Uint8Array(await input.slice(0, ANIMATION_HEAD_BYTES).arrayBuffer());
  if (isVectorImage(head, input.type)) return { problem: "vector" };
  if (typeof createImageBitmap !== "function") return { problem: "undecodable" };

  let bitmap: ImageBitmap;
  try {
    // An animation decodes to its first frame.
    bitmap = await createImageBitmap(input, { imageOrientation: "from-image" });
  } catch {
    return { problem: "undecodable" };
  }
  try {
    if (bitmap.width <= 0 || bitmap.height <= 0) return { problem: "undecodable" };
    const size = wallpaperSize(bitmap.width, bitmap.height);
    const surface = surfaceFor(bitmap, size.width, size.height);
    if (surface === null) return { problem: "undecodable" };

    let encoded = await surface.encode("image/webp", WALLPAPER_WEBP_QUALITY);
    // A browser that cannot encode WebP quietly answers with a PNG.
    if (encoded?.type !== "image/webp") encoded = await surface.encode("image/jpeg", WALLPAPER_JPEG_QUALITY);
    if (encoded === null || (encoded.type !== "image/webp" && encoded.type !== "image/jpeg")) {
      return { problem: "undecodable" };
    }
    const extension = encoded.type === "image/webp" ? "webp" : "jpg";
    const placeholder = makePlaceholder(bitmap);
    return {
      file: new File([encoded], `wallpaper.${extension}`, { type: encoded.type }),
      ...size,
      ...(placeholder === undefined ? {} : { placeholder }),
      firstFrameOnly: isAnimatedImage(head),
    };
  } finally {
    bitmap.close();
  }
}
