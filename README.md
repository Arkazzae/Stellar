<h1 align="center">Stellarr</h1>

<p align="center">
  <strong>Talk on your own server.</strong><br>
  Self-hosted text, voice, video, and screen sharing for small groups — packaged in one Docker container.
</p>

<p align="center">
  <a href="#try-it-locally">Try it locally</a> ·
  <a href="#inviting-friends-over-the-internet">Going online</a> ·
  <a href="https://arkazzae.github.io/Stellarr/">Documentation</a> ·
  <a href="docs/install/unraid.md">Unraid guide</a>
</p>

<p align="center">
  <img src="branding/film/stellarr.webp" width="820" alt="A one-minute tour of Stellarr: the astronaut mascot turns into the logo, friends plan a game night in a text channel, a Polish message is translated in the browser, four people talk in a voice channel while sharing a screen, the interface switches through its six palettes, the owner checks the instance in Cockpit, and one docker compose command starts the container.">
</p>

<p align="center">
  <sub>Experimental · In active development</sub>
</p>

---

## Why Stellarr

Stellarr gives your group a private server for text chat, voice channels, and game streaming—without ads, subscriptions, or third-party cloud dependencies.

- **Familiar text chat** — Text channels, direct messages, threads, replies, reactions, polls, voice messages, custom emojis, stickers, and server soundboards.
- **End-to-end encrypted direct messages and calls** — Optional per-conversation end-to-end encryption for 1:1 messages, attachments, and private voice or video calls using X25519, AES-256-GCM, a 6-digit PIN key backup, and 60-digit safety codes.
- **In-browser translation** — Translate Polish and English chat messages locally in your browser via Bergamot WebAssembly without sending text to external APIs.
- **Voice, video, and screen sharing** — Low-latency WebRTC voice channels, camera streams, and screen sharing up to 4K 60 fps with system audio and local WASM noise suppression (RNNoise, GTCRN, Speex).
- **Six themes and bilingual UI** — Switch between *Midnight*, *Slate*, *Evergreen*, *Harbor*, *Amethyst*, and *Ink*, with full English (`en`) and Polish (`pl`) localization.
- **Built-in operator panel (`/cockpit`)** — Manage accounts, multi-factor authentication policies, OIDC single sign-on, live logs, and automatic daily backups (with optional Cloudflare R2 offsite uploads).

## One container, one folder

Stellarr packages the web client, the `/cockpit` operator panel, the ASP.NET Core API, the Rust WebRTC media server (`stellarr-media`), `coturn` STUN/TURN, and SQLite into **one Docker container** backed by **one volume** (`/data/stellarr`). You do not need external Postgres, Redis, or media servers, and backing up or migrating an instance means copying a single directory.

## Try it locally

Install [Docker](https://docs.docker.com/get-docker/) (with Docker Compose 2.24+), then run:

```bash
curl -fsSL https://stellarr.online/install.sh | bash
```

The installer checks host prerequisites, writes `docker-compose.yml` and `.env` to `~/stellarr`, starts the container, and prints the local URL along with the initial setup code. Open the address in your browser, accept the self-signed TLS warning, and complete the three-step `/setup` wizard to create the owner account and first server. Once signed in, open **`https://localhost:8443/cockpit`** for the operator panel. For a manual Docker Compose setup, see the [Quick start](docs/install/quick-start.md).

### Inviting friends over the internet

Local testing works immediately on the host machine. Before inviting connections from other devices:

1. Set `STELLARR_DOMAIN` and `STELLARR_PUBLIC_IP` in `.env`.
2. Forward the required UDP voice ports on your router — see [Requirements](docs/install/requirements.md#behind-nat).
3. Configure a trusted HTTPS certificate so browsers allow microphone, camera, and screen capture — see [Reverse proxy and TLS](docs/install/reverse-proxy-and-tls.md).

Running on **Unraid**? Follow the [Unraid setup guide](docs/install/unraid.md).

## Documentation

Browse the documentation at [**arkazzae.github.io/Stellarr**](https://arkazzae.github.io/Stellarr/) or directly in [`docs/`](docs/README.md):

- **Self-hosting**: [Quick start](docs/install/quick-start.md) · [Requirements](docs/install/requirements.md) · [Configuration](docs/install/configuration.md) · [Reverse proxy & TLS](docs/install/reverse-proxy-and-tls.md) · [Unraid](docs/install/unraid.md) · [Upgrading](docs/install/upgrading.md)
- **Administration**: [Operator CLI](docs/admin/cli.md) · [Account security](docs/admin/account-security.md) · [Automatic backups](docs/admin/backups.md) · [Cloudflare R2](docs/admin/cloudflare-r2.md) · [Activity notifications](docs/admin/notifications.md)
- **Internals & reference**: [Architecture](docs/architecture.md) · [RTC & ports](docs/rtc-and-ports.md) · [Voice & RTC protocol](docs/rtc-protocol.md) · [Stellarr Media SFU](docs/stellarr-media.md) · [Localization](docs/localization.md)

## License

Source code is licensed under the [MIT License](LICENSE). See [`branding/README.md`](branding/README.md#name-and-logo-policy) for usage rules covering the Stellarr name, logo, and mascot.
