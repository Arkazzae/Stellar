# Stellarr Web Workspace (`stellarr-web`)

Multi-page React 19, TypeScript, and Vite workspace in `web/` compiling both static browser applications served by container Nginx:

- **Web client** (`index.html` → `src/stellarr/`): channels, DMs, WebRTC voice/video, 1:1 E2EE, activities, installed PWA features, offline Bergamot translation, and the theme studio.
- **Stellarr Cockpit** (`cockpit/index.html` → [`src/cockpit/`](src/cockpit/README.md)): owner-only operator SPA served at `/cockpit` and `/cockpit/*`.

Both compile into `web/dist/` (`gzip -9` pre-compressed in the production image; zero Node.js runtime in production). `eslint.config.mjs` forbids imports between `src/stellarr/` and `src/cockpit/`; shared primitives live in [`@stellarr/ui`](../packages/stellarr-ui/README.md), [`@stellarr/icons`](../packages/stellarr-icons/README.md), and [`@stellarr/router`](../packages/stellarr-router/README.md).

## Quick start

```bash
npm ci
npm run dev -w stellarr-web   # http://localhost:3000 (/) and http://localhost:3000/cockpit
```

`predev`, `prebuild`, and `pretest` run [`../artifacts/assemble.mjs`](../artifacts/README.md) to verify SHA-256 digests and stage WASM audio worklets and Bergamot models into `web/public/`. Vite proxies `/api`, `/assets`, `/health`, `/hubs`, and `/.well-known/stellarr/client` to `STELLARR_API_ORIGIN` (`http://127.0.0.1:5000` default; see `.env.example`).

## Build and runtime contracts

| Contract | Source | Guarantee |
| --- | --- | --- |
| **Pre-paint & route parity** | `documents.ts`, `documents.test.ts` | Inlines `LOCALE_SCRIPT` and `THEME_SCRIPT` in `<head>`, `modulepreload`s only the active locale chunk (`en` or `pl`), and enforces route parity with [`infra/nginx.conf`](../infra/README.md). |
| **Entry bundle budget** | `bundleReport.ts` | Fails `npm run build` if the static JS+CSS closure of `index.html` exceeds `259,000` gzipped bytes (`UP_FRONT_BUDGET_GZIP`). Signed-in views, voice/RTC, BetterDiscord conversion, and locale catalogs load via dynamic `import()`. |
| **AudioWorklets & asset paths** | `worklets.ts` | Emits standalone `/audio/{detector,speex,rnnoise}-worklet.js` (`no-cache`) while content-hashed app chunks live under `static/` (`immutable`, 1 year) so `/assets/` stays reserved for API file downloads. |
| **Service Worker & PWA** | `release.ts`, `public/stellarr-worker.js` | Tracks `/release.json` for zero-stale tab reloads, handles Web Push notifications, syncs unread app badges, and receives OS-shared files (`/pwa/share`). |

## Directory layout

| Path | Contents |
| --- | --- |
| `src/stellarr/pages/` | Public & entry routes (`/`, `/login`, `/register`, `/setup`, `/forgot-password`, `/reset-password`, `/confirm-email`, `/link-account`, `/invite/:code`) |
| `src/stellarr/containers/` | Signed-in feature views (transcript, composer, threads, voice stage/dock, activities, PWA `ShareTarget`, settings, server admin, E2EE) |
| `src/stellarr/components/` | Domain UI components composed over `@stellarr/ui` |
| `src/stellarr/state/` | Session, bootstrap, chat journal reconciliation, presence, voice, E2EE, and [`ThemeProvider`](../docs/themes.md) |
| `src/stellarr/ports/`, `adapters/` | Typed port interfaces and HTTP/`stellarr.v1` WebSocket adapters (`signedInCode.ts` code-split boundary) |
| `src/stellarr/design/`, `styles/` | Palettes, custom color math, `.stellarr-theme.json`, pre-paint `themeScript.ts`, and `--ui-*` token bindings |
| `src/stellarr/i18n/` | Locale resolution, `localeScript.ts`, and `en.ts` / `pl.ts` catalogs |
| `src/stellarr/lib/` | `voice/` (SFU client, Web Audio DSP, SFrame E2EE), `e2ee/` (X25519/AES-GCM DMs), `realtime/`, `chatTranslation/` (Bergamot WASM), `betterDiscord/`, `platform/` (PWA install, badge, file share), `messages/`, and `deviceStore/` (IndexedDB cache) |
| [`src/cockpit/`](src/cockpit/README.md) | Standalone operator panel SPA |
| [`bench/`](bench/README.md) | Headless Chrome CDP benchmarks (10k-message transcript, 150-channel rail drag) and [voice DSP characterization records](bench/voice/README.md) |
| `scripts/` | Build/asset utilities (`derive-language-profiles.mjs`, `verify-bergamot-runtime.mjs`) |

## Verification

```bash
npm run lint:web
npm run typecheck:web
npm run test:web
npm run build:web
npm run test:translation-runtime -w stellarr-web   # offline Bergamot WASM check in headless Chromium
```
