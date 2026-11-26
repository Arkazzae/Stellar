# Local setup

`infra/compose/dev.yml` runs every service from bind mounts with live reload so edits to TypeScript, CSS, or Rust apply without rebuilding the production container image.

## 1. Start the Docker development stack

```bash
docker compose -f infra/compose/dev.yml up -d
```

First boot installs npm workspace dependencies (running `artifacts/assemble.mjs` to stage verified WASM suppressors and Bergamot models; see [`artifacts/README.md`](../../artifacts/README.md)) and compiles `api` and `media` into named Cargo cache volumes.

| Service | Default host endpoint | Runtime process |
| --- | --- | --- |
| `web` | `http://localhost:3000` | Vite dev server (`/` and `/cockpit`), proxying `/api`, `/assets`, `/health`, `/hubs`, and `/.well-known/stellarr/client` to `api:5000` |
| `api` | `http://localhost:5000` | `stellarr-api` under `watchexec` (`cargo run -p stellarr-api`) |
| `media` | `127.0.0.1:49100` UDP + TCP (`5200/tcp` control) | `stellarr-media` under `watchexec` (`49100` avoids clashing with prod `49000`) |
| `mailpit` | `http://127.0.0.1:8025` (`1025/tcp` SMTP) | Captures outgoing verification, 2FA, password-reset, and alert emails |
| `coturn` | `3478` + `49160–49200/udp` (`--profile turn`) | Optional TURN relay (`STELLARR_TURN_ENABLED=true`; advertise a non-loopback LAN IPv4 for relayed tests) |

> [!TIP]
> On macOS, AirPlay Receiver often occupies port `5000`. Remap dev ports via `STELLARR_DEV_WEB_PORT`, `STELLARR_DEV_API_PORT`, `STELLARR_DEV_MEDIA_PORT`, `STELLARR_DEV_MEDIA_CONTROL_PORT`, or `STELLARR_DEV_MAILPIT_PORT` ([Configuration: Development stack](../install/configuration.md#development-stack)).

### Booting a clean empty database

Use a separate Compose project name to test `/setup` from scratch (stop `stellarr-dev` first if sharing ports):

```bash
docker compose -f infra/compose/dev.yml --project-name stellarr-dev-fresh up -d
docker compose -f infra/compose/dev.yml --project-name stellarr-dev-fresh logs api | grep "Setup code"
```

Then open `http://localhost:3000/setup`.

## 2. Seed sample data

Populate the dev database with sample accounts (`maya`, `theo`, `priya`, `jonas`, `kenji`, `ola` — password `dev-seed-2026`), custom roles, sections, multi-day transcripts, attachments, DMs, and presence while preserving your owner account:

```bash
./scripts/seed-dev.sh
```

Pass `--reset` to recreate seeded data cleanly, or `--reset --bootstrap-owner` on an empty database to create an `admin` owner account automatically. See [`scripts/README.md`](../../scripts/README.md).

## 3. Run natively without Docker

Requires Node.js `22+`, Rust `stable`, and Python `3.9+` (for seed scripts):

```bash
npm ci

# Terminal 1 — WebRTC SFU (0.0.0.0:49000 + 127.0.0.1:5200)
cd media && cargo run

# Terminal 2 — Rust API & WebSocket hub
cd api && STELLARR_ENVIRONMENT=Development STELLARR_DATA_ROOT=./dev-data cargo run -p stellarr-api

# Terminal 3 — Vite dev server (http://localhost:3000 and /cockpit)
npm run dev -w stellarr-web
```

Optional Vite proxy overrides (such as `STELLARR_API_ORIGIN=http://127.0.0.1:5000`) are listed in `web/.env.example`.
