import { useEffect } from "react";

import { releaseWatcher, runningRelease, watchRelease } from "@/lib/platform/release";

import { useRuntime } from "./PortsProvider";

/**
 * Starts the check that keeps the page on the server's release
 * (`lib/platform/release.ts`) and draws nothing. The desktop client is left
 * out: its shell is its own, and reloading it would not bring it a release.
 */
export function ReleaseWatcher() {
  const { ports } = useRuntime();
  useEffect(() => {
    if (ports.desktop !== undefined || runningRelease() === null) return;
    return watchRelease(releaseWatcher(), {
      connection: ports.connection,
      document,
      window,
      now: () => Date.now(),
    });
  }, [ports]);
  return null;
}
