# Requirements

Stellarr runs as a single Docker container. To test it locally on one machine, follow the [Quick start](quick-start.md).

## Host

| Resource | Requirement |
| --- | --- |
| Software | Docker Engine with the Compose plugin (`docker compose`), `2.24.0` or newer |
| Architecture | `linux/amd64` or `linux/arm64` |
| Memory | At least 2 GiB host RAM recommended; 4 GiB default container cap (`STELLARR_CONTAINER_MEMORY_LIMIT`), ~120–150 MiB at idle |
| CPU | 4 CPUs default container cap (`STELLARR_CONTAINER_CPUS`) |
| Process limit | 512 processes (`STELLARR_CONTAINER_PIDS_LIMIT`) |
| Disk | At least 2 GiB free; ~390 MB unpacked image (~152 MB compressed) plus `/data/stellarr` (`stellarr_stellarr_data` volume) |

Uploads enforce per-user and instance storage quotas plus a free-space floor on `/data/stellarr`: 2 GiB per account (`STELLARR_UPLOAD_USER_BYTES`), 20 GiB total (`STELLARR_UPLOAD_TOTAL_BYTES`), and at least 1 GiB free disk space (`STELLARR_MIN_FREE_BYTES`). See [Configuration](configuration.md#uploads-and-quotas).

## Ports

`docker-compose.yml` publishes the following host ports. By default, `3478/tcp` and `5349` (TURN over TCP and TURN over TLS/DTLS) are disabled in `coturn` unless explicitly enabled in `.env`.

| Default port | Protocol | Purpose | Required for | Variable |
| --- | --- | --- | --- | --- |
| `8443` | TCP | HTTPS: web client, API, SignalR WebSocket, and Cockpit (`/cockpit`) | Always | `STELLARR_HTTPS_PORT` |
| `8888` | TCP | HTTP redirect to HTTPS | HTTP redirect only | `STELLARR_HTTP_PORT` |
| `49000` | UDP | Stellarr Media (`stellarr-media`), carries all voice and video calls | Voice & video | `STELLARR_MEDIA_PORT` |
| `3478` | UDP | STUN and TURN (UDP) | Voice & video | `STELLARR_TURN_PORT` |
| `49160-49200` | UDP | TURN media relay range (41 ports, mapped 1:1) | Voice & video | `STELLARR_TURN_RELAY_RANGE` |
| `3478` | TCP | TURN over TCP (when `STELLARR_TURN_TCP_ENABLED=true`) | Optional | `STELLARR_TURN_PORT` |
| `5349` | TCP, UDP | TURN over TLS/DTLS (when `STELLARR_TURNS_ENABLED=true`; requires a CA-signed TLS certificate) | Optional | `STELLARR_TURNS_PORT` |

Internal services (SQLite, ASP.NET Core on `127.0.0.1:5000`, and the `stellarr-media` control API on `127.0.0.1:5200`) listen only on container loopback. See [RTC and ports](../rtc-and-ports.md) for relay sizing and troubleshooting.

## Behind NAT

All voice and video media routes through `stellarr-media` inside the container. A browser running on the Docker host itself connects over local loopback without extra configuration. Calls from any other device—including other machines on your LAN—require:

1. `STELLARR_PUBLIC_IP` set to the IPv4 address browsers use to reach the Docker host (your LAN IPv4 for LAN-only use, your public IPv4 for internet access, or `auto` to resolve the `A` record of `STELLARR_DOMAIN` at container startup).
2. UDP `49000` (`STELLARR_MEDIA_PORT`) forwarded on your router to the Docker host on the **same external UDP port**, because `stellarr-media` advertises that port in its SDP ICE candidates.
3. UDP `3478`, UDP `49160-49200`, and the HTTPS port (or an external [reverse proxy](reverse-proxy-and-tls.md#run-behind-a-reverse-proxy)) forwarded to the Docker host.

> [!WARNING]
> When `STELLARR_PUBLIC_IP` is unset, `stellarr-media` advertises only the container's internal bridge IPv4 address. Browsers outside the Docker host can then connect only via TURN (`coturn`), and calls will fail completely if TURN is also unreachable. See [Stellarr Media](../stellarr-media.md) and [RTC and ports](../rtc-and-ports.md#nat-and-firewall-configuration).

## Domain and TLS

When `STELLARR_DOMAIN` is unset, TLS certificate generation and TURN realm configuration default to `localhost`, which works only on the Docker host. Remote connections require a domain name or IPv4 address pointing to the host. On first start, the container generates a 365-day self-signed certificate for `STELLARR_DOMAIN`. To use a browser-trusted certificate, see [Reverse proxy and TLS](reverse-proxy-and-tls.md).

## Browser requirements

> [!NOTE]
> Browsers restrict WebRTC microphone, camera, and screen capture (`getUserMedia` and `getDisplayMedia`), Service Workers, Web Push, and WebCrypto to secure contexts (`https://` or `http://localhost`). Remote deployments must serve the web client over HTTPS.

End-to-end encrypted direct calls additionally require a browser supporting WebRTC Encoded Transform (`RTCRtpScriptTransform` or `createEncodedStreams`) and WebCrypto `X25519`.

## Outbound network

Stellarr makes no outbound requests to external servers unless optional features are enabled:

- **Voice, noise suppression, and translation run offline**: The web client, audio worklets, WASM noise suppressors (`rnnoise`, `gtcrn`, `speex`), Bergamot translator WASM, and Polish/English translation models are bundled inside the container image and served from the instance's own origin.
- **Link previews (`Stellarr__Embeds__Enabled`)**: Enabled by default for public HTTP(S) URLs posted in text channels (up to 2 links per message, 6 s timeout, private IP ranges blocked via `AllowPrivateNetworks=false`). Preview thumbnails are downloaded once by the server and stored as local attachments so reader browsers never contact the linked host. Setting `Stellarr__Embeds__Enabled=false` in `.env` stops all outbound HTTP fetching (YouTube and Vimeo links still render an embedded player stub locally because recognizing their URLs requires no network request).
- **Optional server-side integrations**: Outbound traffic occurs when you configure SMTP or Resend email delivery, OIDC identity providers, Web Push delivery, or Cloudflare R2 offsite backups.
- **GIPHY (browser-side)**: When `STELLARR_GIPHY_API_KEY` is configured and GIF search is enabled, the server contacts `api.giphy.com` only when validating the key in Cockpit. Signed-in browsers query GIPHY directly when opening the GIF picker unless the user disables GIPHY in their account privacy settings.
