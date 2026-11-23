// The faces a person may choose and the rules that pick one: with the
// signed-in app, not the first page. Only the chosen face is downloaded.
import "@/styles/themes/font-pack.css";

import { useLayoutEffect } from "react";

import { useTheme } from "@/state/hooks";

/**
 * What of the theme belongs to the signed-in app alone: the face it is set
 * in. The sign-in pages stay in Outfit, the brand's face, even after they
 * have fetched the app ahead of signing in, because the attribute that picks
 * another face is written only here, as the app mounts; a layout effect, so
 * the app's first frame already has it.
 */
export function AppAppearance() {
  const { theme } = useTheme();

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.dataset.font = theme.font;
    return () => {
      delete root.dataset.font;
    };
  }, [theme.font]);

  return null;
}
