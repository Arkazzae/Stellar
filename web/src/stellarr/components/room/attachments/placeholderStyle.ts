import type { CSSProperties } from "react";

import { placeholderUrlFor } from "@/lib/media/placeholderUrl";

/** The blurred stand-in as a background, for an image that has not loaded yet; undefined draws nothing. */
export function placeholderStyle(text: string | undefined): CSSProperties | undefined {
  const url = placeholderUrlFor(text);
  if (url === null) return undefined;
  return { backgroundImage: `url("${url}")`, backgroundSize: "100% 100%" };
}
