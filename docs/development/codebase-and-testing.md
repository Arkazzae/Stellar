# Codebase and testing

## Monorepo layout

| Directory | Stack | Purpose |
| --- | --- | --- |
| [`api/`](../../api/README.md) | Rust (20 crates, `unsafe` forbidden) | `stellarr-api` (`crates/server`), `stellarr` CLI (`crates/cli`), `stellarr-hub` (`crates/hub`), and 17 domain/protocol crates (`archive`, `deflate`, `discord`, `embeds`, `mail`, `mentions`, `messages`, `oidc`, `outbound`, `s3`, `smtp`, `storage`, `timezone`, `unicode`, `webauthn`, `webpush`, `websocket`). |
| [`web/`](../../web/README.md) | TypeScript, React 19, Vite | Multi-page static build for the web client (`src/stellarr/`) and [Cockpit](../../web/src/cockpit/README.md) (`src/cockpit/`), plus [browser & voice benchmarks](../../web/bench/README.md). |
| `packages/` | TypeScript, React | Shared libraries: [`@stellarr/ui`](../../packages/stellarr-ui/README.md), [`@stellarr/icons`](../../packages/stellarr-icons/README.md), and [`@stellarr/router`](../../packages/stellarr-router/README.md). |
| [`media/`](../../media/README.md) | Rust (`str0m`, `axum`) | `stellarr-media` WebRTC SFU and headless browser [smoke suite](../../media/smoke/README.md). |
| [`init/`](../../init/README.md) | Rust (`std`, `libc`) | `stellarr-init` PID 1 supervisor and [`init/installer/`](../../init/installer/README.md) (`stellarr-installer` host binary). |
| [`infra/`](../../infra/README.md) | Shell, Nginx, coturn | `entrypoint.sh`, `settings.sh`, `stellarr-cli.sh`, `tls-maintenance.sh`, `nginx.conf`, Compose overlays, and shell contract tests. |
| [`artifacts/`](../../artifacts/README.md) | BuildKit, `wasi-sdk`, Emscripten | SHA-256 `manifest.json`, `assemble.mjs`, reproducible WASM [`recipes/`](../../artifacts/recipes/README.md), and `NOTICES`. |
| [`sdk/`](../../sdk/README.md) | JavaScript, Node.js CLI | `@stellarr/sdk` activity client, local simulator, bundler, and [Activity contract](../../sdk/CONTRACT.md). |
| `docs/` | VitePress, Mermaid | Documentation site (`navigation.ts` derives sidebar order from `docs/README.md`). |
| [`website/`](../../website/README.md) | Vanilla JS, CSS, Vite | `stellarr.online` landing page and `/install.sh` endpoint (standalone npm project). |

## Core engineering invariants

| Rule | Enforcement |
| --- | --- |
| **Union-merged API registries** | Register endpoint groups, workers, hub methods, and integration tests by adding one line to `groups.list.rs`, `workers.list.rs`, `realtime/methods.list.rs`, or `tests/api/modules.list.rs` (`merge=union` in `.gitattributes`; see [`api/README.md`](../../api/README.md#adding-a-group-of-endpoints)). |
| **Strict app isolation (`web/`)** | `web/eslint.config.mjs` forbids imports between `src/stellarr/` and `src/cockpit/`. Shared primitives belong in `packages/*`. |
| **`259,000`-byte gzip entry budget** | `web/bundleReport.ts` fails `npm run build:web` if `index.html`'s static import closure exceeds `UP_FRONT_BUDGET_GZIP`. Keep signed-in UI, WebRTC/voice, BetterDiscord conversion, and locale chunks behind dynamic `import()`. |
| **Compile-time bilingual parity** | `api/crates/messages` (`en`/`pl` exhaustive `match`) and `web/` (`pl.ts`/`en.ts` key parity) refuse to compile when any key is missing in either language. |

---

## Running test suites

### Web client, Cockpit, SDK, and packages

```bash
npm ci
npm run lint:web
npm run typecheck:ui && npm run typecheck:icons && npm run typecheck:router && npm run typecheck:web
npm run test:ui && npm run test:icons && npm run test:router && npm run test:web && npm run test:sdk
npm run icons:build -w @stellarr/icons -- --check
npm run tokens:check -w @stellarr/ui
npm run build:web

# Headless Chrome runtime & benchmark checks
npm run test:translation-runtime -w stellarr-web
npm run bench:transcript -w stellarr-web -- --messages 1000,10000 --check
npm run bench:voice-playback -w stellarr-web -- --check
```

### Rust API & CLI (`api/`)

Each integration test in `api/crates/server/tests/api/` spawns an isolated `stellarr-api` process on an ephemeral port and temp directory:

```bash
cd api
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

### Stellarr Media (`media/`) and Stellarr Init (`init/`)

```bash
# SFU unit, simulated-link, and headless browser smoke tests
cd media && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd smoke && npm ci && npm run smoke

# Linux PID 1 namespace & signalfd tests (run on Linux or privileged Docker)
docker run --rm --privileged -v "$PWD/init":/src -w /src rust:1-slim-bookworm \
  sh -c 'rustup component add rustfmt clippy && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test'
```

### Docs, website, installer, and container smoke suite

```bash
cd docs && npm test && npm run build
cd website && npm run build
(cd init/installer && cargo test --locked --features test-seams)
bash infra/tests/setting-names.test.sh
bash infra/tests/install.test.sh

# Full container image end-to-end smoke check (normal + 512 MiB / 1 CPU constrained)
docker build -t stellarr:ci .
STELLARR_SMOKE_IMAGE=stellarr:ci bash infra/tests/smoke.test.sh
STELLARR_SMOKE_IMAGE=stellarr:ci STELLARR_SMOKE_LIMITS='--memory 512m --cpus 1' bash infra/tests/smoke.test.sh
```

## CI contract guards

| Guard | What it verifies |
| --- | --- |
| `infra/tests/setting-names.test.sh` | Exact parity of all 61 `STELLARR_*` variables across `variables.rs`, `infra/*.sh`, `docker-compose.yml`, `.env.example`, and [`docs/install/configuration.md`](../install/configuration.md). |
| `npm run tokens:check -w @stellarr/ui` | Every `--ui-*` token read by `@stellarr/ui` is defined by both apps and documented in [`packages/stellarr-ui/README.md`](../../packages/stellarr-ui/README.md). |
| `npm run icons:build -w @stellarr/icons -- --check` | Generated `paths.ts` matches `glyphs.ts`. |
| `web/documents.test.ts` & `artifacts.test.ts` | Route parity with `infra/nginx.conf`, locale parity with `infra/entrypoint.sh`, and WASM/model SHA-256 parity with `artifacts/manifest.json`. |
| `cd docs && npm test && npm run build` | `<h1>` titles, sidebar ordering, GitHub relative-link rewriting, and internal link/anchor validity across `docs/`. |
