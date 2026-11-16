import { canonicalPath, entryKey, formatAddress, type RouterLocation } from "./address";

/** How the address changed: a new entry, the same entry rewritten, or Back and Forward. */
export type ChangeKind = "push" | "replace" | "pop";

/** An address without its entry key, as a navigation asks for it. */
export type Address = Pick<RouterLocation, "pathname" | "search" | "hash">;

/**
 * What the router reads and moves: the browser's history in an app, a list in
 * memory in a test (`@stellarr/router/testing`).
 */
export interface RouterHistory {
  readonly location: RouterLocation;
  /** The origin addresses are resolved against. */
  readonly origin: string;
  /**
   * True for the browser's history: a link out of the app is left to the
   * browser, and the router keeps the window's scroll position and focus.
   */
  readonly native: boolean;
  /** Hears every change of `location`, after it happened. */
  listen(listener: (kind: ChangeKind) => void): () => void;
  push(address: Address): void;
  replace(address: Address): void;
  /** Loads `href`, a page outside this app, as a new document. */
  leave(href: string, replace: boolean): void;
}

interface EntryState {
  readonly stellarrRouterKey?: unknown;
}

function keyOf(state: unknown): string | null {
  const key = (state as EntryState | null)?.stellarrRouterKey;
  return typeof key === "string" ? key : null;
}

/**
 * The history of `window`. On creation it rewrites the address it was loaded
 * at to the canonical path and gives the entry a key, and it takes scroll
 * restoration over from the browser, which would restore a position before
 * the page it belongs to is drawn again.
 */
export function createBrowserHistory(window: Window): RouterHistory {
  const listeners = new Set<(kind: ChangeKind) => void>();

  /** Reads the current entry, naming it when the browser created it without a key. */
  function read(): RouterLocation {
    const { pathname, search, hash } = window.location;
    let key = keyOf(window.history.state);
    const canonical = canonicalPath(pathname);
    if (key === null || canonical !== pathname) {
      key ??= entryKey();
      window.history.replaceState({ stellarrRouterKey: key }, "", formatAddress({ pathname: canonical, search, hash }));
    }
    return { pathname: canonical, search, hash, key };
  }

  let location = read();
  window.history.scrollRestoration = "manual";

  function notify(kind: ChangeKind): void {
    for (const listener of [...listeners]) listener(kind);
  }

  // A fragment link the browser followed itself changes the entry too; it
  // fires `hashchange`, and in some browsers `popstate` as well.
  function onTraverse(): void {
    const next = read();
    if (next.key === location.key && formatAddress(next) === formatAddress(location)) return;
    location = next;
    notify("pop");
  }
  window.addEventListener("popstate", onTraverse);
  window.addEventListener("hashchange", onTraverse);

  function write(address: Address, kind: "push" | "replace"): void {
    const key = kind === "push" ? entryKey() : location.key;
    const state = { stellarrRouterKey: key };
    if (kind === "push") window.history.pushState(state, "", formatAddress(address));
    else window.history.replaceState(state, "", formatAddress(address));
    location = { ...address, key };
    notify(kind);
  }

  return {
    get location() {
      return location;
    },
    origin: window.location.origin,
    native: true,
    listen(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    push: (address) => write(address, "push"),
    replace: (address) => write(address, "replace"),
    leave(href, replace) {
      if (replace) window.location.replace(href);
      else window.location.assign(href);
    },
  };
}

let shared: RouterHistory | null = null;

/** The history of this window, one for the page however many routers read it. */
export function browserHistory(): RouterHistory {
  shared ??= createBrowserHistory(window);
  return shared;
}
