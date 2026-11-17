import { useCallback, useSyncExternalStore } from "react";

/**
 * A media query as external state.
 *
 * The server render and the hydration render both answer `serverValue`, so
 * they cannot disagree; the real answer takes over on the first client commit.
 * The layout is desktop-first, so callers pass the wide-viewport answer.
 */
export function useMediaQuery(query: string, serverValue: boolean): boolean {
  const subscribe = useCallback(
    (onChange: () => void) => {
      if (typeof window === "undefined" || !window.matchMedia) return () => {};
      const list = window.matchMedia(query);
      list.addEventListener("change", onChange);
      return () => {
        list.removeEventListener("change", onChange);
      };
    },
    [query],
  );

  const getSnapshot = useCallback(() => {
    if (typeof window === "undefined" || !window.matchMedia) return serverValue;
    return window.matchMedia(query).matches;
  }, [query, serverValue]);

  return useSyncExternalStore(subscribe, getSnapshot, () => serverValue);
}
