# Documentation

Stellarr is self-hosted text, voice, and video chat for small groups of friends, packaged as one Docker container backed by `/data/stellarr`.

> [!TIP]
> To start a local instance in two minutes, follow the [Quick start](install/quick-start.md). Before inviting friends from outside your machine, review [Behind NAT](install/requirements.md#behind-nat) for reachable media addresses, firewall rules, and router forwarding.

## Self-hosting

- [Quick start](install/quick-start.md): install with `install.sh` or Docker Compose, complete `/setup`, invite a second user, and test voice.
- [Requirements](install/requirements.md): host sizing (tested down to 512 MiB RAM and 1 CPU core), published ports, NAT forwarding, TLS, and outbound network rules.
- [Configuration](install/configuration.md): all 61 `STELLARR_*` variables, raw `Section__Key` settings, structured logging, dev stack settings, and `docker run`.
- [Reverse proxy and TLS](install/reverse-proxy-and-tls.md): custom TLS certificates, Caddy and Nginx reverse-proxy configs, `STELLARR_TRUSTED_PROXIES`, and direct WebRTC over UDP and TCP.
- [Unraid](install/unraid.md): install, map port pairs, run CLI checks, and update using the Unraid container template.
- [Upgrading](install/upgrading.md): stopping and archiving `/data/stellarr`, pulling new releases, SQLite schema rules, and rollbacks.

## Administration

- [Servers, channels, and features](admin/servers-and-features.md): text channels, threads, polls, soundboards, arrival clips, roles, permissions, private channels (`viewChannel`), and 1:1 E2EE.
- [Import from Discord](admin/servers-and-features.md#import-from-discord) covers bot setup, available channels and history, copying attachments, and resuming an import.
- [Stellarr Cockpit](admin/cockpit.md): `/cockpit` operator panel for accounts, servers, runtime telemetry, logs, backups, integrations, and settings.
- [Operator CLI](admin/cli.md): inspect health, edit database settings, seal secrets, revoke sessions, recover accounts, and stage restores with `stellarr`.
- [Account security](admin/account-security.md): passkeys, TOTP, email codes, OIDC linking and passwordless sign-ups, instance MFA policy, and account recovery.
- [Automatic backups](admin/backups.md): scheduled full `.zip` backups, archive layout, post-restore session/token invalidation, and restore steps.
- [Cloudflare R2 offsite backups](admin/cloudflare-r2.md): direct S3 multipart uploads, the Backup Relay Worker, SHA-256 checks, and retention rules.
- [Activity notifications](admin/notifications.md): opt-in email and Web Push alerts for `@mentions` and direct messages, VAPID keys, and retry rules.

## Reference

- [Architecture](architecture.md): container supervision, Nginx routing, `/data/stellarr` layout, `master.key` sealing, E2EE DMs and calls, and server authorization.
- [Themes and appearance](themes.md): color presets, custom palette editor, self-hosted fonts, wallpapers, `.stellarr-theme.json`, and BetterDiscord `.theme.css` conversion.
- [RTC and ports](rtc-and-ports.md): published host ports, internal loopback ports, TURN relay sizing, firewall rules, and call diagnostics.
- [Voice and RTC protocol](rtc-protocol.md): `stellarr.v1` WebSocket signaling, live `voice::enforce` seat eviction, soundboards, Opus profiles, Web Audio DSP, and 4K60 screen sharing.
- [Stellarr Media](stellarr-media.md): Rust `str0m` SFU, SDP negotiation, receive subscriptions, per-receiver downlink adaptation, SFrame E2EE frames, and control API.
- [Localization](localization.md): `STELLARR_LANGUAGE` (`en` and `pl`), message catalogs, offline Bergamot WASM chat translation, and adding new languages.

## Development

- [Local setup](development/local-setup.md): building WebAssembly recipes, running `infra/compose/dev.yml` with live reload, seeding sample data, and running natively.
- [Codebase and testing](development/codebase-and-testing.md): monorepo layout, adding endpoints and workers, test commands, and CI contract guards.

## Component guides

- [API workspace](../api/README.md): 20-crate Rust Cargo workspace (`stellarr-api`, `stellarr-cli`, `stellarr-hub`, `stellarr-discord`), HTTP pipeline, and test harness.
- [Web workspace](../web/README.md): `stellarr-web` multi-page Vite build, up-front bundle budget, AudioWorklets, and the [Cockpit SPA](../web/src/cockpit/README.md).
- [Media SFU crate](../media/README.md): building and testing `stellarr-media` and running the [browser smoke suite](../media/smoke/README.md).
- [Init supervisor](../init/README.md): Rust PID 1 supervisor, readiness probes, Unix control socket protocol, and PID-namespace tests.
- [Container infrastructure](../infra/README.md): `entrypoint.sh`, `settings.sh`, `stellarr-cli.sh`, `tls-maintenance.sh`, `nginx.conf`, optional TURN relay configuration, and shell contract tests.
- [Runtime artifacts](../artifacts/README.md): `manifest.json`, `assemble.mjs`, prebuilt models, and reproducible [WebAssembly recipes](../artifacts/recipes/README.md).
- [Shared frontend packages](../packages/stellarr-ui/README.md): [`@stellarr/ui`](../packages/stellarr-ui/README.md) components and `--ui-*` tokens, [`@stellarr/icons`](../packages/stellarr-icons/README.md), and [`@stellarr/router`](../packages/stellarr-router/README.md).
- [Web benchmarks](../web/bench/README.md): browser benchmarks for 10,000-message transcripts, channel rail drag, live-call updates, and [voice DSP records](../web/bench/voice/README.md).
- [Development scripts](../scripts/README.md): seeding the development SQLite database with sample accounts, channels, messages, media, and presence.
- [Platform templates](../deploy/README.md): packaging rules for non-Compose hosts and the [Cloudflare backup relay Worker](../deploy/cloudflare/backup-relay/README.md).
- [Branding](../branding/README.md): logo, mascot, [UI icon SVGs](../branding/icons/README.md), [README tour film](../branding/film/README.md), color tokens, typography, and trademark policy.
- [Project website](../website/README.md): building and deploying the `stellarr.online` landing page and `/install.sh` endpoint.
