# Documentation

Stellarr is a self-hosted text, voice, and video chat for small groups, packaged as a single Docker container.

> [!TIP]
> To try Stellarr locally in a few minutes, start with the [Quick start](install/quick-start.md). Before exposing an instance to friends over the internet, review [Requirements](install/requirements.md#behind-nat) for `PUBLIC_IP` and UDP port forwarding.

## Self-hosting

- [Quick start](install/quick-start.md): start a local instance with Docker Compose, create the owner account, and test voice.
- [Requirements](install/requirements.md): host resources, published ports, NAT forwarding, and outbound network traffic.
- [Configuration](install/configuration.md): environment variables for production, development, and non-Compose deployments.
- [Reverse proxy and TLS](install/reverse-proxy-and-tls.md): custom TLS certificates, reverse proxy configuration, trusted proxies, and WebRTC ports.
- [Unraid](install/unraid.md): install and update from the Unraid template.
- [Upgrading](install/upgrading.md): volume backups, image upgrades, schema updates, and rollbacks.

## Administration

- [Operator CLI](admin/cli.md): inspect health, edit settings, rotate secrets, manage sessions, and stage backups with `stellarr`.
- [Account security](admin/account-security.md): second-factor methods, passkeys, OIDC linking, instance MFA policy, and account recovery.
- [Automatic backups](admin/backups.md): scheduled backups, archive verification, local retention, and full or partial restore procedures.
- [Cloudflare R2 offsite backups](admin/cloudflare-r2.md): direct S3 uploads, the Backup Relay Worker, retention, and security notes.
- [Activity notifications](admin/notifications.md): opt-in email and Web Push delivery, VAPID key setup, privacy guarantees, and retry behavior.

## Reference

- [Architecture](architecture.md): container services, Nginx routing, persistent storage, security, and authorization rules.
- [RTC and ports](rtc-and-ports.md): published ports, TURN relay sizing, NAT/firewall rules, and call diagnostics.
- [Voice and RTC protocol](rtc-protocol.md): SignalR signaling, client state model, audio/video pipeline, screen sharing, and camera profiles.
- [Stellarr Media](stellarr-media.md): Rust SFU media server, SDP negotiation, receive subscriptions, downlink adaptation, and loopback control API.
- [Localization](localization.md): interface locales, server message catalogs, client-side Bergamot chat translation, and adding a language.

## Component guides

- [Stellarr Cockpit](../web/src/cockpit/README.md): operator panel architecture, localization, and checks.
- [Stellarr Media crate](../media/README.md): building and testing the `stellarr-media` binary and the [Chromium smoke suite](../media/smoke/README.md).
- [Stellarr Init](../init/README.md): PID 1 process manager, readiness probes, control socket, and PID-namespace tests.
- [Platform templates](../deploy/README.md): packaging rules for non-Compose templates and the Cloudflare backup relay.
- [Branding](../branding/README.md): logo, mascot, color palette, typography, and trademark policy.
