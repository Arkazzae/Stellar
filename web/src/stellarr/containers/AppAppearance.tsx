// The faces a person may choose and the rules that pick one: with the
// signed-in app, not the first page. Only the chosen face is downloaded.
import "@/styles/themes/font-pack.css";

import { useLayoutEffect } from "react";

import { WallpaperLayer } from "@/components/shell/WallpaperLayer";
import { useAccount, useTheme } from "@/state/hooks";

/**
 * What of the theme belongs to the signed-in app alone: the face it is set
 * in and the wallpaper behind it. The sign-in pages stay in Outfit and
 * opaque, even after they have fetched the app ahead of signing in, because
 * the attributes that pick another face and let a wallpaper through are
 * written only here, as the app mounts; in layout effects, so the app's
 * first frame already has them.
 */
export function AppAppearance() {
  const { theme } = useTheme();
  const { account } = useAccount();

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.dataset.font = theme.font;
    return () => {
      delete root.dataset.font;
    };
  }, [theme.font]);

  return <WallpaperLayer settings={theme.wallpaper} url={account.wallpaperUrl} known={account.id !== ""} />;
}
