# Stellarr Cockpit

Stellarr Cockpit is the instance operator panel, at `/cockpit` on the instance's own origin. Its code lives here, in `web/src/cockpit/`, beside the web client's; the same Vite build (`web/`) writes both as static files. It provides:

- **Overview**: headline metrics with 7-day trends and week-over-week change, 30-day activity across six series, API uptime, the last hour of API and database availability, request and connection charts, and service health status (auto-refreshed every 30 s).
- **Users**: account management, active sessions, built-in roles, MFA recovery, and OIDC identity unlinking.
- **Servers**: list of servers on the instance with ownership transfer and deletion.
- **Runtime**: 60-minute process telemetry (requests, errors, CPU, memory, connections, database latency) and per-minute availability across all container services.
- **Logs**: paged, searchable, and filterable 10,000-line in-memory log stream with TSV export.
- **Backups**: background database snapshots or full `.zip` archives (database, object store, data-protection keys, and instance files) with manifest verification, scheduling policy, and retention pruning.
- **Integrations**: Resend mail delivery, GIPHY GIF search, the sign-in providers (whether each may create accounts for people with an invite), and the optional Cloudflare R2 backup destination, with key checks.
- **Settings**: instance name, time zone, retention windows, SMTP configuration, instance MFA policy, and panel language.

> [!NOTE]
> Secrets saved in Cockpit (the SMTP password, Resend API key, and GIPHY API key) are encrypted with the instance's data-protection keys, take precedence over environment variables, and are never returned to the browser.

## How it is built and served

Cockpit is an app of its own inside the web workspace, with no build or server of its own:

- Its document is [`web/cockpit/index.html`](../../cockpit/index.html), which loads `main.tsx`. `npm run build` builds it with the web client's into `web/dist/`, at `dist/cockpit/index.html`, and `npm run dev` serves both.
- The addresses are in `routes.tsx`, on [`@stellarr/router`](../../../packages/stellarr-router/README.md): one page per area (`/cockpit`, `/cockpit/users`, `/cockpit/servers`, `/cockpit/runtime`, `/cockpit/logs`, `/cockpit/backups`, `/cockpit/integrations`, `/cockpit/settings`), each from `pages/`, and the panel's not-found page for any other address under `/cockpit`. Every address under `/cockpit` gets this document; the router shows the page.
- `main.tsx` renders the router, `CockpitApp` (`app.tsx`) and the page, and imports the stylesheet (`index.css`). `CockpitApp` holds the session, the language and the gate around the page.
- The document is separate from the web client's (`web/index.html`), so the two apps share no stylesheet, language or providers. Going from one to the other is a full page load: a link to `/` is not one of Cockpit's addresses, so the browser loads it. Tailwind scans only this folder for Cockpit's stylesheet, and the web client's leaves it out.
- Code here imports its own modules by relative path and nothing of the web client's; a lint rule in `web/eslint.config.mjs` enforces that.

Development uses the web client's dev server:

```bash
# from the repository root
npm install
docker buildx bake recipes   # once, and again when artifacts/recipes/ changes
npm run dev -w stellarr-web
```

The bake builds the browser binaries that [`artifacts/recipes/`](../../../artifacts/recipes/README.md) makes from source; the dev server, the tests and the build check every binary against `artifacts/manifest.json` before they start. The dev server listens on `http://localhost:3000`, serves Cockpit at `http://localhost:3000/cockpit` with hot reload, and proxies `/api`, `/assets`, `/health`, `/hubs` and `/.well-known/stellarr/client` to `STELLARR_API_ORIGIN` (`http://127.0.0.1:5000` by default). `docker-compose.dev.yml` runs the same server in its `web` service.

The UI parts (buttons, fields, cards, tables, tabs, dialogs, sheets, menus, tooltips) come from the shared [`@stellarr/ui`](../../../packages/stellarr-ui/README.md) package and the icons from [`@stellarr/icons`](../../../packages/stellarr-icons/README.md), which `components/icons/icon.tsx` sizes in rem; Cockpit composes its pages from them with Tailwind utilities. `index.css` imports the package's stylesheet and maps Cockpit's palette onto its `--ui-*` tokens.

In the production container image Nginx serves the built files, on the same origin as the web client. All `/api/cockpit/*` endpoints require an authenticated session with the instance owner role.

## Localization

Cockpit supports English (`en`, default) and Polish (`pl`). The active locale is the one chosen under **Settings**, which this browser keeps in `localStorage` (`stellarr.cockpit.locale`), else `en`. It follows neither the browser's languages nor the web client's language.

Catalogs live in `i18n/en.ts` and `i18n/pl.ts` and define identical key sets. Components format messages via `t("some.key", { name: value })`.

## Verification

```bash
# from the repository root
npm run lint:web
npm run typecheck:web
npm run test:web
npm run build:web
```

Each covers both apps. The tests run as two Vitest projects, `web` and `cockpit` (`web/vitest.config.ts`), each with its own setup; `npx vitest run --project cockpit` in `web/` runs Cockpit's alone. The tests of components that read the address or link within the panel render them under `Router` with a memory history from `@stellarr/router/testing`, which lists where each link went.
