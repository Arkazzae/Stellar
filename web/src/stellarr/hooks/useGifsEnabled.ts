import { useEffect, useState, useSyncExternalStore } from "react";

import { answeredGiphyConfig, readGiphyConfig } from "@/lib/giphy/giphy";
import { giphyChoice, type GiphyAccess } from "@/lib/giphy/giphyChoice";

/**
 * Whether the operator offers GIFs, as Stellarr's API says; false until it
 * has answered, and false when it cannot be asked.
 */
export function useGifsEnabled(): boolean {
  const [enabled, setEnabled] = useState(() => answeredGiphyConfig()?.enabled ?? false);

  useEffect(() => {
    let live = true;
    readGiphyConfig().then(
      (config) => {
        if (live) setEnabled(config.enabled);
      },
      () => {
        if (live) setEnabled(false);
      },
    );
    return () => {
      live = false;
    };
  }, []);

  return enabled;
}

/** What the person's own switch lets this browser send to GIPHY now. */
export function useGiphyAccess(): GiphyAccess {
  return useSyncExternalStore(giphyChoice.subscribe, giphyChoice.access, giphyChoice.access);
}

/** The person's switch as the settings show it. */
export function useGiphyEnabled(): boolean {
  return useSyncExternalStore(giphyChoice.subscribe, giphyChoice.isEnabled, giphyChoice.isEnabled);
}
