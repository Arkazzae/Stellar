import { WALLPAPER_INPUT_MAX_BYTES } from "@/lib/wallpaper/prepareWallpaper";

import { convertTheme, fetchCapped, stylesheetAddress, type Conversion } from "./convert";
import { browserEvaluator } from "./evaluate";
import { INLINE_LIMITS } from "./text";

/**
 * The converter as the dialog uses it, in one module the dialog imports only
 * when it is first asked to read a theme.
 */

const fetchSheet = async (url: string) => (await fetchCapped(url, INLINE_LIMITS.total)).text();

export function convertText(text: string): Promise<Conversion> {
  return convertTheme(text, null, { fetchSheet, evaluator: browserEvaluator() });
}

export async function convertAddress(
  input: string,
): Promise<{ conversion: Conversion; source: string } | { problem: "notHttps" | "betterDiscordSite" | "invalid" }> {
  const address = stylesheetAddress(input);
  if ("problem" in address) return address;
  const text = await fetchSheet(address.url);
  return {
    conversion: await convertTheme(text, address.url, { fetchSheet, evaluator: browserEvaluator() }),
    source: address.url,
  };
}

/** The picture a theme names, read the way its stylesheets are and within the wallpaper's cap. */
export function fetchPicture(url: string): Promise<Blob> {
  return fetchCapped(url, WALLPAPER_INPUT_MAX_BYTES);
}
