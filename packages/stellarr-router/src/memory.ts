import { canonicalPath, entryKey, formatAddress, type RouterLocation } from "./address";
import type { Address, ChangeKind, RouterHistory } from "./history";

/** One navigation a memory history saw, in the order it saw them. */
export type Navigation =
  | { readonly kind: ChangeKind; readonly href: string }
  /** A page load of an address outside the app, which a browser would follow. */
  | { readonly kind: "leave"; readonly href: string; readonly replace: boolean };

export interface MemoryHistory extends RouterHistory {
  /** The current address as the address bar would show it. */
  readonly href: string;
  /** Every entry, oldest first, as addresses. */
  readonly entries: readonly string[];
  readonly index: number;
  /** Every navigation so far, the first address excluded. */
  readonly navigations: readonly Navigation[];
  back(): void;
  forward(): void;
}

/**
 * An address as the address bar would hold it. A rooted one is read as a
 * path even when it starts with two slashes, which a link would take for
 * another host: the browser loaded the page, so it is on this origin.
 */
function parse(href: string, origin: string): Address {
  const url = href.startsWith("/") ? new URL(`${origin}${href}`) : new URL(href, `${origin}/`);
  return { pathname: canonicalPath(url.pathname), search: url.search, hash: url.hash };
}

/**
 * A history kept in memory, for tests: render a page at an address, then
 * read where it went. Nothing reaches the window, a link out of the app is
 * recorded instead of followed, and scroll and focus are left alone.
 */
export function createMemoryHistory(href = "/", options: { origin?: string } = {}): MemoryHistory {
  const origin = options.origin ?? "http://localhost";
  const listeners = new Set<(kind: ChangeKind) => void>();
  const stack: RouterLocation[] = [{ ...parse(href, origin), key: entryKey() }];
  const navigations: Navigation[] = [];
  let index = 0;

  const current = (): RouterLocation => stack[index] as RouterLocation;

  function notify(kind: ChangeKind): void {
    navigations.push({ kind, href: formatAddress(current()) });
    for (const listener of [...listeners]) listener(kind);
  }

  function go(delta: number): void {
    const target = index + delta;
    if (target < 0 || target >= stack.length) return;
    index = target;
    notify("pop");
  }

  return {
    get location() {
      return current();
    },
    get href() {
      return formatAddress(current());
    },
    get entries() {
      return stack.map(formatAddress);
    },
    get index() {
      return index;
    },
    get navigations() {
      return [...navigations];
    },
    origin,
    native: false,
    listen(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    push(address) {
      stack.splice(index + 1, stack.length, { ...address, key: entryKey() });
      index += 1;
      notify("push");
    },
    replace(address) {
      stack[index] = { ...address, key: current().key };
      notify("replace");
    },
    leave(target, replace) {
      navigations.push({ kind: "leave", href: target, replace });
    },
    back: () => go(-1),
    forward: () => go(1),
  };
}
