import { WALLPAPER_SHADE_DEFAULT, type ThemePreference, type WallpaperSettings } from "@/design/themes";
import type { MessageKey, Translator } from "@/i18n";
import { prepareWallpaper, type WallpaperProblem } from "@/lib/wallpaper/prepareWallpaper";

/** The account's wallpaper as the appearance pane reaches it. */
export interface WallpaperAccess {
  /** `/assets/{id}`, or undefined when the account has none. */
  url?: string;
  /**
   * Uploads the prepared picture to the account. A refusal resolves with the
   * server's own sentence when it gave one (too large, no room left).
   */
  upload: (file: File) => Promise<{ ok: true } | { ok: false; message?: string }>;
  remove: () => Promise<void>;
  /** The picture's bytes, for a theme file that carries it. */
  read: () => Promise<Blob>;
}

/** A picture as a `data:` address, for a theme file. */
export function dataAddressOf(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error("The picture could not be read."));
    reader.readAsDataURL(blob);
  });
}

const PROBLEMS: Record<WallpaperProblem, MessageKey> = {
  tooLarge: "theme.wallpaper.tooLarge",
  vector: "theme.wallpaper.vector",
  undecodable: "theme.wallpaper.undecodable",
};

export type WallpaperOutcome =
  | { ok: true; firstFrameOnly: boolean }
  | { ok: false; message: string };

/**
 * Makes a picture the account's wallpaper: decoded and re-encoded here
 * (`prepareWallpaper`), uploaded, and only then recorded in the theme with
 * its stand-in. Every way a wallpaper arrives goes through here: a file
 * picked in the pane, one embedded in a theme file, and one a converted
 * theme names. `settings` carries the shading and blur a file or a theme
 * came with; a picked file keeps the ones already in use.
 */
export async function applyWallpaper(
  input: Blob,
  access: WallpaperAccess,
  theme: ThemePreference,
  onChange: (patch: Partial<ThemePreference>) => void,
  t: Translator,
  settings?: Pick<WallpaperSettings, "shade" | "blur">,
): Promise<WallpaperOutcome> {
  const prepared = await prepareWallpaper(input);
  if ("problem" in prepared) return { ok: false, message: t(PROBLEMS[prepared.problem]) };
  const uploaded = await access
    .upload(prepared.file)
    .catch((): { ok: false; message?: string } => ({ ok: false }));
  if (!uploaded.ok) return { ok: false, message: uploaded.message ?? t("theme.wallpaper.failed") };
  onChange({
    wallpaper: {
      shade: settings?.shade ?? theme.wallpaper?.shade ?? WALLPAPER_SHADE_DEFAULT,
      blur: settings?.blur ?? theme.wallpaper?.blur ?? 0,
      ...(prepared.placeholder === undefined ? {} : { placeholder: prepared.placeholder }),
    },
  });
  return { ok: true, firstFrameOnly: prepared.firstFrameOnly };
}
