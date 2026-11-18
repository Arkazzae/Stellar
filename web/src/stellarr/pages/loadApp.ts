import { useEffect } from "react";

import { loadSignedInCode } from "@/adapters/signedInLoader";

/**
 * The signed-in app, a chunk of its own and most of the client's code, and
 * with it the socket and voice code that only it uses. Every caller shares
 * the one set of modules the browser fetches. A failure of either rejects, so
 * the home page shows the app as unavailable instead of an app whose socket
 * never opens.
 */
export const loadApp = () =>
  Promise.all([import("@/containers/AppContainer"), loadSignedInCode()]).then(([app]) => app);

/** How long a page waits for the browser to be idle before fetching the app anyway. */
const IDLE_TIMEOUT_MS = 2000;

/**
 * Fetches the signed-in app in the background once this page is usable and
 * the browser has a moment, so that signing in opens it without a wait. For
 * the pages that lead to the app: sign-in, registration and invites. Until
 * `usable`, the page's own requests (the session check, the setup question)
 * have the connection to themselves. A failure is the home page's to show,
 * when the app is wanted.
 */
export function usePrefetchApp(usable: boolean): void {
  useEffect(() => {
    if (!usable) return;
    const fetchApp = () => void loadApp().catch(() => undefined);
    if (typeof window.requestIdleCallback === "function") {
      const handle = window.requestIdleCallback(fetchApp, { timeout: IDLE_TIMEOUT_MS });
      return () => window.cancelIdleCallback(handle);
    }
    // Safari has no idle callback; after the first frames is close enough.
    const handle = window.setTimeout(fetchApp, IDLE_TIMEOUT_MS / 2);
    return () => window.clearTimeout(handle);
  }, [usable]);
}
