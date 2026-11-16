import { useSyncExternalStore, type AnchorHTMLAttributes, type MouseEvent } from "react";

import { COCKPIT_ROOT } from "../lib/paths";

/*
 * A stand-in for Next's router, for the tests of Cockpit's components that
 * read the address or link within the panel. A test file mocks
 * next/navigation with this module and next/link with its Link:
 *
 *   vi.mock("next/navigation", () => import("../test/navigation"));
 *   vi.mock("next/link", () => import("../test/navigation").then(({ Link }) => ({ default: Link })));
 */

let pathname = COCKPIT_ROOT;
const listeners = new Set<() => void>();

/** Opens `path`, as a page load or a followed link would. */
export function visit(path: string): void {
  pathname = path;
  for (const listener of listeners) listener();
}

/** Where the last visit or followed link went. */
export function currentPath(): string {
  return pathname;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePathname(): string {
  return useSyncExternalStore(subscribe, currentPath, currentPath);
}

type LinkProps = Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> & { href: string };

/** next/link as far as these tests need it: an anchor whose click moves the stand-in. */
export function Link({ href, onClick, ...props }: LinkProps) {
  function follow(event: MouseEvent<HTMLAnchorElement>) {
    onClick?.(event);
    if (event.defaultPrevented) return;
    event.preventDefault();
    visit(href);
  }

  return <a {...props} href={href} onClick={follow} />;
}
