# Container infrastructure (`infra/`)

Boot scripts, service configuration templates, Compose overlays, and shell contract tests packaged with the production Stellarr container.

## Files

| File | Container path | Purpose |
| --- | --- | --- |
| `entrypoint.sh` | `/usr/local/bin/stellarr-entrypoint` | Container `ENTRYPOINT`: validates `STELLARR_*`, applies staged backup restores, initializes `/data/stellarr`, prepares TLS & secrets, renders configs, and `exec`s [`stellarr-init`](../init/README.md) as PID 1. |
| `settings.sh` | `/usr/local/lib/stellarr/settings.sh` | Shared `STELLARR_*` parser, validator, defaults table, and `public_url` resolver sourced by `entrypoint.sh` and `stellarr-cli.sh`. |
| `stellarr-cli.sh` | `/usr/local/bin/stellarr` | `docker exec` wrapper around `/opt/stellarr/cli/stellarr` sourcing `settings.sh` (warns on malformed `.env` values without blocking offline CLI repair). |
| `tls-maintenance.sh` | `/usr/local/bin/stellarr-tls-maintenance` | Manages `/data/stellarr/tls/{fullchain,privkey}.pem` (`prepare` on boot + 12-hour `watch` loop under `stellarr-init`). |
| `nginx.conf` | `/etc/nginx/nginx.conf` (rendered) | Terminates TLS (`:443`), redirects `:80` unless a trusted proxy sends `X-Forwarded-Proto: https`, serves `/opt/stellarr/web`, and proxies API/WebSockets to `127.0.0.1:5000`. |
| `turnserver.conf.template` | `/etc/turnserver.conf` (rendered) | `coturn` STUN/TURN (`:3478`), optional TURNS (`:5349`), UDP relay pool (`49160–49200`), and HMAC `use-auth-secret` template. |
| `compose/` | *(Host Compose overlays)* | `build.yml` (local source build), `turn.yml` (optional TURN port publishing), and `dev.yml` (live-reload development stack). |

## Boot sequence (`entrypoint.sh`)

1. **Validate settings:** Parses `STELLARR_*` via `settings.sh` (empty values use built-in defaults; invalid values print an error and exit `1`).
2. **Staged restore check:** Aborts if `/data/stellarr/restore/broken` exists; runs `stellarr backup apply --yes` when `restore/staged.zip` is present (exit `4` moves a refused archive aside and boots existing data).
3. **Volume & secrets:** Creates `/data/stellarr/{db,files,incoming,keys,secrets,tls,releases,backups,restore,logs}`, writes `0600` `media-control-secret` (and `turn-shared-secret` when TURN is enabled), and resolves `STELLARR_PUBLIC_IP=auto`.
4. **TLS preparation:** Generates/renews a 365-day RSA-3072 self-signed cert (`.stellarr-self-signed`) when absent, SAN-mismatched, or `< 30` days from expiry; operator-mounted CA certs are never overwritten.
5. **Render & handoff:** Writes `/run/stellarr/stellarr-instance.js` (`STELLARR_LANGUAGE`), renders `/etc/nginx/nginx.conf` (and `/etc/turnserver.conf`), exports resolved env vars, and `exec`s `stellarr-init run`.

## Nginx cache and routing rules (`nginx.conf`)

- `/static/`, `/fonts/` (`200`/`206`/`304`): `public, max-age=31536000, immutable` (`gzip_static on`).
- `/stellarr-instance.js` (`/run/stellarr/stellarr-instance.js`): `public, max-age=3600, must-revalidate`.
- All other static files under `/opt/stellarr/web/` (`index.html`, `cockpit/index.html`, `release.json`, `stellarr-worker.js`, `/audio/*`, `/codecs/*`, `/bergamot-models/*`, `/identity/*`, `/mascot/*`, `/pwa/*`, `/sounds/*`): `no-cache`. Missing paths inside file directories or ending with `.ext` return plain `404` (verified by `web/documents.test.ts`); `/pwa/share` redirects `303 /?share=error` when hit without an active Service Worker.
- `/api/`, `/assets/`, `/health`, `/.well-known/stellarr/`, and `/hubs/` (`Upgrade`/`Connection`, `128m` body ceiling): proxied to `http://127.0.0.1:5000`.

## Contract and smoke tests (`infra/tests/`)

| Script | Scope | Command |
| --- | --- | --- |
| `setting-names.test.sh` | Verifies all 61 `STELLARR_*` names match across `variables.rs`, `infra/*.sh`, `docker-compose.yml`, `.env.example`, and `docs/install/configuration.md`. | `bash infra/tests/setting-names.test.sh` |
| `install.test.sh` | Tests `stellarr-installer` (`--features test-seams`) and `install.sh` across fresh install, update, checksum, and failure flows against stubbed host tools. | `bash infra/tests/install.test.sh` |
| `tls-maintenance.test.sh` | Tests self-signed generation, SAN updates, renewal thresholds, and preservation of mounted CA certs (Bash + OpenSSL 3+). | `bash infra/tests/tls-maintenance.test.sh` |
| `entrypoint.test.sh` | Tests `entrypoint.sh` exports, rendered configs, restore handling, and `stellarr-cli.sh` inside a throwaway base container. | Run as root in base image (see `.github/workflows/ci.yml`) |
| `smoke.test.sh` | End-to-end container boot, `/setup`, WebSocket chat, file upload/download, full `.zip` backup, staged restore + restart, and `stellarr status`. | `STELLARR_SMOKE_IMAGE=stellarr:ci bash infra/tests/smoke.test.sh` |
