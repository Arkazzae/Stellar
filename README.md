# Stellarr

**Talk on your own server.** A self-hosted Discord alternative built for you and your friends.

Private by design. (Over)Engineered for performance  ·  [Try it](#try-it) · [Inviting friends](#inviting-friends) · [Docs](https://arkazzae.github.io/Stellarr/)

![A tour of Stellarr: friends planning game night in a text channel, translating a message right in the browser, hanging out in voice with screen sharing, switching themes, checking Cockpit, and starting the container.](branding/film/stellarr.webp)

> [!WARNING]
> Stellarr is an early preview and may still have breaking changes before v1.0. Optional 1:1 end-to-end encryption is experimental and has not been independently audited yet.

Hop into voice, stream your game in 4K 60 fps without paying for Nitro, spam the soundboard, and keep all of your group's chats on your own hardware.

### Built for game night

- Stream games and screens up to 4K 60 fps with system audio, webcams, and built-in local noise suppression.
- Hang out with text channels, DMs, threads, replies, polls, reactions, voice messages, custom emojis, stickers, server soundboards, and six built-in themes.
- Translate messages across languages right inside the browser, offline and without character limits (starting with English and Polish).

### Private by design

- Your server stays on your machine with no cloud accounts, ads, or subscriptions. Noise suppression and message translation run locally in your browser, and link previews are fetched by your server so friends' browsers never ping external sites (you can turn previews off too).
- Flip on optional 1:1 end-to-end encryption for private DMs, file uploads, and voice or video calls, with PIN-protected key backup and safety codes.
- Lock down accounts with passkeys, TOTP, email codes, or your own OIDC provider, and schedule automatic daily backups to disk or Cloudflare R2.

### Easy to host, (over)engineered inside

Easy setup is a core focus of Stellarr. Instead of wiring up Postgres, Redis, LiveKit, and five separate containers just to hang out with your friends, everything runs inside a single `amd64` / `arm64` Docker container (~120–150 MiB RAM at idle) backed by one folder (`/data/stellarr`), with a guided `/setup` flow and a built-in `/cockpit` panel for accounts, live logs, backups, and settings.

Under the hood, Stellarr pairs a custom Rust media server built specifically for this app with ASP.NET Core, SQLite, built-in STUN/TURN, and a static web frontend. Backing up or moving your server is just copying one directory. See [Architecture](docs/architecture.md) if you want the full breakdown.

### Coming next

- Dedicated desktop and mobile clients alongside the web app.
- An Activities SDK for running shared mini-apps and games right inside your server.
- More languages for offline in-browser translation beyond English and Polish.
- Import your discord server structure with resources!

## Try it

Install [Docker](https://docs.docker.com/get-docker/) (with Compose `2.24+`), then run:

```bash
curl -fsSL https://stellarr.online/install.sh | bash
```

Or start it manually with Docker Compose

Grab [`docker-compose.yml`](docker-compose.yml) and [`.env.example`](.env.example) (or read [`install.sh`](install.sh) first):

```bash
mkdir stellarr && cd stellarr
curl -fsSLO https://raw.githubusercontent.com/Arkazzae/Stellarr/main/docker-compose.yml
curl -fsSL https://raw.githubusercontent.com/Arkazzae/Stellarr/main/.env.example -o .env
docker compose up -d
```

Want to build from source instead? Clone the repo and run `docker compose up -d --build`.



1. Grab the one-time setup code printed by the installer (or run `docker compose logs stellarr | grep "Setup code"`).
2. Open `https://localhost:8443`, accept the local self-signed certificate, and run through `/setup` to create your owner account and first server.
3. Visit `https://localhost:8443/cockpit` anytime to manage accounts, security, logs, and backups.

## Inviting friends

Local testing works right away on your machine. Before inviting friends over the internet:

1. Set `STELLARR_DOMAIN` and `STELLARR_PUBLIC_IP` (or `auto`) in `.env` and restart with `docker compose up -d`.
2. Forward UDP ports `49000`, `3478`, and `49160-49200` on your router so voice and video can connect ([Requirements](docs/install/requirements.md#behind-nat)).
3. Add a browser-trusted HTTPS certificate or put Stellarr behind your reverse proxy so browsers allow mic, camera, and screen capture ([Reverse proxy & TLS](docs/install/reverse-proxy-and-tls.md)).

> [!TIP]
> Running Unraid? There is a ready-to-use container template and walkthrough in the [Unraid guide](docs/install/unraid.md).

## Documentation

Read the full docs at [arkazzae.github.io/Stellarr](https://arkazzae.github.io/Stellarr/) or browse [`docs/`](docs/README.md):

- **Self-hosting:** [Quick start](docs/install/quick-start.md) · [Requirements](docs/install/requirements.md) · [Configuration](docs/install/configuration.md) · [Reverse proxy & TLS](docs/install/reverse-proxy-and-tls.md) · [Unraid](docs/install/unraid.md) · [Upgrading](docs/install/upgrading.md)
- **Administration:** [Operator CLI](docs/admin/cli.md) · [Account security](docs/admin/account-security.md) · [Automatic backups](docs/admin/backups.md) · [Cloudflare R2](docs/admin/cloudflare-r2.md) · [Notifications](docs/admin/notifications.md)
- **Under the hood:** [Architecture](docs/architecture.md) · [RTC & ports](docs/rtc-and-ports.md) · [Voice & RTC protocol](docs/rtc-protocol.md) · [Stellarr Media SFU](docs/stellarr-media.md) · [Localization](docs/localization.md)

## License

Source code is licensed under the [MIT License](LICENSE). See [`branding/README.md`](branding/README.md#name-and-logo-policy) for usage rules covering the Stellarr name, logo, and mascot.
