# @stellarr/router

The router of the web client (`web/src/stellarr/`) and Cockpit
(`web/src/cockpit/`): a table of addresses, links, hooks that read the address,
and a history in memory for tests. The package is private.

It is consumed as TypeScript source: `exports` points at `src/index.ts` and
there is no build step. Vite and Vitest compile it as part of the importing
app. It depends on nothing but React.

## Use

Each app has one table, in its `routes.tsx`, and renders the page for the
address with `Outlet` inside `Router`:

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

| Export | What it is |
| --- | --- |
| `defineRoutes({ routes, notFound })` | The table. A `path` starts with `/`, has no trailing slash, and may name `:param` segments, each matching one segment. The first route that matches answers; any other address gets `notFound`. It throws on a path it would never match and on two routes for the same addresses. |
| `Router` | Follows the address. `routes` is the table, `history` the browser's unless a test gives one. |
| `Outlet` | The page for the address, in a `Suspense` with an optional `fallback`. |
| `Link` | An `<a href>` that opens an address of the app without a page load. |
| `useRouter()` | `push(href)` and `replace(href)`. |
| `usePathname()` | The current path. |
| `useSearchParams()` | The query, to read. |
| `useParams()` | The matched route's parameters. |
| `canonicalPath(path)` | The path in the form the router uses. |

`@stellarr/router/testing` exports `createMemoryHistory(href)`, a history that
starts at `href` and lists every navigation in `navigations`, with `back()` and
`forward()`. A test renders the page under it and reads where it went:

```tsx
const history = createMemoryHistory("/invite/abc123");
render(
  <Router routes={routes} history={history}>
    <InvitePage />
  </Router>,
);
expect(history.navigations).toEqual([{ kind: "replace", href: "/register" }]);
```

## Behaviour

- **What a link takes over.** `Link` renders a real `<a href>` and takes over a
  plain primary click on an address its app's table claims. A modified click,
  another mouse button, `target` other than `_self`, `download`, a link that
  changes only the fragment, another site, and an address no route claims are
  the browser's. The not-found page claims nothing, so the web client's link to
  `/cockpit` and Cockpit's link to `/` load the other app's document. The
  caller's `onClick` runs first, and a prevented default stops the link; that
  is how it works as the child of `<Button asChild>` from `@stellarr/ui`.
  Without a `Router` above it, it is a plain link.
- **Navigations.** `push` adds a history entry and `replace` rewrites the
  current one. An address the table does not claim, or another origin, is
  loaded as a new document, replacing the entry for `replace`. Navigating to
  the address already shown changes nothing. Back and Forward render the
  entry's page again.
- **Slashes.** Paths are canonical with every run of slashes merged into one,
  as nginx merges them before it picks the document, and without a trailing
  slash, except the root. The address the page was loaded at is rewritten in
  place (`/cockpit/` and `//cockpit` become `/cockpit`), and links and
  navigations are resolved to the canonical path. An address that starts with
  two slashes is never written back as it is, which the browser would read as
  another host and refuse.
- **Parameters.** `useParams` gives each value decoded. A value whose escapes
  are malformed is `null`, and the route still matches, so a page can say the
  address is unreadable. Fixed segments are compared decoded too.
- **The query.** `useSearchParams` reads the current one. A change of query
  renders the same page again, keeping its state; a change of path mounts the
  new page afresh.
- **Code loaded on demand.** A page made with `lazy` shows the `Outlet`'s
  `fallback` when the document is opened on it. A navigation renders in a
  transition, so later the old page stays on screen until the new one is
  ready.
- **Scroll.** With the browser's history, a change of path scrolls the window
  to the top, and Back and Forward return it to where that entry was left. The
  router takes scroll restoration over from the browser, which would restore a
  position before the page it belongs to is drawn again. A change of query or
  fragment alone leaves the window where it is.
- **Focus.** After a push, Back or Forward to another path, when the element
  that had the focus went down with the old page, the new page's first `<h1>`
  takes it (made focusable with `tabindex="-1"`, without scrolling), so reading
  starts at the new page. Focus on an element that outlived the change, such as
  a link in a sidebar, stays there, and a `replace` (a redirect) never moves it.

A memory history leaves scroll and focus alone, and records a page load in
`navigations` instead of following it.

## Verification

```sh
npm run typecheck -w @stellarr/router
npm run test -w @stellarr/router
```
