import type { CSSProperties } from "react";

export function videoAspectRatio(width: number | undefined, height: number | undefined): number | undefined {
  if (width === undefined || height === undefined || width <= 0 || height <= 0) return undefined;
  const ratio = width / height;
  return Number.isFinite(ratio) && ratio > 0 ? ratio : undefined;
}

export function videoFrameStyle(ratio: number | undefined): CSSProperties | undefined {
  return ratio === undefined ? undefined : { "--video-ratio": ratio } as CSSProperties;
}
