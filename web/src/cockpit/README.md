# Stellarr Cockpit (`web/src/cockpit/`)

Standalone instance operator SPA served at `/cockpit` on the instance origin and compiled alongside the chat client by the `web/` Vite build. See the [Cockpit administration guide](../../../docs/admin/cockpit.md) for MFA policies and operational workflows.

## Pages and routes (`routes.tsx`)

| Page | Route | Scope |
| --- | --- | --- |
| **Overview** | `/cockpit` | Activity trends, API/SQLite availability bars, request/hub charts, and `stellarr-init` service health (`30 s` poll). |
| **Users** | `/cockpit/users` | Account directory, session revocation, role management, MFA recovery, and OIDC identity unlinking. |
| **Servers** | `/cockpit/servers` | Instance-wide server list, ownership transfer, and deletion. |
| **Runtime** | `/cockpit/runtime` | `/proc` telemetry (CPU, RSS/mapped memory, threads, hub sockets, voice rooms, SQLite latency) and per-minute availability. |
| **Logs** | `/cockpit/logs` | Searchable, filterable 10,000-line in-memory log ring buffer with TSV export. |
| **Activities** | `/cockpit/activities` | Install static multiplayer activities from GitHub releases, review permissions/external hosts, update/toggle/uninstall, install and update dates, the server's update checks (every 12 h, or on demand), and pair dev previews. |
| **Backups** | `/cockpit/backups` | Online `.db` snapshots and full `.zip` archives, SHA-256 verification, downloads, and schedules. |
| **Integrations** | `/cockpit/integrations` | Resend mail, GIPHY key validation, Web Push VAPID status, OIDC invite sign-up toggles, and Cloudflare R2 (`s3` or `worker`). |
| **Settings** | `/cockpit/settings` | Instance name, time zone, file storage quota, retention windows, SMTP transport, MFA policy, desktop version gates, and UI language. |

> [!NOTE]
> Secrets stored in Cockpit are sealed with AES-256-GCM under `/data/stellarr/keys/master.key`, override `.env` immediately, and are never returned to browsers.

## Architecture and isolation

- **Separate HTML entry:** [`web/cockpit/index.html`](../../cockpit/index.html) mounts `src/cockpit/main.tsx` (`web/dist/cockpit/index.html`). Nginx serves `cockpit/index.html` for all `/cockpit/*` URLs.
- **Zero shared app state:** `web/eslint.config.mjs` forbids imports from `src/stellarr/`. Cross-links between `/cockpit` and `/` perform full document navigations on [`@stellarr/router`](../../../packages/stellarr-router/README.md).
- **Shared design system:** Consumes [`@stellarr/ui`](../../../packages/stellarr-ui/README.md) and [`@stellarr/icons`](../../../packages/stellarr-icons/README.md), mapping Cockpit's palette and fluid `--ui-unit: calc(1rem / 16)` onto `--ui-*` tokens in `src/cockpit/index.css`.

## Localization

Cockpit supports English (`en`, default), Polish (`pl`) and German (`de`), persisted in `localStorage` (`stellarr.cockpit.locale`) independently of `STELLARR_LANGUAGE` and the chat client. The web build takes Cockpit strings from the `cockpit` section of each language pack and checks them against English. [Language packs](../../../docs/localization.md)

## Development and testing

```bash
# Start Vite dev server (http://localhost:3000/cockpit -> proxies /api to 127.0.0.1:5000)
npm run dev -w stellarr-web

# Full lint, typecheck, unit tests, and production bundle verification
npm run lint:web && npm run typecheck:web && npm run test:web && npm run build:web

# Run Cockpit unit tests in isolation
npx -w stellarr-web vitest run --project cockpit
```
