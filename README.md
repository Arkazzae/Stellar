# Stellarr

<p align="center">
  <img src="branding/film/stellarr.webp" width="800" alt="A one-minute tour of Stellarr: the astronaut mascot turns into the logo, friends plan a game night in a text channel, a Polish message is translated in the browser, four people talk in a voice channel while one of them shares a screen, the interface switches through its six palettes, the owner checks the instance in Cockpit, and one docker compose command starts the single container.">

[Experimental] [DEVELOPMENT] The self-hosted Discord alternative for your friends. Fast, solid, and fully yours.

## Documentation

Full documentation lives in [`docs/`](docs/README.md):

- [Requirements](docs/install/requirements.md)
- [Quick start](docs/install/quick-start.md)
- [Platforms](docs/install/platforms.md)
- [Unraid](docs/install/unraid.md)
- [Configuration](docs/install/configuration.md)
- [Operator CLI](docs/install/cli.md)
- [Reverse proxy and TLS](docs/install/reverse-proxy-and-tls.md)
- [Upgrading](docs/install/upgrading.md)
- [Architecture](docs/architecture.md)
- [RTC and ports](docs/rtc-and-ports.md)
- [Voice and RTC protocol](docs/rtc-protocol.md)
- [Stellarr Media (SFU)](docs/stellarr-media.md)
- [Localization](docs/localization.md)

## Repository layout

| Directory | Contents |
| --- | --- |
| `web/` | The Next.js app: the web client in `web/src/stellarr/` and Cockpit in `web/src/cockpit/`, with the routes of both in `web/src/app/` |
| `web/src/cockpit/` | [Stellarr Cockpit](web/src/cockpit/README.md), the instance operator panel (`/cockpit`), served by the same Next.js app |
| `packages/` | Shared packages used by the web client and Cockpit in one npm workspace |
| `packages/stellarr-ui/` | [`@stellarr/ui`](packages/stellarr-ui/README.md), the components and their stylesheet |
| `packages/stellarr-icons/` | [`@stellarr/icons`](packages/stellarr-icons/README.md), the icon set, which also builds the SVGs in `branding/icons/` |
| `artifacts/` | The prebuilt files the browser loads at run time (the noise suppressors, the translation runtime and its models), checked against `manifest.json` and copied into `web/public/` before every development run, build and test |
| `server/` | ASP.NET Core API, SignalR hub, operator CLI, and integration tests |
| `media/` | [Stellarr Media](media/README.md), the Rust WebRTC SFU (`stellarr-media`) |
| `infra/` | Container entrypoint, Nginx, coturn, supervisord, and TLS scripts |
| `deploy/` | [Platform templates](deploy/README.md) for the production container, such as Unraid's, and the optional [Cloudflare backup relay](deploy/cloudflare/backup-relay/README.md) |
| `scripts/` | [Development scripts](scripts/README.md) that seed the local database with realistic activity |
| `docs/` | VitePress documentation site |
| `branding/` | [Logo, mascot, and brand guidelines](branding/README.md), and the [README film](branding/film/README.md) |

## License

Code is licensed under the [MIT License](LICENSE). See [`branding/README.md`](branding/README.md#name-and-logo-policy) for use of the Stellarr name, logo, and mascot.
