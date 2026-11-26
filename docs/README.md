# Documentation

Stellarr is a self-hosted text, voice, and video platform for small groups, packaged as a single `amd64` / `arm64` Docker container backed by one persistent directory (`/data/stellarr`).

> [!TIP]
> **New to Stellarr?** Follow the [Quick start](install/quick-start.md) to boot a local instance and verify a two-user voice call in under five minutes. Before opening access over the internet, check [Behind NAT](install/requirements.md#behind-nat) for media port forwarding and TLS setup.

## Self-hosting

Step-by-step installation, networking, TLS, and upgrade guides for operators.

| Guide | Scope |
| --- | --- |
| [Split deployment](https://github.com/Arkazzae/Stellarr/tree/main/deploy/split) | Separate web, API, media, and optional TURN containers |
| [Quick start](install/quick-start.md) | Automated `install.sh` setup, manual Docker Compose install, `/setup` wizard, and first voice call |
| [Requirements](install/requirements.md) | Host sizing (~35 MiB idle RAM), storage quotas, published ports, LAN/NAT rules, and outbound network policy |
| [Configuration](install/configuration.md) | Precedence rules, all 61 `STELLARR_*` variables, raw `Section__Key` overrides, logging, and `docker run` |
| [Reverse proxy and TLS](install/reverse-proxy-and-tls.md) | Custom CA certificates, Caddy and Nginx configs, `STELLARR_TRUSTED_PROXIES`, and direct WebRTC routing |
| [Unraid](install/unraid.md) | User template installation, port-pair rules, Nginx Proxy Manager bridge setup, and CLI commands |
| [Upgrading](install/upgrading.md) | Pre-upgrade volume snapshots, release updates, SQLite schema compatibility, and rollbacks |

## Administration

Day-to-day server management, security policies, backups, and external integrations.

| Guide | Scope |
| --- | --- |
| [Servers, channels, and features](admin/servers-and-features.md) | Channels, threads, polls, 4K60 streams, soundboards, activities, PWA features, roles/permissions, private channels, 1:1 E2EE, and [Discord imports](admin/servers-and-features.md#import-from-discord) |
| [Stellarr Cockpit](admin/cockpit.md) | `/cockpit` operator panel: users, servers, activities, runtime metrics, logs, backups, integrations, and settings |
| [Operator CLI](admin/cli.md) | `stellarr` commands for health checks, DB settings, sealed secrets, session revocation, MFA recovery, and staged restores |
| [Account security](admin/account-security.md) | Passkeys, TOTP, email codes, OIDC linking and passwordless sign-ups, instance MFA policies, and recovery flows |
| [Automatic backups](admin/backups.md) | Scheduled full `.zip` archives, archive structure, staged startup restores, and post-restore token cleanup |
| [Cloudflare R2 offsite backups](admin/cloudflare-r2.md) | Direct S3 multipart streaming, the Backup Relay Worker, SHA-256 verification, and retention rules |
| [Activity notifications](admin/notifications.md) | Zero-metadata email and Web Push alerts for `@mentions` and DMs, VAPID key generation, and retry policies |

## Activity SDK

Build multiplayer games, watch parties, and solo mini-apps with `@stellarr/sdk`—no backend server required.

| Guide | Scope |
| --- | --- |
| [Getting started](sdk/getting-started.md) | 2-minute zero-build activity, Vite/TypeScript setup, local multi-tab simulator, theme/locale/device sync, and live voice previews |
| [Rooms, state, and saves](sdk/rooms-and-state.md) | Choosing data channels, conflict-free `state.set(fn)`, host commands, spectator seats, secret player hands, `now()` timers, and `progress` saves |
| [Media, manifest, and publishing](sdk/media-and-publishing.md) | **Game audio** volume, consented `microphoneLevel` & `cameraTile` overlays, `stellarr.json`, CSP `connects`, and Cockpit releases |
| [API and CLI reference](sdk/api-reference.md) | Full `DevSession<S>` TypeScript API, observables, `stellarr` CLI flags, allowed asset extensions, error codes, and runtime limits |

## Reference

System architecture, wire protocols, media server internals, theming, and localization.

| Document | Scope |
| --- | --- |
| [Architecture](architecture.md) | Process supervision, Nginx routing, `/data/stellarr` layout, `master.key` sealing, E2EE cryptography, and authorization invariants |
| [Themes and appearance](themes.md) | Six-axis theme model, built-in presets, CSS palette editor, wallpapers, and BetterDiscord conversion |
| [RTC and ports](rtc-and-ports.md) | Host and loopback ports, optional TURN relay sizing, NAT/firewall rules, and WebRTC call diagnostics |
| [Voice and RTC protocol](rtc-protocol.md) | `stellarr.v1` signaling, `voice::enforce` seat eviction, soundboard sync, Opus profiles, and Web Audio DSP chains |
| [Stellarr Media](stellarr-media.md) | Rust `str0m` SFU, glare-free SDP negotiation, receive subscriptions, GoogCC downlink adaptation, SFrame E2EE, and control API |
| [Localization](localization.md) | `STELLARR_LANGUAGE` (`en`, `pl` and `de`), one JSON file per language, offline Bergamot WASM translation, and adding locales |

## Development

Contributor setup, repository architecture, test suites, and CI contract guards.

| Guide | Scope |
| --- | --- |
| [Local setup](development/local-setup.md) | Live-reload Docker development stack (`infra/compose/dev.yml`), sample database seeding, and native execution |
| [Codebase and testing](development/codebase-and-testing.md) | Monorepo layout, module registration rules, bundle budgets, test commands, and automated contract checks |

## Component guides

Internal architecture and package-level documentation inside each workspace directory:

- **Backend & runtime:** [API workspace (`api/`)](../api/README.md) · [Media SFU (`media/`)](../media/README.md) ([smoke suite](../media/smoke/README.md)) · [Init supervisor (`init/`)](../init/README.md) · [Host installer (`init/installer/`)](../init/installer/README.md) · [Container scripts (`infra/`)](../infra/README.md)
- **Web & UI packages:** [Web workspace (`web/`)](../web/README.md) · [Cockpit SPA (`web/src/cockpit/`)](../web/src/cockpit/README.md) · [`@stellarr/ui`](../packages/stellarr-ui/README.md) · [`@stellarr/icons`](../packages/stellarr-icons/README.md) · [`@stellarr/router`](../packages/stellarr-router/README.md) · [Web & voice benchmarks (`web/bench/`)](../web/bench/README.md) ([voice DSP](../web/bench/voice/README.md))
- **Activities & artifacts:** [Activity SDK (`sdk/`)](../sdk/README.md) ([Activity contract](../sdk/CONTRACT.md)) · [Browser runtime artifacts (`artifacts/`)](../artifacts/README.md) ([WASM recipes](../artifacts/recipes/README.md))
- **Tooling & deploy:** [Dev seed scripts (`scripts/`)](../scripts/README.md) · [Platform templates (`deploy/`)](../deploy/README.md) ([R2 Relay Worker](../deploy/cloudflare/backup-relay/README.md)) · [Project website (`website/`)](../website/README.md)
