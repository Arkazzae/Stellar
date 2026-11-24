import { useEffect } from "react";

import { loadSignedInCode } from "@/adapters/signedInLoader";
import type * as AppModule from "@/containers/AppContainer";

let loading: Promise<typeof AppModule> | undefined;

/**
 * Loads the signed-in shell and its socket and voice implementations together.
 * Every caller shares the pending request or the module it resolved to.
 * A failure leaves the next demand free to try again.
 */
export function loadApp(): Promise<typeof AppModule> {
  return (loading ??= Promise.all([import("@/containers/AppContainer"), loadSignedInCode()])
    .then(([app]) => app)
    .catch((cause: unknown) => {
      loading = undefined;
      throw cause;
    }));
}

export function canPrefetchApp(): boolean {
  const { connection } = navigator as Navigator & {
    connection?: { saveData?: boolean; effectiveType?: string };
  };
  const effectiveType = connection?.effectiveType;
  return (
    !connection?.saveData &&
    effectiveType !== "slow-2g" &&
    effectiveType !== "2g" &&
    effectiveType !== "3g"
  );
}

/** How long a page waits for the browser to be idle before fetching the app anyway. */
const IDLE_TIMEOUT_MS = 2000;

/**
 * Fetches the signed-in app in the background once this page is usable and
 * the browser has a moment. An active sign-in can start the request immediately
 * if idle work has yet to run. On a constrained connection the app waits until
 * the session needs it.
 */
export function usePrefetchApp(usable: boolean, signingIn = false): void {
  useEffect(() => {
    if (!usable || !canPrefetchApp()) return;
    const fetchApp = () => void loadApp().catch(() => undefined);
    if (signingIn) {
      fetchApp();
      return;
    }
    if (typeof window.requestIdleCallback === "function") {
      const handle = window.requestIdleCallback(fetchApp, { timeout: IDLE_TIMEOUT_MS });
      return () => window.cancelIdleCallback(handle);
    }
    // Safari has no idle callback; after the first frames is close enough.
    const handle = window.setTimeout(fetchApp, IDLE_TIMEOUT_MS / 2);
    return () => window.clearTimeout(handle);
  }, [signingIn, usable]);
}
