# Stellarr

**Talk on your own server.** A self-hosted Discord alternative built for you and your friends.

Private by design. (Over)Engineered for performance  ·  [Try it](#try-it) · [Inviting friends](#inviting-friends) · [Docs](https://arkazzae.github.io/Stellarr/)

![A tour of Stellarr: friends planning game night in a text channel, translating a message right in the browser, hanging out in voice with screen sharing, switching themes, checking Cockpit, and starting the container.](website/src/images/film/stellarr.webp)

> [!WARNING]
> Stellarr is an early preview and may still have breaking changes before v1.0. Optional 1:1 end-to-end encryption is experimental and has not been independently audited yet.

Hop into voice, stream your game in 4K 60 fps without paying for Nitro, spam the soundboard, bring over your favorite theme, and keep your group's chats on your own hardware.

### Built for game night

- Stream games and screens up to 4K 60 fps with stereo audio and mid-stream quality switching, talk in Opus up to 256 kb/s, and clean up noisy mics right in the browser.
- Run public and private channels with roles, threads, polls, and voice messages. Bring your own emojis, hit the server soundboard, or set a walk-on clip for when you join voice.
- [Import from Discord](docs/admin/servers-and-features.md#import-from-discord) to copy selected public channels and message history into a new private Stellarr server. Copy attachments from the import report.
- Make the app yours with dark and light themes, custom colors, and wallpapers. Share themes with friends as a file, or import a BetterDiscord theme.
- Translate messages right inside the browser, offline and without character limits, starting with English and Polish.

### Private by design

- Your server stays on your machine with no cloud accounts, ads, quests, or subscriptions. Noise suppression and message translation run locally in your browser, and link previews are fetched by your server so friends' browsers never ping external sites (you can turn previews off too).
- Flip on optional 1:1 end-to-end encryption for private DMs, file uploads, and voice or video calls, with PIN-protected key backup and safety codes.
- Lock down accounts with passkeys, TOTP, email codes, or your own OIDC provider, and schedule automatic daily backups to disk or Cloudflare R2.

### Easy to host, (over)engineered inside

Easy setup is a core focus of Stellarr. Instead of wiring up Postgres, Redis, LiveKit, and five separate containers just to hang out with your friends, everything runs inside a single `amd64` / `arm64` Docker container for small group of users (35 MiB RAM at idle) backed by one folder (`/data/stellarr`), with a guided `/setup` flow and a built-in `/cockpit` panel for accounts, live logs, backups, and settings.

Under the hood, the entire backend is written in Rust: a custom WebRTC media server built specifically for Stellarr, the chat API and WebSocket hub, and the container's own init supervisor, paired with SQLite, optional STUN/TURN, and a static web frontend. Backing up or moving your server is just copying one directory. See [Architecture](docs/architecture.md) if you want the full breakdown.

### Coming next

- Dedicated desktop and mobile clients alongside the web app.
- An Activities SDK for running shared mini-apps and games right inside your server.
- More languages for offline in-browser translation beyond English and Polish.

## Try it

Install [Docker](https://docs.docker.com/get-docker/) (with Compose `2.24+`), then run:

```bash
curl -fsSL https://stellarr.online/install.sh | bash
```

Or start it manually with Docker Compose ([`docker-compose.yml`](docker-compose.yml) and [`.env.example`](.env.example)):

```bash
mkdir stellarr && cd stellarr
curl -fsSLO https://raw.githubusercontent.com/Arkazzae/Stellarr/main/docker-compose.yml
curl -fsSL https://raw.githubusercontent.com/Arkazzae/Stellarr/main/.env.example -o .env
docker compose up -d
```

To build from source with Docker, run this in a checkout:

```bash
docker compose -f docker-compose.yml -f infra/compose/build.yml up -d --build
```

With Cargo installed, you can run the installer from the checkout before a release is available:

```bash
cargo run --locked --release --manifest-path init/installer/Cargo.toml -- --build --dir "$PWD"
```

`docker-compose.yml` defines the production stack. `infra/compose/` contains the local build, TURN, and development configurations.

1. Grab the one-time setup code printed by the installer (or run `docker compose logs stellarr | grep "Setup code"`).
2. Open the HTTPS address printed by the installer (default `https://localhost`), accept the local self-signed certificate, and run through `/setup` to create your owner account and first server.
3. Visit `/cockpit` on your instance to manage accounts, security, logs, and backups.

## Install the client

Open your instance over trusted HTTPS. After joining, choose **Install Stellarr** from the menu beneath the server list. Chrome and Edge offer an install dialog when available. In Safari on iPhone or iPad, choose **Share → Add to Home Screen**. On macOS, choose **File → Add to Dock**. The icon opens all your servers on that instance. For mentions and direct messages on a closed app, enable [Web Push](docs/admin/notifications.md).

## Inviting friends

Local testing works right away on your machine. Before inviting friends over the internet:

1. Set `STELLARR_DOMAIN` and `STELLARR_PUBLIC_IP` (or `auto`) in `.env` and restart with `docker compose up -d`.
2. Forward HTTPS on `443/tcp` and media on `49000/udp` and `49000/tcp`, or the ports configured for your instance. Voice and video use direct UDP when available and can fall back to ICE-TCP. Enable and forward TURN separately if you need a relay. [Requirements](docs/install/requirements.md#behind-nat)
3. Add a browser-trusted HTTPS certificate or put Stellarr behind Caddy or Nginx so browsers allow mic, camera, and screen capture ([Reverse proxy and TLS](docs/install/reverse-proxy-and-tls.md)).

> [!TIP]
> Running Unraid? There is a ready-to-use container template and walkthrough in the [Unraid guide](docs/install/unraid.md).

## Documentation

Read the full docs at [arkazzae.github.io/Stellarr](https://arkazzae.github.io/Stellarr/) or browse [`docs/`](docs/README.md):

- **Self-hosting:** [Quick start](docs/install/quick-start.md) · [Requirements](docs/install/requirements.md) · [Configuration](docs/install/configuration.md) · [Reverse proxy and TLS](docs/install/reverse-proxy-and-tls.md) · [Unraid](docs/install/unraid.md) · [Upgrading](docs/install/upgrading.md)
- **Administration:** [Servers and features](docs/admin/servers-and-features.md) · [Stellarr Cockpit](docs/admin/cockpit.md) · [Operator CLI](docs/admin/cli.md) · [Account security](docs/admin/account-security.md) · [Automatic backups](docs/admin/backups.md) · [Cloudflare R2](docs/admin/cloudflare-r2.md) · [Notifications](docs/admin/notifications.md)
- **Reference:** [Architecture](docs/architecture.md) · [Themes and appearance](docs/themes.md) · [RTC and ports](docs/rtc-and-ports.md) · [Voice and RTC protocol](docs/rtc-protocol.md) · [Stellarr Media SFU](docs/stellarr-media.md) · [Localization](docs/localization.md)
- **Development:** [Local setup](docs/development/local-setup.md) · [Codebase and testing](docs/development/codebase-and-testing.md)

## License

Source code is licensed under the [MIT License](LICENSE).
