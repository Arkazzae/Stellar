/** Where the app is: the parts of an address the router reads, and the history entry. */
export interface RouterLocation {
  /** Always starts with `/`, and never ends with one unless it is the root. */
  readonly pathname: string;
  /** `""` or the query with its `?`. */
  readonly search: string;
  /** `""` or the fragment with its `#`. */
  readonly hash: string;
  /** Names the history entry, so a position can be kept for it. */
  readonly key: string;
}

/**
 * The one form of a path the router works with: no trailing slash, except
 * for the root. `/login/` and `/login` are the same page, and the address
 * bar is corrected to the second.
 */
export function canonicalPath(pathname: string): string {
  const trimmed = pathname.replace(/\/+$/, "");
  return trimmed === "" ? "/" : trimmed;
}

/** The address as the address bar shows it, without the origin. */
export function formatAddress(location: Pick<RouterLocation, "pathname" | "search" | "hash">): string {
  return `${location.pathname}${location.search}${location.hash}`;
}

/** Where an `href` leads, read against the page it is followed from. */
export interface ResolvedAddress {
  /** False for another origin, or for a scheme such as `mailto:`. */
  readonly sameOrigin: boolean;
  /** The whole URL, for a page load. */
  readonly href: string;
  readonly pathname: string;
  readonly search: string;
  readonly hash: string;
}

/**
 * Resolves `href` as the browser would from `current` on `origin`, with the
 * path made canonical. An `href` that cannot be parsed resolves to itself
 * on another origin, so it is left to the browser.
 */
export function resolveAddress(
  href: string,
  current: Pick<RouterLocation, "pathname" | "search" | "hash">,
  origin: string,
): ResolvedAddress {
  let url: URL;
  try {
    url = new URL(href, `${origin}${formatAddress(current)}`);
  } catch {
    return { sameOrigin: false, href, pathname: "", search: "", hash: "" };
  }
  const sameOrigin = url.origin === origin;
  const pathname = sameOrigin ? canonicalPath(url.pathname) : url.pathname;
  return {
    sameOrigin,
    href: sameOrigin ? `${pathname}${url.search}${url.hash}` : url.href,
    pathname,
    search: url.search,
    hash: url.hash,
  };
}

/** Whether two addresses show the same page with the same query and fragment. */
export function sameAddress(
  a: Pick<RouterLocation, "pathname" | "search" | "hash">,
  b: Pick<RouterLocation, "pathname" | "search" | "hash">,
): boolean {
  return a.pathname === b.pathname && a.search === b.search && a.hash === b.hash;
}

let counter = 0;

/** A name for a new history entry, unique within this page load. */
export function entryKey(): string {
  counter += 1;
  return `${Date.now().toString(36)}-${counter.toString(36)}`;
}
