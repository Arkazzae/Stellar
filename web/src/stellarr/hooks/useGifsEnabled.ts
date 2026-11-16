"use client";

import { useEffect, useState } from "react";

import { readGiphyConfig } from "@/lib/giphy/giphy";

/**
 * Whether to offer GIFs, as Stellarr's API says; false until it has answered,
 * and false when it cannot be asked.
 */
export function useGifsEnabled(): boolean {
  const [enabled, setEnabled] = useState(false);

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
