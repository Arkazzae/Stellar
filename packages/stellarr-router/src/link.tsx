import type { AnchorHTMLAttributes, MouseEvent, Ref } from "react";

import { resolveAddress } from "./address";
import { useRouterState } from "./router";

export interface LinkProps extends Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> {
  readonly href: string;
  /** Replaces the current history entry instead of adding one. */
  readonly replace?: boolean;
  readonly ref?: Ref<HTMLAnchorElement>;
}

function isPlainClick(event: MouseEvent<HTMLAnchorElement>): boolean {
  return event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey;
}

/**
 * A real `<a href>`. It takes over a click only when the click is a plain
 * primary click without `target` or `download`, and only for an address the
 * app's own table claims; the router then opens it without a page load.
 * Everything else is the browser's: a modified click, a new tab, a download,
 * another app on this origin such as Cockpit, another site, and a link that
 * changes only the fragment. Without a router above it, it is a plain link.
 *
 * The caller's `onClick` runs first, and a caller that prevents the default
 * keeps the click from navigating at all. That is also how it works as the
 * child of an `asChild` component, whose own handlers it receives merged.
 */
export function Link({ href, replace = false, onClick, ref, ...props }: LinkProps) {
  const router = useRouterState();

  function follow(event: MouseEvent<HTMLAnchorElement>) {
    onClick?.(event);
    if (event.defaultPrevented || router === null || !isPlainClick(event)) return;
    const anchor = event.currentTarget;
    const target = anchor.getAttribute("target");
    if ((target !== null && target !== "" && target !== "_self") || anchor.hasAttribute("download")) return;

    const { history, routes, navigator } = router;
    const current = history.location;
    const destination = resolveAddress(href, current, history.origin);
    const fragmentOnly =
      destination.sameOrigin &&
      destination.hash !== "" &&
      destination.pathname === current.pathname &&
      destination.search === current.search;
    if (fragmentOnly) return;

    if (destination.sameOrigin && routes.claims(destination.pathname)) {
      event.preventDefault();
      if (replace) navigator.replace(destination.href);
      else navigator.push(destination.href);
    } else if (!history.native) {
      // Only a browser can load another document; a test's history records it.
      event.preventDefault();
      history.leave(destination.href, replace);
    }
  }

  return <a {...props} ref={ref} href={href} onClick={follow} />;
}
