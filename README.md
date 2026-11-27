# Stellarr

**Talk on your own server.** A self-hosted Discord alternative built for you and your friends.

[Try it](#try-it) · [Inviting friends](#inviting-friends) · [Docs](https://arkazzae.github.io/Stellarr/)

![A tour of Stellarr: friends planning game night in a text channel, translating a message right in the browser, hanging out in voice with screen sharing, switching themes, checking Cockpit, and starting the container.](website/src/images/film/stellarr.webp)

> [!WARNING]
> Stellarr is an early preview and may still have breaking changes before v1.0. Optional 1:1 end-to-end encryption is experimental and has not been independently audited yet.

Hop into voice, stream your game in 4K 60 fps without paying for Nitro, spam the soundboard, bring over your favorite theme, and keep your group's chats on your own hardware.

### Built for game night

- Stream games and screens up to 4K 60 fps with stereo audio and mid-stream quality switching, talk in Opus up to 256 kb/s, and clean up noisy mics right in the browser.
- Play activities together in a voice channel or open one solo. Build your own with the [Activity SDK](docs/sdk/getting-started.md).
- Run public and private channels with roles, threads, polls, and voice messages. Bring your own emojis, hit the server soundboard, or set a walk-on clip for when you join voice.
- [Import from Discord](docs/admin/servers-and-features.md#import-from-discord) to copy selected public channels and message history into a new private Stellarr server. Copy attachments from the import report.
- Make the app yours with dark and light themes, custom colors, and wallpapers. Share themes with friends as a file, or import a BetterDiscord theme.
- Use the chat client and Cockpit in English, Polish or German.
- Translate messages right inside the browser, offline and without character limits, starting with English and Polish.

### Private by design

- Your server stays on your machine with no cloud accounts, ads, quests, or subscriptions. Noise suppression and message translation run locally in your browser, and link previews are fetched by your server so friends' browsers never ping external sites (you can turn previews off too).
- Flip on optional 1:1 end-to-end encryption for private DMs, file uploads, and voice or video calls, with a browser-generated recovery code for your key backup and safety codes for verifying contacts.
- Lock down accounts with passkeys, TOTP, email codes, or your own OIDC provider, and schedule automatic daily backups to disk or Cloudflare R2.

### Hosting options

The **single-container release** runs on `amd64` and `arm64`. Nginx serves the web client and Cockpit. Stellarr Init supervises Nginx, the Rust API, the WebRTC media server and optional TURN. SQLite and uploaded files live under `/data/stellarr`.

**Compose services** builds a separate image for each service. Choose it if you want to restart the API independently or collect web and media logs separately. [Deployment guide](deploy/services/README.md)

For a live backup, download a full archive from **Cockpit → Backups**. To copy the data volume by hand, stop the container first and save your `.env` alongside it. [Backup and restore](docs/install/upgrading.md)

### Coming next

- Dedicated desktop and mobile clients alongside the web app.
- More languages for offline in-browser translation beyond English and Polish.

## Try it

With Docker and Compose 2.24 or newer, check out a server release and select its matching image. These commands set up a new instance.

```bash
git clone --depth 1 --branch server-v0.3.1 https://github.com/Arkazzae/Stellarr.git &&
  cd Stellarr &&
  cp .env.example .env &&
  echo 'STELLARR_VERSION=0.3.1' >> .env
```

Review the ports and data volume in `docker-compose.yml`. To host friends, [set the public address](#inviting-friends) in `.env` before starting. For a trial on this computer, the defaults work.

```bash
docker compose up -d --wait
docker compose logs stellarr | grep 'Setup code'
```

Open `https://localhost/setup` (or your configured address) and enter the code to create your owner account and first server. The operator panel is at `/cockpit`.

The [quick start](docs/install/quick-start.md) also covers building from source and the optional guided installer.

## Install the client

Open your instance over trusted HTTPS. After joining, choose **Install Stellarr** from the menu beneath the server list. Chrome and Edge offer an install dialog when available. In Safari on iPhone or iPad, choose **Share → Add to Home Screen**. On macOS, choose **File → Add to Dock**. The icon opens all your servers on that instance. For mentions and direct messages on a closed app, enable [Web Push](docs/admin/notifications.md).

## Inviting friends

Local testing works right away on your machine. A friend must be able to reach both the web address in the invitation and the address advertised for calls:

1. Set `STELLARR_DOMAIN` in `.env` to your hostname, such as `chat.example.com`. Set `STELLARR_PUBLIC_IP=auto` if its DNS A record points to your Docker host or router. Behind an HTTP CDN, set the host's reachable IPv4 address instead. Restart with `docker compose up -d`.
2. Forward HTTPS on `443/tcp` and media on `49000/udp` and `49000/tcp`, or the ports configured for your instance. Voice and video use direct UDP when available and can fall back to ICE-TCP. Enable and forward TURN separately if you need a relay. [Requirements](docs/install/requirements.md#behind-nat)
3. Add a browser-trusted HTTPS certificate or put Stellarr behind Caddy or Nginx so browsers allow mic, camera, and screen capture ([Reverse proxy and TLS](docs/install/reverse-proxy-and-tls.md)).

Before sharing the link, open it on a phone using mobile data and join the same voice channel from both devices. Check that each side can hear the other.

> [!TIP]
> Running Unraid? There is a ready-to-use container template and walkthrough in the [Unraid guide](docs/install/unraid.md).

## Documentation

Read the full docs at [arkazzae.github.io/Stellarr](https://arkazzae.github.io/Stellarr/) or browse [`docs/`](docs/README.md):

- **Self-hosting:** [Quick start](docs/install/quick-start.md) · [Requirements](docs/install/requirements.md) · [Configuration](docs/install/configuration.md) · [Reverse proxy and TLS](docs/install/reverse-proxy-and-tls.md) · [Hosting without port forwarding](docs/install/no-port-forwarding.md) · [Unraid](docs/install/unraid.md) · [Upgrading](docs/install/upgrading.md)
- **Administration:** [Servers and features](docs/admin/servers-and-features.md) · [Stellarr Cockpit](docs/admin/cockpit.md) · [Operator CLI](docs/admin/cli.md) · [Account security](docs/admin/account-security.md) · [Automatic backups](docs/admin/backups.md) · [Cloudflare R2](docs/admin/cloudflare-r2.md) · [Notifications](docs/admin/notifications.md)
- **Reference:** [Architecture](docs/architecture.md) · [Themes and appearance](docs/themes.md) · [RTC and ports](docs/rtc-and-ports.md) · [Voice and RTC protocol](docs/rtc-protocol.md) · [Stellarr Media SFU](docs/stellarr-media.md) · [Localization](docs/localization.md)
- **Development:** [Local setup](docs/development/local-setup.md) · [Codebase and testing](docs/development/codebase-and-testing.md)

## About the name

Stellarr is its own project. It is not part of the Servarr (*arr) family of media apps and does not need any of them.

## License

Source code is licensed under the [MIT License](LICENSE).
