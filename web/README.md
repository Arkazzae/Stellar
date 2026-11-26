# Stellarr web workspace (`stellarr-web`)

Multi-page React, TypeScript, and Vite workspace in `web/` that builds both browser applications served by the Stellarr container:

- **Web client** (`index.html` → `src/stellarr/`): text channels, direct messages, voice and video calls, end-to-end encryption, offline Bergamot translation, and the theme studio. Serves `/` and every non-Cockpit route.
- **Stellarr Cockpit** (`cockpit/index.html` → [`src/cockpit/`](src/cockpit/README.md)): instance operator panel served at `/cockpit` and `/cockpit/*`.

Both applications compile into `web/dist/` (`gzip -9` pre-compressed in the container image and served directly by Nginx without a Node.js runtime). An ESLint rule in `eslint.config.mjs` forbids imports between `src/stellarr/` and `src/cockpit/`; shared primitives live in [`@stellarr/ui`](../packages/stellarr-ui/README.md), [`@stellarr/icons`](../packages/stellarr-icons/README.md), and [`@stellarr/router`](../packages/stellarr-router/README.md).

## Quick start

From the repository root:

```bash
npm ci
npm run dev -w stellarr-web    # http://localhost:3000 (/) and http://localhost:3000/cockpit
```

`predev`, `prebuild`, and `pretest` run [`../artifacts/assemble.mjs`](../artifacts/README.md) automatically to verify SHA-256 digests and copy WebAssembly binaries and translation models into `web/public/`. By default, Vite proxies `/api`, `/assets`, `/health`, `/hubs`, and `/.well-known/stellarr/client` to `STELLARR_API_ORIGIN` (`http://127.0.0.1:5000`; see `.env.example`).

## Build and runtime invariants

- **Document routing and pre-paint scripts (`documents.ts`)**: Injects inline `LOCALE_SCRIPT` and `THEME_SCRIPT` into `index.html` before first paint so language, palette, scheme, contrast, text scale, and density apply without a flash. Also injects `<link rel="modulepreload">` scripts for only the active language catalog (`en.ts` or `pl.ts`, never both) and serves `/stellarr-instance.js` in dev/preview. `documents.test.ts` checks route and slash-merging parity against [`infra/nginx.conf`](../infra/README.md).
- **Up-front bundle budget (`bundleReport.ts`)**: `npm run build` measures the static import closure of `index.html` (the bytes the sign-in screen waits for) and fails if total gzipped JavaScript and CSS exceed `UP_FRONT_BUDGET_GZIP` (`259,000` bytes). Signed-in screens, realtime/voice modules, custom fonts, the BetterDiscord converter, and language catalogs load via dynamic `import()`.
- **AudioWorklet bundles (`worklets.ts`)**: Compiles `detector-worklet.js`, `speex-worklet.js`, and `rnnoise-worklet.js` as standalone ES module workers written to stable `/audio/*` paths (served with `Cache-Control: no-cache` alongside `/audio/speex.wasm`, `/audio/rnnoise.wasm`, and `/audio/gtcrn.wasm`). Bundled app chunks live under `static/` (`immutable`, 1 year) so `/assets/` stays reserved for API file downloads.
- **Release tracking and Service Worker (`release.ts`, `public/stellarr-worker.js`)**: Caches the current release shell keyed by `/release.json`, handles Web Push notifications, and reloads open tabs cleanly after a server upgrade.

## Directory layout

| Path | Contents |
| --- | --- |
| `src/stellarr/pages/` | Top-level routes (`/`, `/login`, `/register`, `/setup`, `/invite/:code`, `/link-account`, `/desktop-login`, `/reset-password`, `/confirm-email`) |
| `src/stellarr/containers/` | Feature views (channel transcript, composer, thread panel, voice stage and dock, settings modals, server admin, E2EE setup) |
| `src/stellarr/components/` | App-specific UI building blocks composed over `@stellarr/ui` |
| `src/stellarr/state/` | Session, bootstrap, chat reconciliation, presence, voice, E2EE, and [`ThemeProvider`](../docs/themes.md) state |
| `src/stellarr/ports/`, `adapters/` | Typed API/realtime port interfaces and HTTP/WebSocket adapters (`signedInCode.ts` splits signed-in adapters out of the entry chunk) |
| `src/stellarr/design/`, `styles/` | Palettes, custom color derivation, `.stellarr-theme.json` import/export, pre-paint `themeScript.ts`, and `--ui-*` token mappings |
| `src/stellarr/i18n/` | Locale resolution, pre-paint `localeScript.ts`, and `en.ts` / `pl.ts` message catalogs |
| `src/stellarr/lib/` | Subsystems: `voice/` (WebRTC SFU client, Web Audio capture/playback DSP, SFrame E2EE worker), `e2ee/` (X25519/AES-GCM DM keys and epochs), `realtime/` (`/hubs/app` client), `translation/` (Bergamot WASM worker), `betterDiscord/` (isolated `.theme.css` converter), `messages/` (Markdown/mention parser), and `deviceStore/` (IndexedDB transcript cache) |
| `src/stellarr/test/` | Shared test fixtures and in-memory port fakes |
| [`src/cockpit/`](src/cockpit/README.md) | Standalone operator panel SPA |
| [`bench/`](bench/README.md) | Real-browser CDP benchmarks (10,000-message transcript, channel rail drag/live updates) and [voice DSP characterization records](bench/voice/README.md) |
| `scripts/` | Asset and verification scripts (`sync-theme-fonts.mjs`, `export-identity-art.mjs`, `export-mascot.mjs`, `derive-language-profiles.mjs`, `verify-bergamot-runtime.mjs`) |

## Verification

From the repository root:

```bash
npm run lint:web
npm run typecheck:web
npm run test:web
npm run build:web
npm run test:translation-runtime -w stellarr-web   # verifies Bergamot WASM in headless Chromium
```
