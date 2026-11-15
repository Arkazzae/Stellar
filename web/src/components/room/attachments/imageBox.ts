import type { CSSProperties } from "react";

/**
 * The size an image was measured at when it was uploaded, as the custom
 * properties `.att-image.reserved` builds its box from. Undefined when the
 * size is unknown or unusable, which leaves the image to size itself on load.
 *
 * Only the measurement crosses over: the caps (the column, the 400 px width,
 * the 300 px height) stay in the stylesheet that applies them to the loaded
 * image, so the reserved box cannot drift from it.
 */
export function reservedImageStyle(width?: number, height?: number): CSSProperties | undefined {
  if (width === undefined || height === undefined) return undefined;
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    return undefined;
  }
  return {
    "--att-image-w": `${width}px`,
    "--att-image-ratio": `${width} / ${height}`,
  } as CSSProperties;
}
