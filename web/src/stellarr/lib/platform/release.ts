import type { ConnectionPort, ConnectionStatus } from "@/ports/presence";

import { keepShell } from "./worker";

/**
 * Keeps a running page on the release the server serves.
 *
 * The build gives each release an id (`web/release.ts`) and publishes it
 * twice: in a <meta> of the web client's document, which says which release
 * this page is, and in /release.json, which says which the server serves now.
 * A page that finds them different reloads at once; nothing is asked and
 * nothing is shown. Whoever draws an update screen reads `state` and calls
 * `check`.
 *
 * Nothing here is ever certain that a release is newer. A page with no id (the
 * dev server, tests), a request that fails, an answer that is no release file:
 * each leaves the page where it is.
 */

/** Where the build writes the release file and its id into the document; repeats `web/release.ts`. */
export const RELEASE_PATH = "/release.json";
export const RELEASE_META = "stellarr-release";

/** The browser's storage key of the id this tab last reloaded for. */
export const RELOADED_FOR_KEY = "stellarr.release-reloaded-for";

/** A tab that becomes visible asks at most this often. */
export const VISIBLE_CHECK_INTERVAL_MS = 60_000;

/** How long the question to the server may take before it counts as unanswered. */
const FETCH_TIMEOUT_MS = 10_000;

/**
 * `current`: this page runs what the server serves, or nothing is known to
 * say otherwise. `updating`: a newer release was found and the page is being
 * reloaded. `stuck`: the page reloaded for a release and still runs an older
 * one, so it holds still instead of reloading again; the next release
 * reloads it once more.
 */
export type ReleaseState = "current" | "updating" | "stuck";

export interface ReleaseWatcherDeps {
  /** The id of the release this page is, or null when it has none. */
  running: string | null;
  /** The id of the release the server serves now; null when it cannot tell. */
  fetchLatest(): Promise<string | null>;
  /** The id this tab last reloaded for. May throw, as storage does. */
  readReloadedFor(): string | null;
  /** May throw, as storage does. */
  writeReloadedFor(id: string): void;
  reload(): void;
  /**
   * Has the service worker keep the shell of the release the server serves
   * now (`keepShell` in `./worker`). False means the worker could not, and
   * the page must not reload. Absent where there is no worker to ask.
   */
  keepShell?(): Promise<boolean>;
}

export interface ReleaseWatcher {
  readonly state: ReleaseState;
  subscribe(listener: () => void): () => void;
  /**
   * Asks the server now. A call that overlaps one under way is answered by
   * it, and by one more question after it. A call starts the retries over: see
   * `RETRY_FIRST_MS`.
   */
  check(): Promise<void>;
  /** Cancels a retry that is waiting. */
  stop(): void;
}

/**
 * When a check could not settle, because the server gave no answer or the
 * worker could not keep the shell, the page asks again after this long and
 * then after twice as long, up to the cap, until it is current or stuck. A
 * tab left open and visible would otherwise stay on the old release until it
 * was hidden and shown again.
 */
export const RETRY_FIRST_MS = 15_000;
export const RETRY_MAX_MS = 300_000;

