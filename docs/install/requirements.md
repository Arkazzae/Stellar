# Requirements

Stellarr runs as a single Docker container. To test it locally on one machine, follow the [Quick start](quick-start.md).

## Host

| Resource | Requirement |
| --- | --- |
| Software | Docker Engine with the Compose plugin (`docker compose`), `2.24.0` or newer |
| Architecture | `linux/amd64` or `linux/arm64` |
| Memory | At least 2 GiB host RAM recommended; 4 GiB default container cap (`STELLARR_CONTAINER_MEMORY_LIMIT`), ~120–150 MiB at idle (and smoke-tested end-to-end in CI inside `512 MiB` RAM and `1` CPU core) |
| CPU | 4 CPUs default container cap (`STELLARR_CONTAINER_CPUS`) |
| Process limit | 512 processes (`STELLARR_CONTAINER_PIDS_LIMIT`) |
| Disk | At least 2 GiB free; ~390 MB unpacked image (~152 MB compressed) plus `/data/stellarr` (`stellarr_stellarr_data` volume) |

Uploads enforce per-user and instance storage quotas plus a free-space floor on `/data/stellarr`: 2 GiB per account (`STELLARR_UPLOAD_USER_BYTES`), 20 GiB total (`STELLARR_UPLOAD_TOTAL_BYTES`), and at least 1 GiB free disk space (`STELLARR_MIN_FREE_BYTES`). See [Configuration](configuration.md#uploads-and-quotas).

## Ports

By default, `docker-compose.yml` publishes `443/tcp` for HTTPS and WebSocket traffic, and `49000/udp` and `49000/tcp` for the Stellarr Media SFU. The HTTP upstream for a local reverse proxy binds to `127.0.0.1:8888`. TURN ports are published only when you use the optional TURN Compose override.

See [RTC and ports](../rtc-and-ports.md#published-host-ports) for the full port table, corresponding `STELLARR_*` variables, internal loopback ports, and relay port sizing.

## Behind NAT

With the default `localhost` configuration, Stellarr advertises the published media port on loopback for browsers on the Docker host. For other devices, the SFU must advertise an IPv4 address those browsers can reach. Set `STELLARR_PUBLIC_IP` to that address, use `auto` to resolve the IPv4 address of `STELLARR_DOMAIN` at startup, or set `STELLARR_MEDIA_ADVERTISE` explicitly. If `STELLARR_PUBLIC_IP` is unset, a numeric IPv4 `STELLARR_DOMAIN` is enough for direct media.

- **LAN:** Use the Docker host’s LAN IPv4 address and allow HTTPS and both media transports (`49000/udp` and `49000/tcp` by default) through the host firewall. LAN clients using the host’s LAN address need no router port forwarding.
- **Internet behind NAT:** Advertise the reachable public IPv4 address and forward the configured media port on both UDP and TCP to the Docker host on the same external port. Forward HTTPS to Stellarr or to your reverse proxy. If you enable TURN, forward its enabled transports and UDP relay range too. See [Reverse proxy and TLS](reverse-proxy-and-tls.md#run-behind-a-reverse-proxy).

See [RTC and ports: NAT and firewall configuration](../rtc-and-ports.md#nat-and-firewall-configuration) for candidate advertisement rules and call diagnostics.

## Domain and TLS

When `STELLARR_DOMAIN` is unset, TLS certificate generation and, when TURN is enabled, the TURN realm default to `localhost`, which works only on the Docker host. Remote connections require a domain name or IPv4 address pointing to the host. On first start, the container generates a 365-day self-signed certificate for `STELLARR_DOMAIN`. To use a browser-trusted certificate, see [Reverse proxy and TLS](reverse-proxy-and-tls.md).

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
