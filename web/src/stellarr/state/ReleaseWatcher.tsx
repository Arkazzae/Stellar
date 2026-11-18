import { useEffect } from "react";

import { releaseWatcher, runningRelease, watchRelease } from "@/lib/platform/release";
import { registerWorker } from "@/lib/platform/worker";

import { useRuntime } from "./PortsProvider";

/**
 * Starts the check that keeps the page on the server's release
 * (`lib/platform/release.ts`) and registers the service worker that keeps
 * the shell's local copy (`lib/platform/worker.ts`), and draws nothing. The
 * desktop client is left out of both: its shell is its own, and reloading it
 * would not bring it a release. So is a page with no release in its document,
 * which is the dev server's.
 */
export function ReleaseWatcher() {
  const { ports } = useRuntime();
  useEffect(() => {
    if (ports.desktop !== undefined || runningRelease() === null) return;
    void registerWorker();
    return watchRelease(releaseWatcher(), {
      connection: ports.connection,
      document,
      window,
      now: () => Date.now(),
    });
  }, [ports]);
  return null;
}
