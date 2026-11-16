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

const SLASH = "/".charCodeAt(0);

/**
 * The one form of a path the router works with: every run of slashes is one
 * slash, and there is no trailing slash, except for the root. `/login/`,
 * `//login` and `/login` are the same page, and the address bar is corrected
 * to the last. nginx merges the slashes the same way before it picks the
 * document, so the page shown is the page the server answered for.
 *
 * A path that started with two slashes would also be read as another host
 * if it were written back to the address bar as it is, and the browser
 * refuses that with a SecurityError.
 *
 * The path comes from the address bar and from links, so anyone can write
 * it. It is walked once, from start to end: a pattern such as `/\/+$/` would
 * retry every slash of a long run that does not end the path, which takes
 * time quadratic in its length and can freeze the tab.
 */
export function canonicalPath(pathname: string): string {
  const segments: string[] = [];
  let start = -1;
  for (let index = 0; index <= pathname.length; index += 1) {
    if (index === pathname.length || pathname.charCodeAt(index) === SLASH) {
      if (start !== -1) segments.push(pathname.slice(start, index));
      start = -1;
    } else if (start === -1) {
      start = index;
    }
  }
  return `/${segments.join("/")}`;
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
