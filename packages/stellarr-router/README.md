# `@stellarr/router`

Client-side router shared by the Stellarr web client (`web/src/stellarr/`) and Cockpit (`web/src/cockpit/`). Provides declarative route matching, `<Link>` navigation, location hooks, scroll and focus restoration, and an in-memory history driver for tests.

The package is private and consumed directly as TypeScript source (`exports` points to `src/index.ts`; Vite and Vitest compile it as part of the importing application). React is its only runtime dependency.

## Usage

Each application defines a route table in `routes.tsx` and renders `<Outlet />` inside `<Router>`:

```tsx
import { defineRoutes, Outlet, Router } from "@stellarr/router";

export const routes = defineRoutes({
  routes: [
    { path: "/", page: HomePage },
    { path: "/invite/:code", page: lazy(() => import("./pages/InvitePage")) },
  ],
  notFound: NotFoundPage,
});

createRoot(element).render(
  <Router routes={routes}>
    <Providers>
      <Outlet />
    </Providers>
  </Router>,
);
```

| Export | Description |
| --- | --- |
| `defineRoutes({ routes, notFound })` | Validates and compiles a route table. Route patterns must start with `/`, omit trailing slashes, and may include `:param` single-segment parameters. Routes match in declaration order and fall back to `notFound`. Throws at startup on duplicate or unreachable patterns. |
| `Router` | Top-level provider that tracks location state (`routes` table and optional custom `history` instance). |
| `Outlet` | Renders the matched route's `page` component wrapped in `<Suspense fallback={...}>`. |
| `Link` | Renders an `<a href>` element that performs client-side navigation for in-app routes. |
| `useRouter()` | Returns `{ push(href), replace(href) }` for programmatic navigation. |
| `usePathname()` | Returns the canonical current pathname. |
| `useSearchParams()` | Returns a read-only `URLSearchParams` instance for the current query string. |
| `useParams()` | Returns decoded route parameters (`Record<string, string \| null>`) for the active route. |
| `canonicalPath(path)` | Collapses consecutive slashes and strips trailing slashes (except `/`). |

### Testing with `@stellarr/router/testing`

`@stellarr/router/testing` exports `createMemoryHistory(initialHref)`, which records every navigation in `history.navigations` and supports `back()` and `forward()`:

```tsx
const history = createMemoryHistory("/invite/abc123");
render(
  <Router routes={routes} history={history}>
    <InvitePage />
  </Router>,
);
expect(history.navigations).toEqual([{ kind: "replace", href: "/register" }]);
```

## Routing behavior

- **Link interception**: `<Link>` renders a standard `<a href>` and intercepts only unmodified primary-button clicks (`button === 0`, no modifier keys, `target` unset or `_self`, no `download` attribute, non-hash-only change) whose target URL is same-origin and matches a declared route in the current app's route table. Unclaimed paths (including `notFound`) fall through to full browser navigation, so a link from the web client to `/cockpit` or from Cockpit to `/` loads the other application's HTML document. Custom `onClick` handlers run first and cancel navigation if they call `event.preventDefault()` (enabling composition with `<Button asChild>` from `@stellarr/ui`).
- **Programmatic navigation**: `push(href)` appends a history entry; `replace(href)` replaces the active entry. Navigating to a URL outside the app's route table or origin triggers a full document load (`location.assign` or `location.replace`). Navigating to the exact current URL is a no-op.
- **Path canonicalization**: Consecutive slashes are collapsed and trailing slashes are stripped (except `/`), matching Nginx's normalization. Initial non-canonical URLs (`/cockpit/`, `//cockpit`) are normalized in place via `history.replaceState` on mount, and protocol-relative strings starting with `//` are never passed raw to the History API.
- **Parameter decoding**: `useParams()` URL-decodes each `:param` segment. If a segment contains malformed percent-encoding, the route still matches and the parameter value is `null` so the page component can render an invalid-link state. Static segments are also compared in decoded form.
- **Query string updates**: Changing only the query string re-renders the current page component in place while preserving component state; changing the pathname mounts the new route fresh.
- **Code splitting and transitions**: Lazy route components show `<Outlet fallback={...} />` on initial load and use React transitions on subsequent navigations so the current view stays visible until the incoming chunk resolves.
- **Scroll restoration**: With browser history, pathname pushes scroll the window to `(0, 0)`, while `popstate` (Back/Forward) restores the saved scroll offset after the destination route renders. Query-only or hash-only updates preserve the current scroll position.
- **Accessible focus management**: After a `push` or `popstate` navigation to a new pathname, if the previously focused element unmounted with the old page, focus moves to the new page's first `<h1>` (temporarily given `tabindex="-1"` with `preventScroll: true`). Focus on persistent shell elements (such as sidebar navigation links) is preserved, and `replace` redirects never steal focus.

## Verification

```sh
npm run typecheck -w @stellarr/router
npm run test -w @stellarr/router
```