export function createReleaseWatcher(deps: ReleaseWatcherDeps): ReleaseWatcher {
  const listeners = new Set<() => void>();
  let state: ReleaseState = "current";
  let asking: Promise<void> | null = null;
  let askedAgain = false;

  const setState = (next: ReleaseState) => {
    if (state === next) return;
    state = next;
    for (const listener of [...listeners]) listener();
  };

  let retryTimer: ReturnType<typeof setTimeout> | undefined;
  let retryDelay = RETRY_FIRST_MS;

  /** Resolves to "retry" when the check could not settle whether the page is current. */
  async function ask(): Promise<"done" | "retry"> {
    if (deps.running === null) return "done";
    const latest = await deps.fetchLatest();
    if (latest === null) return "retry";
    if (latest === deps.running) {
      setState("current");
      // Not waited for: a worker that has no copy of this release yet takes
      // it now, for the next load.
      void deps.keepShell?.();
      return "done";
    }
    // One reload per release id, so a release that a reload does not bring
    // cannot send the page round in circles. A tab that cannot remember
    // what it reloaded for is held the same way.
    try {
      if (deps.readReloadedFor() === latest) {
        setState("stuck");
        return "done";
      }
    } catch {
      setState("stuck");
      return "done";
    }
    // With a worker in control a plain reload would bring the old shell back,
    // so the worker stores the new one first. When it cannot, the page stays
    // as it is and the next check tries again.
    if (deps.keepShell !== undefined) {
      setState("updating");
      if (!(await deps.keepShell())) {
        setState("current");
        return "retry";
      }
    }
    try {
      deps.writeReloadedFor(latest);
    } catch {
      setState("stuck");
      return "done";
    }
    setState("updating");
    deps.reload();
    return "done";
  }

  return {
    get state() {
      return state;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    check() {
      return start(false);
    },
    stop() {
      clearTimeout(retryTimer);
      retryTimer = undefined;
    },
  };

  function start(retrying: boolean): Promise<void> {
    if (!retrying) {
      clearTimeout(retryTimer);
      retryTimer = undefined;
      retryDelay = RETRY_FIRST_MS;
    }
    // A check asked for while one is under way may mean the server has moved
    // on since that one asked, so it gets one more when the first is done.
    if (asking !== null) {
      if (!retrying) askedAgain = true;
      return asking;
    }
    asking = ask()
      .then((outcome) => {
        if (outcome !== "retry" || retryTimer !== undefined) return;
        retryTimer = setTimeout(() => {
          retryTimer = undefined;
          void start(true);
        }, retryDelay);
        retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
      })
      .finally(() => {
        asking = null;
        if (!askedAgain) return;
        askedAgain = false;
        void start(true);
      });
    return asking;
  }
}

export interface ReleaseTriggers {
  connection: ConnectionPort;
  /** The page's document and window, or stand-ins. */
  document: Pick<Document, "visibilityState" | "addEventListener" | "removeEventListener">;
  window: Pick<Window, "addEventListener" | "removeEventListener">;
  now(): number;
}

/**
 * Makes `watcher` ask at the moments a page may have missed a release: when
 * it starts, when the hub connection comes back, when the tab becomes visible
 * again (at most once a minute) and when the browser comes back online. The
 * sign-in pages have no hub and rely on the last two. Returns what stops it.
 */
export function watchRelease(watcher: ReleaseWatcher, triggers: ReleaseTriggers): () => void {
  let lastVisibleCheck = Number.NEGATIVE_INFINITY;
  let dropped = false;
  let status: ConnectionStatus = triggers.connection.getStatus();

  const onConnection = () => {
    const next = triggers.connection.getStatus();
    if (next === status) return;
    status = next;
    if (next === "reconnecting" || next === "disconnected") dropped = true;
    else if (next === "connected" && dropped) {
      dropped = false;
      void watcher.check();
    }
  };
  const onVisibility = () => {
    if (triggers.document.visibilityState !== "visible") return;
    const now = triggers.now();
    if (now - lastVisibleCheck < VISIBLE_CHECK_INTERVAL_MS) return;
    lastVisibleCheck = now;
    void watcher.check();
  };
  const onOnline = () => void watcher.check();

  const unsubscribe = triggers.connection.subscribe(onConnection);
  triggers.document.addEventListener("visibilitychange", onVisibility);
  triggers.window.addEventListener("online", onOnline);
  void watcher.check();
  return () => {
    watcher.stop();
    unsubscribe();
    triggers.document.removeEventListener("visibilitychange", onVisibility);
    triggers.window.removeEventListener("online", onOnline);
  };
}

/** The release this page is, from its document; null when the build wrote none. */
export function runningRelease(doc: Document = document): string | null {
  const id = doc.querySelector(`meta[name="${RELEASE_META}"]`)?.getAttribute("content");
  return id === null || id === undefined || id === "" ? null : id;
}

/** The release the server serves now, asked past every cache; null when it cannot be told. */
export async function fetchLatestRelease(): Promise<string | null> {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), FETCH_TIMEOUT_MS);
  try {
    const response = await fetch(RELEASE_PATH, { cache: "no-store", credentials: "same-origin", signal: abort.signal });
    if (!response.ok) return null;
    const { id } = (await response.json()) as { id?: unknown };
    return typeof id === "string" && id !== "" ? id : null;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

let shared: ReleaseWatcher | undefined;

/** This page's watcher, made on first use from the document, the tab's storage and the network. */
export function releaseWatcher(): ReleaseWatcher {
  shared ??= createReleaseWatcher({
    running: runningRelease(),
    fetchLatest: fetchLatestRelease,
    readReloadedFor: () => window.sessionStorage.getItem(RELOADED_FOR_KEY),
    writeReloadedFor: (id) => window.sessionStorage.setItem(RELOADED_FOR_KEY, id),
    reload: () => window.location.reload(),
    keepShell,
  });
  return shared;
}

/** Asks the server now whether this page runs the release it serves. */
export function checkRelease(): Promise<void> {
  return releaseWatcher().check();
}
