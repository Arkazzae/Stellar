import "@/styles/themes/wallpaper.css";

import { useLayoutEffect, useMemo, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";

import type { WallpaperSettings } from "@/design/themes";
import { placeholderDataUrl, placeholderFromBase64 } from "@/lib/media/placeholder";

/** Only a picture on our own host, as the account gives it: `/assets/{id}`. */
export function wallpaperAddress(url: string | undefined): string | undefined {
  return url !== undefined && /^\/assets\/[\da-f-]{36}$/i.test(url) ? url : undefined;
}

/**
 * The picture, its stand-in and the shading over it, filling whatever box
 * holds it: the fixed layer behind the app, or the miniature in the
 * appearance pane. The stand-in is drawn from the synced theme's few bytes by
 * our own decoder, so it paints before the file arrives and on a device that
 * has never fetched it.
 */
export function WallpaperPicture({
  settings,
  url,
  className,
}: {
  settings: WallpaperSettings;
  url: string | undefined;
  className: string;
}) {
  const [loaded, setLoaded] = useState<string | null>(null);
  const placeholder = useMemo(() => {
    const bytes = settings.placeholder === undefined ? null : placeholderFromBase64(settings.placeholder);
    return bytes === null ? null : placeholderDataUrl(bytes);
  }, [settings.placeholder]);
  // Blurring fades the edges, so the picture is drawn a little larger than its box.
  const blurred: CSSProperties =
    settings.blur === 0 ? {} : { filter: `blur(${settings.blur}px)`, transform: `scale(${1 + settings.blur / 120})` };

  return (
    <div className={className} aria-hidden="true">
      {placeholder === null ? null : <img className="wallpaper-placeholder" src={placeholder} alt="" />}
      {url === undefined ? null : (
        <img
          className={loaded === url ? "wallpaper-picture on" : "wallpaper-picture"}
          src={url}
          alt=""
          decoding="async"
          style={blurred}
          onLoad={() => setLoaded(url)}
        />
      )}
      <div className="wallpaper-shade" style={{ opacity: settings.shade / 100 }} />
    </div>
  );
}

export interface WallpaperLayerProps {
  /** The theme's wallpaper settings, null for none. */
  settings: WallpaperSettings | null;
  /** The account's picture, as it gave it. */
  url: string | undefined;
  /** Whether the account has loaded, and so whether `url` can be believed. */
  known: boolean;
}

/**
 * The account's wallpaper behind the whole signed-in app, on one fixed layer.
 *
 * It alone writes `data-wallpaper`, in a layout effect, so the app's first
 * frame already has the see-through surfaces and nothing else ever does: not
 * the sign-in pages, which fetch the app's stylesheets ahead of signing in.
 * The attribute stays only while there is a picture to show. Until the
 * account has loaded that is not known, so the stand-in and the shading hold
 * the place; an account without a picture takes the attribute away, and so
 * does signing out, which unmounts this.
 */
export function WallpaperLayer({ settings, url: given, known }: WallpaperLayerProps) {
  const url = wallpaperAddress(given);
  const shown = settings !== null && (url !== undefined || !known);

  useLayoutEffect(() => {
    const root = document.documentElement;
    if (shown) root.dataset.wallpaper = "on";
    else delete root.dataset.wallpaper;
    return () => {
      delete root.dataset.wallpaper;
    };
  }, [shown]);

  if (!shown || settings === null) return null;
  return createPortal(<WallpaperPicture settings={settings} url={url} className="wallpaper" />, document.body);
}
