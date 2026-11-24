# Container infrastructure (`infra/`)

Startup scripts, configuration templates, and container contract tests shipped inside the production Stellarr image.

## Files

| File | Installed path in image | Role |
| --- | --- | --- |
| `entrypoint.sh` | `/usr/local/bin/stellarr-entrypoint` | Container `ENTRYPOINT`: prepares `/data/stellarr`, applies any staged backup restore, generates internal secrets and self-signed TLS if needed, renders service configs, exports resolved settings, and `exec`s [`stellarr-init`](../init/README.md) as PID 1. |
| `settings.sh` | `/usr/local/lib/stellarr/settings.sh` | Shared `STELLARR_*` parser, validator, default table, and `public_url` derivation sourced by both `entrypoint.sh` and `stellarr-cli.sh`. |
| `stellarr-cli.sh` | `/usr/local/bin/stellarr` | Wrapper around `/opt/stellarr/cli/stellarr` for `docker exec`. Sources `settings.sh` so `Operations__PublicUrl` and `Desktop__CanonicalOrigin` match the running API; prints a warning without blocking the CLI if `.env` has a broken setting so offline repair still works. |
| `tls-maintenance.sh` | `/usr/local/bin/stellarr-tls-maintenance` | Manages `/data/stellarr/tls/{fullchain,privkey}.pem`. Runs once in `prepare` mode at boot and continuously in `watch` mode (every 12 hours by default) under `stellarr-init`. |
| `nginx.conf` | `/etc/nginx/nginx.conf` (rendered) | Terminates TLS (`443`), redirects plain HTTP (`80`) unless `X-Forwarded-Proto: https` is set, serves pre-compressed static web assets from `/opt/stellarr/web`, and proxies API and WebSocket traffic to `127.0.0.1:5000`. |
| `turnserver.conf.template` | `/etc/turnserver.conf` (rendered) | `coturn` configuration for STUN/TURN (`3478`), optional TURNS (`5349`), UDP relay range (`49160-49200`), and `use-auth-secret` HMAC credentials. |

## Startup sequence (`entrypoint.sh`)

1. Validates `STELLARR_*` variables via `settings.sh` (blank values fall back to image defaults; malformed values exit `1` with an explicit message).
2. Refuses to boot if `/data/stellarr/restore/broken` exists; runs `stellarr backup apply --yes` when `/data/stellarr/restore/staged.zip` is present (exit `4` marks the archive refused and boots with existing data).
3. Creates `/data/stellarr/{db,files,incoming,keys,secrets,tls,releases,backups,restore,logs}`, ensures `/data/stellarr/secrets/{turn-shared-secret,media-control-secret}` exist (`0600`), and resolves `STELLARR_PUBLIC_IP=auto` via `getent ahostsv4`.
4. Runs `tls-maintenance.sh prepare`: generates a 365-day RSA-3072 self-signed certificate (marked by `.stellarr-self-signed`) when absent, mismatched with `STELLARR_DOMAIN`, or within 30 days of expiry. Operator-mounted certificates are never overwritten (and log a warning within 7 days of expiry); when `watch` mode renews a managed certificate, it calls `stellarr-init reload nginx` and `stellarr-init restart turn`.
5. Writes `/opt/stellarr/web/stellarr-instance.js` (`STELLARR_LANGUAGE`), renders `/etc/turnserver.conf` and `/etc/nginx/nginx.conf`, exports resolved `STELLARR_*` variables for `stellarr-api` and `stellarr-media`, and hands PID 1 to `stellarr-init run`.

## Nginx caching and routing (`nginx.conf`)

- `/static/`, `/fonts/`, and `/typefaces/` on `200`/`206`/`304`: `public, max-age=31536000, immutable` (`gzip_static on`).
- `/stellarr-instance.js`: `public, max-age=3600, stale-while-revalidate=86400`.
- All other files in `/opt/stellarr/web/` (`index.html`, `cockpit/index.html`, `release.json`, `stellarr-worker.js`, `/audio/*`, `/codecs/*`, `/bergamot-models/*`, sounds, avatars, mascot art): `no-cache`. Missing paths under file directories or paths with extensions return `404` rather than falling back to an HTML shell (enforced in `web/documents.test.ts`).
- `/api/`, `/assets/`, `/health`, `/.well-known/stellarr/`, and `/hubs/` (with `Upgrade` / `Connection` WebSocket headers and `128m` max body size): proxied to `http://127.0.0.1:5000`.

## Tests (`infra/tests/`)

| Script | What it verifies | How to run |
| --- | --- | --- |
| `setting-names.test.sh` | Exact parity of all `STELLARR_*` names across `variables.rs`, `infra/*.sh`, `docker-compose.yml`, `.env.example`, and `docs/install/configuration.md` | `bash infra/tests/setting-names.test.sh` |
| `install.test.sh` | Root `install.sh` against stubbed `docker`/`curl` commands across fresh install, update, missing release, and startup failure flows | `bash infra/tests/install.test.sh` |
| `tls-maintenance.test.sh` | Self-signed generation, SAN changes, renewal windows, and non-overwriting of operator-mounted certificates (requires Bash and OpenSSL 3+) | `bash infra/tests/tls-maintenance.test.sh` |
| `entrypoint.test.sh` | `entrypoint.sh` exports, rendered configs, restore handling, and `stellarr-cli.sh` inside a throwaway container of the image base | Run as root inside the base container (see script header and `.github/workflows/ci.yml`) |
| `smoke.test.sh` | Full container boot, `/setup`, sign-in, WebSocket message delivery, upload/download, full backup, `stellarr backup restore` + restart, and `stellarr status` | `STELLARR_SMOKE_IMAGE=stellarr:ci bash infra/tests/smoke.test.sh` |
