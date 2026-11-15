import { useSyncExternalStore } from "react";

const subscribe = () => () => undefined;

/**
 * False while a page is rendered ahead of time and while React hydrates that
 * render, true from the render after hydration. A component that is only
 * ever rendered in the browser, as in tests or after a client-side
 * navigation, sees true from its first render.
 */
export function useHydrated(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => true,
    () => false,
  );
}
