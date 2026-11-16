import {
  createContext,
  startTransition,
  Suspense,
  useContext,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { resolveAddress, sameAddress, type RouterLocation } from "./address";
import { browserHistory, type ChangeKind, type RouterHistory } from "./history";
import { NO_ROUTES, type Params, type RouteMatch, type RouteTable } from "./routes";

/** Moves the app to another address. */
export interface Navigator {
  /** Opens `href` in a new history entry, which Back leaves again. */
  push(href: string): void;
  /** Opens `href` in place of the current entry, as a redirect does. */
  replace(href: string): void;
}

interface RouterState {
  readonly history: RouterHistory;
  readonly routes: RouteTable;
  readonly location: RouterLocation;
  readonly match: RouteMatch;
  readonly navigator: Navigator;
}

const RouterContext = createContext<RouterState | null>(null);

/** The router, or null outside one; `Link` works without it. */
export function useRouterState(): RouterState | null {
  return useContext(RouterContext);
}

function useRequiredState(caller: string): RouterState {
  const state = useContext(RouterContext);
  if (state === null) throw new Error(`${caller} needs a <Router> above it.`);
  return state;
}

/**
 * Where a navigation leads: within the app when a route claims the address,
 * otherwise a page load, since another app (Cockpit from the web client, the
 * web client from Cockpit) or another site is a document of its own.
 * Navigating to the address already shown changes nothing.
 */
function navigate(history: RouterHistory, routes: RouteTable, href: string, replace: boolean): void {
  const target = resolveAddress(href, history.location, history.origin);
  if (!target.sameOrigin || !routes.claims(target.pathname)) {
    history.leave(target.href, replace);
    return;
  }
  if (sameAddress(target, history.location)) return;
  const address = { pathname: target.pathname, search: target.search, hash: target.hash };
  if (replace) history.replace(address);
  else history.push(address);
}

/**
 * Gives a page whose element held the focus a new starting point: when the
 * focus went down with the old page, the new page's first heading takes it,
 * so a keyboard or screen-reader user starts reading there instead of at the
 * top of the document. Focus that survived the change is left where it is.
 */
function focusNewPage(document: Document): void {
  const active = document.activeElement;
  if (active !== null && active !== document.body) return;
  const heading = document.querySelector("h1");
  if (!(heading instanceof HTMLElement)) return;
  if (!heading.hasAttribute("tabindex")) heading.setAttribute("tabindex", "-1");
  heading.focus({ preventScroll: true });
}

interface View {
  readonly location: RouterLocation;
  readonly kind: ChangeKind | "load";
}

export interface RouterProps {
  /** The app's addresses. Without one, nothing is claimed and every link loads a page. */
  readonly routes?: RouteTable;
  /** The browser's history unless given, as a test gives a memory history. */
  readonly history?: RouterHistory;
  readonly children?: ReactNode;
}

/**
 * Follows the address and gives it to the hooks, `Link` and `Outlet` below.
 *
 * A change of address renders in a transition, so a page whose code is still
 * loading keeps the old one on screen instead of a fallback. With the
 * browser's history, a change of path scrolls the window to the top, Back
 * and Forward return it to where that entry was left, and a push or a
 * traversal hands the focus to the new page (see `focusNewPage`). A change
 * of query or fragment alone touches neither.
 */
export function Router({ routes = NO_ROUTES, history, children }: RouterProps) {
  const [source] = useState(() => history ?? browserHistory());
  const [view, setView] = useState<View>(() => ({ location: source.location, kind: "load" }));
  const shown = useRef(view.location);
  const positions = useRef(new Map<string, readonly [number, number]>());

  useLayoutEffect(() => {
    const update = (kind: ChangeKind) => {
      if (source.native) positions.current.set(shown.current.key, [window.scrollX, window.scrollY]);
      const location = source.location;
      startTransition(() => setView({ location, kind }));
    };
    const stop = source.listen(update);
    // A child's layout effect may have navigated before this one subscribed.
    if (source.location !== shown.current) update("replace");
    return stop;
  }, [source]);

  useLayoutEffect(() => {
    const previous = shown.current;
    shown.current = view.location;
    if (!source.native || view.kind === "load" || previous.pathname === view.location.pathname) return;
    const [x, y] = (view.kind === "pop" ? positions.current.get(view.location.key) : undefined) ?? [0, 0];
    window.scrollTo(x, y);
    if (view.kind !== "replace") focusNewPage(window.document);
  }, [source, view]);

  const navigator = useMemo<Navigator>(
    () => ({
      push: (href) => navigate(source, routes, href, false),
      replace: (href) => navigate(source, routes, href, true),
    }),
    [source, routes],
  );

  const state = useMemo<RouterState>(
    () => ({
      history: source,
      routes,
      location: view.location,
      match: routes.match(view.location.pathname),
      navigator,
    }),
    [source, routes, view.location, navigator],
  );

  return <RouterContext.Provider value={state}>{children}</RouterContext.Provider>;
}

/**
 * The page the table gives the current address. A page whose code is loaded
 * on demand shows `fallback` the first time; later navigations keep the old
 * page until the new one is ready. A new path mounts the page afresh; a new
 * query renders the same page again.
 */
export function Outlet({ fallback = null }: { fallback?: ReactNode }) {
  const { match, location } = useRequiredState("Outlet");
  const Page = match.page;
  return (
    <Suspense fallback={fallback}>
      <Page key={location.pathname} />
    </Suspense>
  );
}

/** `push` and `replace`, stable for the life of the router. */
export function useRouter(): Navigator {
  return useRequiredState("useRouter").navigator;
}

/** The current path, canonical: `/login`, never `/login/`. */
export function usePathname(): string {
  return useRequiredState("usePathname").location.pathname;
}

/** The query of the current address, which is read and never written. */
export type ReadonlySearchParams = Omit<URLSearchParams, "append" | "delete" | "set" | "sort">;

export function useSearchParams(): ReadonlySearchParams {
  const { search } = useRequiredState("useSearchParams").location;
  return useMemo(() => new URLSearchParams(search), [search]);
}

/**
 * The parameters of the route that matched, by the names in its path,
 * decoded. A parameter whose escapes are malformed is null, so a page can
 * tell an address nobody could have meant from a value it does not know.
 */
export function useParams<Name extends string = string>(): Readonly<Record<Name, string | null>> {
  return useRequiredState("useParams").match.params as Params & Readonly<Record<Name, string | null>>;
}
