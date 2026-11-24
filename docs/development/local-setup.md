# Local setup

`docker-compose.dev.yml` runs each service from a bind mount so edits to TypeScript, CSS, or Rust reload without rebuilding the production image.

## 1. Build WebAssembly recipes once

Three browser WebAssembly binaries (`speex`, `rnnoise`, and `bergamot`) are compiled from pinned C/C++ sources in `artifacts/recipes/`. Build them once into `artifacts/recipes/*/out/` before starting `web`:

```bash
docker buildx bake recipes
```

Cached runs take seconds; a cold first run downloads `wasi-sdk` and Bergamot's Emscripten archive (on Apple silicon, `bergamot` compiles once under `linux/amd64` emulation so its output matches Firefox Remote Settings). `web/` scripts run `artifacts/assemble.mjs` automatically to copy binaries into `web/public/` and verify SHA-256 hashes against `artifacts/manifest.json`. See [`artifacts/recipes/README.md`](../../artifacts/recipes/README.md).

## 2. Start the development stack

```bash
docker compose -f docker-compose.dev.yml up -d
```

The first start installs npm workspace dependencies and compiles the `api` and `media` crates into named Docker volumes.

| Service | Default port | Process |
| --- | --- | --- |
| `web` | `http://localhost:3000` | Vite dev server for the web client (`/`) and Cockpit (`/cockpit`), proxying `/api`, `/assets`, `/health`, `/hubs`, and `/.well-known/stellarr/client` to `api:5000` |
| `api` | `http://localhost:5000` | `stellarr-api` under `watchexec` and `cargo run -p stellarr-api` |
| `media` | `49100` UDP and TCP (advertised as `127.0.0.1:49100`), loopback `5200/tcp` control | `stellarr-media` under `watchexec` and `cargo run` (`49100` avoids clashing with production `49000`) |
| `mailpit` | `http://127.0.0.1:8025` (`1025/tcp` SMTP) | Captures outgoing verification, 2FA, reset, and notification emails |
| `coturn` | `3478` and `49160-49200/udp` on the host network | Optional TURN development profile |

Two browsers on this machine use the advertised loopback media candidate directly. The dev profile `STELLARR_TURN_ENABLED=true docker compose -f docker-compose.dev.yml --profile turn up -d` starts coturn for relay testing. To test a relayed call, advertise an SFU IPv4 address reachable by coturn and the browsers. The default loopback candidate works for local browser calls, while coturn refuses loopback peers.

> [!TIP]
> On macOS, port `5000` is often used by AirPlay Receiver. Override host ports in your shell or `.env` with `STELLARR_DEV_WEB_PORT`, `STELLARR_DEV_API_PORT`, `STELLARR_DEV_MEDIA_PORT`, `STELLARR_DEV_MEDIA_CONTROL_PORT`, or `STELLARR_DEV_MAILPIT_PORT` (see [Configuration: Development stack](../install/configuration.md#development-stack-docker-composedevyml)).

To start on an empty SQLite volume without touching `stellarr-dev_stellarr_dev_data`, add `docker-compose.dev.fresh.yml`:

```bash
docker compose -f docker-compose.dev.yml -f docker-compose.dev.fresh.yml up -d
```

Read the setup code with `docker logs stellarr-dev-api-1 2>&1 | grep "Setup code"`, then open `http://localhost:3000/setup`.

## 3. Seed sample data

Populate the dev database with sample accounts (`maya`, `theo`, `priya`, `jonas`, `kenji`, `ola`, password `dev-seed-2026`), custom roles, channels, messages, attachments, DMs, and presence while keeping your owner account intact:

```bash
./scripts/seed-dev.sh
```

Pass `--reset` to recreate seeded records, or `--reset --bootstrap-owner` on an empty database to create a starter `admin` owner account. See [`scripts/README.md`](../../scripts/README.md).

## 4. Run natively without Docker

Requires Node.js `22+` / `26`, Rust `stable`, and Python `3.9+` (for seed scripts):

```bash
npm ci
docker buildx bake recipes

# Terminal 1: stellarr-media (0.0.0.0:49000 and 127.0.0.1:5200)
cd media && cargo run

# Terminal 2: stellarr-api
cd api && STELLARR_ENVIRONMENT=Development STELLARR_DATA_ROOT=./dev-data cargo run -p stellarr-api

# Terminal 3: Vite dev server (/ and /cockpit)
npm run dev -w stellarr-web
```

`web/.env.example` lists optional Vite proxy settings such as `STELLARR_API_ORIGIN` (`http://127.0.0.1:5000` default).
