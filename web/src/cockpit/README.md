# Stellarr Cockpit

Stellarr Cockpit is the instance operator panel served at `/cockpit` on the instance's origin. Its source code lives in `web/src/cockpit/` alongside the web client (`web/src/stellarr/`), and both applications are compiled into static assets by the same `web/` Vite build. Cockpit provides:

- **Overview**: Instance activity trends, API uptime, API and SQLite availability windows, request/connection charts, and container service health (polled every 30 s).
- **Users**: Account directory, active session inspection and revocation, built-in role management, MFA recovery, and OIDC identity unlinking.
- **Servers**: Instance-wide server list with ownership transfer and deletion.
- **Runtime**: Process telemetry (requests, errors, CPU, memory, active connections, SQLite latency) and per-minute availability across all supervised container services.
- **Logs**: Paged, searchable, and filterable in-memory log ring buffer with TSV export.
- **Backups**: Background SQLite snapshots and full `.zip` backup archives (database, object storage, data-protection keys, and instance files) with SHA-256 manifest verification, automated schedules, retention pruning, and staged restores.
- **Integrations**: Resend email delivery, GIPHY GIF search, per-provider OIDC account creation controls, and Cloudflare R2 offsite backups (Direct S3 or Backup Relay Worker) with live credential tests.
- **Settings**: Instance name, time zone, telemetry/audit retention windows, SMTP server configuration, instance MFA policy, and Cockpit UI language.

> [!NOTE]
> Secrets saved in Cockpit (SMTP password, Resend API key, GIPHY API key, and R2 credentials) are encrypted at rest with the instance's ASP.NET Core Data Protection keys, override environment variables, and are never returned to the browser.

## Build and runtime architecture

Cockpit is a standalone single-page application within the `web/` workspace:

- **Entry HTML**: [`web/cockpit/index.html`](../../cockpit/index.html) loads `src/cockpit/main.tsx`. `npm run build` outputs the bundle to `web/dist/cockpit/index.html`, and `npm run dev` serves both the web client (`/`) and Cockpit (`/cockpit`).
- **Routing**: `routes.tsx` defines the route table on [`@stellarr/router`](../../../packages/stellarr-router/README.md) (`/cockpit`, `/cockpit/users`, `/cockpit/servers`, `/cockpit/runtime`, `/cockpit/logs`, `/cockpit/backups`, `/cockpit/integrations`, `/cockpit/settings`, plus a Cockpit-scoped 404 view). Nginx serves `cockpit/index.html` for all `/cockpit/*` paths.
- **Strict isolation from the web client**: Cockpit has its own HTML document, Tailwind stylesheet (`src/cockpit/index.css`), localization state, and session gate (`CockpitApp` in `app.tsx`). Navigating between `/cockpit` and `/` triggers a full document load. An ESLint boundary rule in `web/eslint.config.mjs` forbids `src/cockpit/` from importing modules under `src/stellarr/`.
- **Design system**: UI components are imported from [`@stellarr/ui`](../../../packages/stellarr-ui/README.md) and icons from [`@stellarr/icons`](../../../packages/stellarr-icons/README.md) (wrapped by `components/icons/icon.tsx` for `rem` scaling). `src/cockpit/index.css` maps Cockpit's palette and fluid `--ui-unit: calc(1rem / 16)` scale onto the `--ui-*` token tiers.

### Local development

```bash
# From the repository root
npm install
docker buildx bake recipes   # run once, and whenever artifacts/recipes/ changes
npm run dev -w stellarr-web
```

The Vite dev server listens on `http://localhost:3000`, serves Cockpit at `http://localhost:3000/cockpit` with HMR, and proxies `/api`, `/assets`, `/health`, `/hubs`, and `/.well-known/stellarr/client` to `STELLARR_API_ORIGIN` (default `http://127.0.0.1:5000`). All `/api/cockpit/*` endpoints require an authenticated session with the instance `Owner` role.

## Localization

Cockpit supports English (`en`, default) and Polish (`pl`). The active locale is selected under **Cockpit → Settings** and persisted in `localStorage` under `stellarr.cockpit.locale`, independent of the browser's `Accept-Language` header and the web client's locale preference.

Translation catalogs in `i18n/en.ts` and `i18n/pl.ts` define identical key sets and are formatted with `t("some.key", { name: value })`.

## Verification

From the repository root:

```bash
npm run lint:web
npm run typecheck:web
npm run test:web
npm run build:web
```

Each script checks both the web client and Cockpit. Vitest runs two isolated projects defined in `web/vitest.config.ts` (`web` and `cockpit`); run Cockpit's unit tests alone from `web/` with:

```bash
npx vitest run --project cockpit
```
