# Stellarr Cockpit

Stellarr Cockpit is the instance operator panel, at `/cockpit` on the instance's own origin. Its code lives here, in `web/src/cockpit/`, beside the web client's; the same Next.js app (`web/`) renders both. It provides:

- **Overview**: headline metrics with 7-day trends and week-over-week change, 30-day activity across six series, API uptime, the last hour of API and database availability, request and connection charts, and service health status (auto-refreshed every 30 s).
- **Users**: account management, active sessions, and built-in roles.
- **Servers**: list of servers on the instance with ownership transfer and deletion.
- **Runtime**: 60-minute process telemetry (requests, errors, CPU, memory, connections, database latency) and per-minute availability across all container services.
- **Logs**: paged, searchable, and filterable 10,000-line in-memory log stream with TSV export.
- **Backups**: background database snapshots or full `.zip` archives (database, object store, data-protection keys, and instance files) with manifest verification and retention pruning.
- **Integrations**: Resend mail delivery, GIPHY GIF search, and the optional Cloudflare R2 backup destination, with key checks.
- **Settings**: instance name, time zone, retention windows, SMTP server configuration, and panel language.

> [!NOTE]
> Secrets saved in Cockpit (the SMTP password, Resend API key, and GIPHY API key) are encrypted with the instance's data-protection keys, take precedence over environment variables, and are never returned to the browser.

## How it is built and served

Cockpit is an app of its own inside the web workspace, with no build or server of its own:

- The routes are in [`web/src/app/(cockpit)/`](../app/(cockpit)/): a root layout of Cockpit's own, one page per area (`/cockpit`, `/cockpit/users`, `/cockpit/servers`, `/cockpit/runtime`, `/cockpit/logs`, `/cockpit/backups`, `/cockpit/integrations`, `/cockpit/settings`) and a catch-all that shows the panel's not-found page for any other address under `/cockpit`.
- Each route imports its page from here (`@/cockpit/pages/<name>`), and the layout imports `CockpitApp` (`app.tsx`) and the stylesheet (`index.css`). `next build` builds the web client and Cockpit together and `next dev` serves both.
- The layout is separate from the web client's root layout in `web/src/app/(web)/`, so the two apps share no stylesheet, language or providers. Going from one to the other is a full page load. Tailwind scans only this folder for Cockpit's stylesheet, and the web client's leaves it out.
- Code here imports its own modules by relative path and nothing of the web client's; a lint rule in `web/eslint.config.mjs` enforces that.
- `CockpitApp` holds the session, the language and the gate around the routed page, and renders in the browser only: the language is kept in the browser (see below), so the server sends the document without the panel.

Development uses the web client's dev server:

```bash
# from the repository root
npm install
npm run dev -w stellarr-web
```

It listens on `http://localhost:3000`, serves Cockpit at `http://localhost:3000/cockpit` with hot reload, and proxies `/api`, `/assets`, `/health` and `/hubs` to `STELLARR_API_ORIGIN` (`http://127.0.0.1:5000` by default). `docker-compose.dev.yml` runs the same server in its `web` service.

The UI parts (buttons, fields, cards, tables, tabs, dialogs, sheets, menus, tooltips) come from the shared [`@stellarr/ui`](../../../packages/stellarr-ui/README.md) package and the icons from [`@stellarr/icons`](../../../packages/stellarr-icons/README.md), which `components/icons/icon.tsx` sizes in rem; Cockpit composes its pages from them with Tailwind utilities. `index.css` imports the package's stylesheet and maps Cockpit's palette onto its `--ui-*` tokens.

In the production container image the same Next.js server renders Cockpit, behind Nginx, on the same origin as the web client. All `/api/cockpit/*` endpoints require an authenticated session with the instance owner role.

## Localization

Cockpit supports English (`en`, default) and Polish (`pl`). The active locale is the one chosen under **Settings**, which this browser keeps in `localStorage` (`stellarr.cockpit.locale`), else `en`. It follows neither the browser's languages nor the web client's `stellarr_locale` cookie.

Catalogs live in `i18n/en.ts` and `i18n/pl.ts` and define identical key sets. Components format messages via `t("some.key", { name: value })`.

## Verification

```bash
# from the repository root
npm run lint:web
npm run typecheck:web
npm run test:web
npm run build:web
```

Each covers both apps. The tests run as two Vitest projects, `web` and `cockpit` (`web/vitest.config.ts`), each with its own setup; `npx vitest run --project cockpit` in `web/` runs Cockpit's alone. The tests of components that read the address or link within the panel mock `next/navigation` and `next/link` with the stand-in router in `test/navigation.tsx`.
