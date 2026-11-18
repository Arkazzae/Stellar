import { useSyncExternalStore } from "react";

import { releaseWatcher, type ReleaseState } from "@/lib/platform/release";

/**
 * Whether this page runs the release the server serves. A store of its own
 * and not a context, so a screen drawn outside the providers can read it.
 */
export function useReleaseState(): ReleaseState {
  const watcher = releaseWatcher();
  return useSyncExternalStore(watcher.subscribe, () => watcher.state);
}
