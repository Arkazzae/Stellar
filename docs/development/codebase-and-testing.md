# Codebase and testing

## Repository layout

| Directory | Stack | Contents |
| --- | --- | --- |
| [`api/`](../../api/README.md) | Rust (20 crates) | `stellarr-api` server (`crates/server`), `stellarr-hub` (`crates/hub`), `stellarr` CLI (`crates/cli`), and workspace crates (`archive`, `deflate`, `discord`, `embeds`, `mail`, `mentions`, `messages`, `oidc`, `outbound`, `s3`, `smtp`, `storage`, `timezone`, `unicode`, `webauthn`, `webpush`, `websocket`). |
| [`web/`](../../web/README.md) | TypeScript, React, Vite | Multi-page static build for the web client (`web/src/stellarr/`) and [Cockpit](../../web/src/cockpit/README.md) (`web/src/cockpit/`). |
| `packages/` | TypeScript, React | [`@stellarr/ui`](../../packages/stellarr-ui/README.md) components and `--ui-*` tokens, [`@stellarr/icons`](../../packages/stellarr-icons/README.md), and [`@stellarr/router`](../../packages/stellarr-router/README.md). |
| [`media/`](../../media/README.md) | Rust (`str0m`, `axum`) | `stellarr-media` WebRTC SFU and the browser smoke suite in [`media/smoke/`](../../media/smoke/README.md). |
| [`init/`](../../init/README.md) | Rust (`std`, `libc`) | `stellarr-init` PID 1 supervisor, readiness probes, and Unix control socket. |
| [`init/installer/`](../../init/installer/README.md) | Rust (`std`, `libc`, `ureq`) | `stellarr-installer`, the host installer that the root `install.sh` downloads from a release and runs. |
| [`infra/`](../../infra/README.md) | Shell, Nginx, coturn | `entrypoint.sh`, `settings.sh`, `stellarr-cli.sh`, `tls-maintenance.sh`, `nginx.conf`, and `infra/tests/`. |
| [`artifacts/`](../../artifacts/README.md) | BuildKit, `wasi-sdk`, Emscripten | `manifest.json`, `assemble.mjs`, source [`recipes/`](../../artifacts/recipes/README.md) (`speex`, `rnnoise`, `bergamot`), `files/`, and `NOTICES`. |
| `docs/` | VitePress, Mermaid | Documentation site (`navigation.ts` builds the nav and sidebar from `docs/README.md`). |
| [`website/`](../../website/README.md) | Vanilla JS, CSS, Vite | `stellarr.online` landing page and `/install.sh` build (standalone npm project). |
| [`branding/`](../../branding/README.md) | SVG, WebP, CDP | Logos, mascot assets, avatars, backgrounds, icons, and the [`branding/film/`](../../branding/film/README.md) pipeline. |

## Codebase rules

- **Registration lists in `api/`**: Add new endpoint groups, workers, hub methods, and API test modules by appending one line to `groups.list.rs`, `workers.list.rs`, `realtime/methods.list.rs`, or `tests/api/modules.list.rs` (`merge=union` in `.gitattributes`). See [`api/README.md`](../../api/README.md#adding-a-group-of-endpoints).
- **App isolation in `web/`**: `web/eslint.config.mjs` forbids imports between `web/src/stellarr/` and `web/src/cockpit/`. Shared code lives in `packages/*`.
- **Up-front bundle budget (`259,000` gzipped bytes)**: `web/bundleReport.ts` fails `npm run build:web` when the static closure of `web/index.html` passes `UP_FRONT_BUDGET_GZIP`. Keep signed-in screens, realtime/voice modules, custom fonts, the BetterDiscord converter, and language catalogs behind dynamic `import()`.
- **Bilingual catalogs**: `api/crates/messages/` (`en` and `pl` exhaustive `match` arms) and `web/` (`pl.ts` and `en.ts` key parity) fail compilation if a key is missing in either language.

## Test commands

### Web client, Cockpit, and packages

From the repository root (after `npm ci` and `docker buildx bake recipes`):

```bash
npm run lint:web
npm run typecheck:ui && npm run typecheck:icons && npm run typecheck:router && npm run typecheck:web
npm run test:ui && npm run test:icons && npm run test:router && npm run test:web
npm run icons:build -w @stellarr/icons -- --check
npm run tokens:check -w @stellarr/ui
npm run build:web
```

Headless Chromium checks:

```bash
npm run test:translation-runtime -w stellarr-web
npm run bench:transcript -w stellarr-web -- --messages 1000,10000 --check
npm run bench:voice-playback -w stellarr-web -- --check
```

### Rust API and CLI (`api/`)

Each test in `api/crates/server/tests/api/` boots an isolated `stellarr-api` process on a free port and temporary directory:

```bash
cd api
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

### Stellarr Media (`media/`) and Stellarr Init (`init/`)

```bash
cd media
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cd smoke && npm ci && npm run smoke
```

`stellarr-init` uses Linux `signalfd`, `PR_SET_CHILD_SUBREAPER`, `SO_PEERCRED`, and `unshare` PID namespaces; run its tests on Linux or in a privileged container:

```bash
docker run --rm --privileged -v "$PWD/init":/src -w /src rust:1-slim-bookworm \
  sh -c 'rustup component add rustfmt clippy && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test'
```

### Docs, website, and container contracts

```bash
cd docs && npm test && npm run build
cd website && npm run build
bash infra/tests/setting-names.test.sh
bash infra/tests/entrypoint.test.sh
(cd init/installer && cargo test --features test-seams)
bash infra/tests/install.test.sh

docker build -t stellarr:ci .
STELLARR_SMOKE_IMAGE=stellarr:ci bash infra/tests/smoke.test.sh
STELLARR_SMOKE_IMAGE=stellarr:ci STELLARR_SMOKE_LIMITS='--memory 512m --cpus 1' bash infra/tests/smoke.test.sh
```

## CI contract guards

1. `infra/tests/setting-names.test.sh` checks that all 61 `STELLARR_*` names match across `variables.rs`, `infra/*.sh`, `docker-compose.yml`, `.env.example`, and [`docs/install/configuration.md`](../install/configuration.md).
2. `npm run tokens:check -w @stellarr/ui` checks that every `--ui-*` CSS token read by `@stellarr/ui` is defined by both apps and listed in [`packages/stellarr-ui/README.md`](../../packages/stellarr-ui/README.md).
3. `npm run icons:build -w @stellarr/icons -- --check` checks `paths.ts` and `branding/icons/{broken,linear}/*.svg` against `glyphs.ts`.
4. `web/documents.test.ts` and `artifacts.test.ts` check route parity with `infra/nginx.conf`, locale parity with `infra/entrypoint.sh`, and recipe parity with `Dockerfile` and `docker-bake.hcl`.
5. `cd docs && npm test && npm run build` checks navigation, link rewriting, `<h1>` titles, and internal links.
