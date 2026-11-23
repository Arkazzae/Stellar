# Documentation

Stellarr is a self-hosted text, voice, and video chat platform for small groups, packaged as a single Docker container backed by `/data/stellarr`.

> [!TIP]
> To start a local instance in a few minutes, follow the [Quick start](install/quick-start.md). Before exposing your server to external devices, review [Requirements](install/requirements.md#behind-nat) for `STELLARR_PUBLIC_IP` and UDP port forwarding.

## Self-hosting

- [Quick start](install/quick-start.md): install with `install.sh` or Docker Compose, complete the `/setup` wizard, invite a second user, and test voice.
- [Requirements](install/requirements.md): host resources, published ports, NAT forwarding, TLS requirements, and outbound network behavior.
- [Configuration](install/configuration.md): environment variables (`STELLARR_*`), raw `.NET` configuration keys, development stack settings, and `docker run` flags.
- [Reverse proxy and TLS](install/reverse-proxy-and-tls.md): mounting custom TLS certificates, external reverse proxy setup, `STELLARR_TRUSTED_PROXIES`, and direct WebRTC UDP ports.
- [Unraid](install/unraid.md): install, configure port mappings, and update using the Unraid container template.
- [Upgrading](install/upgrading.md): stopping and archiving the data volume, pulling new releases, schema updates, and rollbacks.

## Administration

- [Operator CLI](admin/cli.md): inspect health, edit database settings, manage encrypted secrets, revoke sessions, recover accounts, and stage restores with `stellarr`.
- [Account security](admin/account-security.md): TOTP, email codes, passkeys, OIDC linking and passwordless provider sign-up, instance MFA policy, and account recovery.
- [Automatic backups](admin/backups.md): scheduled full backups, archive structure, retention, and automated or manual restore procedures.
- [Cloudflare R2 offsite backups](admin/cloudflare-r2.md): direct S3 multipart uploads, the Cloudflare Backup Relay Worker, retention rules, and security considerations.
- [Activity notifications](admin/notifications.md): opt-in email and Web Push alerts for mentions and direct messages, VAPID key generation, and retry rules.

## Reference

- [Architecture](architecture.md): container process supervision, Nginx routing, storage layout, end-to-end encryption, account security, and server authorization.
- [RTC and ports](rtc-and-ports.md): published host ports, internal loopback ports, TURN relay sizing, NAT/firewall rules, and call diagnostics.
- [Voice and RTC protocol](rtc-protocol.md): SignalR signaling, `VoiceState` model, Opus and Web Audio DSP pipeline, screen sharing, and camera profiles.
- [Stellarr Media](stellarr-media.md): Rust WebRTC SFU, SDP negotiation, receive subscriptions, per-receiver downlink adaptation, E2EE call frame format, and control API.
- [Localization](localization.md): interface locale resolution, message catalogs, offline Bergamot WASM chat translation, and adding new languages.

## Component guides

- [Stellarr Cockpit](../web/src/cockpit/README.md): operator SPA architecture, routing, localization, and test commands.
- [Stellarr Media crate](../media/README.md): building and testing `stellarr-media` and running the [headless Chromium smoke suite](../media/smoke/README.md).
- [Stellarr Init](../init/README.md): PID 1 process supervisor, readiness probes, control socket protocol, and PID-namespace test suite.
- [Artifact recipes](../artifacts/recipes/README.md): reproducible Docker BuildKit stages for compiling browser WebAssembly binaries (`speex`, `rnnoise`, `bergamot`) from source.
- [Shared frontend packages](../packages/stellarr-ui/README.md): [`@stellarr/ui`](../packages/stellarr-ui/README.md) components and CSS token system, [`@stellarr/icons`](../packages/stellarr-icons/README.md) dual-variant SVG icon set, and [`@stellarr/router`](../packages/stellarr-router/README.md).
- [Web benchmarks](../web/bench/README.md): real-browser benchmarks for the message transcript, channel rail drag and live-call updates, and the [voice DSP characterization records](../web/bench/voice/README.md).
- [Development scripts](../scripts/README.md): seeding the development SQLite database with sample users, channels, messages, media, and presence.
- [Platform templates](../deploy/README.md): packaging rules for non-Compose templates and the [Cloudflare backup relay Worker](../deploy/cloudflare/backup-relay/README.md).
- [Branding](../branding/README.md): logo, mascot, [UI icon SVGs](../branding/icons/README.md), [README tour film](../branding/film/README.md), color tokens, typography, and trademark policy.
- [Project website](../website/README.md): building and deploying the `stellarr.online` landing page and installer endpoint.
