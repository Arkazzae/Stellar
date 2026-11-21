# RTC and ports

Published host ports, internal container ports, NAT and TURN forwarding rules, relay port sizing, and call diagnostics. See [Voice and RTC protocol](./rtc-protocol.md) for signaling and audio/video profiles, and [Stellarr Media](./stellarr-media.md) for the Rust SFU media server.

## Published host ports

| Default port | Protocol | Process | Purpose |
| --- | --- | --- | --- |
| `8888` | TCP | Nginx | HTTP redirect to HTTPS (`STELLARR_HTTP_PORT`) |
| `8443` | TCP | Nginx | HTTPS: web client, API, SignalR WebSocket, and Cockpit (`STELLARR_HTTPS_PORT`) |
| `3478` | UDP, TCP | `coturn` | STUN and TURN (`STELLARR_TURN_PORT`) |
| `5349` | UDP, TCP | `coturn` | TURN over TLS/DTLS (`STELLARR_TURNS_PORT`) |
| `49000` | UDP | `stellarr-media` | SFU media server port (`STELLARR_MEDIA_PORT`); carries all voice and video calls |
| `49160-49200` | UDP | `coturn` | TURN media relay port range (`STELLARR_TURN_RELAY_RANGE`, 41 ports) |

- `STELLARR_HTTPS_PORT` is included in HTTP-to-HTTPS `308` redirects (omitted when set to `443`). Requests arriving from an external reverse proxy with `X-Forwarded-Proto: https` are not redirected.
- `STELLARR_TURN_PORT`, `STELLARR_TURNS_PORT`, and `STELLARR_MEDIA_PORT` set both the host port published by Docker Compose and the port advertised to browser clients. Forward the **same** external UDP port to `STELLARR_MEDIA_PORT` on your router.
- `STELLARR_TURN_TCP_ENABLED` defaults to `false`. Disabled transports are closed in `/etc/turnserver.conf` via `no-tcp`, `no-tls`, and `no-dtls` and omitted from `/api/bootstrap`.

> [!NOTE]
> `STELLARR_TURNS_ENABLED` defaults to `false` because browsers reject self-signed certificates on `turns:` URLs. Enable it only after mounting a CA-signed TLS certificate in `/data/stellarr/tls/`.

### Relay port sizing

The default TURN relay range (`49160-49200/udp`) provides 41 UDP ports. Because Stellarr uses an SFU architecture (`stellarr-media`), each browser opens a single bundled `RTCPeerConnection` (`max-bundle`) to the server, requiring at most 1 TURN relay allocation per relayed participant (~5 ports for a 5-person call), which `coturn` forwards internally to the container's local `stellarr-media` UDP socket.

To expand the relay range for larger deployments, update `min-port`/`max-port` in `infra/turnserver.conf.template`, `STELLARR_TURN_RELAY_RANGE` / `ports` in `docker-compose.yml`, `EXPOSE` in `Dockerfile`, and your host and router firewall rules.

## Container-internal ports

| Port | Service |
| --- | --- |
| `5000/tcp` | ASP.NET Core API and SignalR (`127.0.0.1:5000`) |
| `5200/tcp` | `stellarr-media` HTTP control API (`127.0.0.1:5200`, authenticated with `/data/stellarr/secrets/media-control-secret`) |

Nginx routes HTTP and WebSocket requests to ASP.NET Core or serves static bundles from `/opt/stellarr/web`; see [Nginx routing and cache policy](./architecture.md#nginx-routing-and-cache-policy).

## NAT and firewall configuration

HTTP reverse proxies carry only HTTP and WebSocket traffic. WebRTC STUN, TURN, and SFU RTP/SRTP packets bypass HTTP proxies and must reach the Docker host directly.

Forward these ports on your router to the Docker host:

- TCP `80`/`443` (or your external `STELLARR_HTTP_PORT` / `STELLARR_HTTPS_PORT`)
- UDP `3478` (plus TCP `3478` if `STELLARR_TURN_TCP_ENABLED=true`)
- UDP/TCP `5349` (if `STELLARR_TURNS_ENABLED=true`)
- UDP `49160-49200`
- UDP `49000` (or `STELLARR_MEDIA_PORT`) on the **same external UDP port**

Recommended `.env` settings behind NAT:

```dotenv
STELLARR_DOMAIN=chat.example.com
STELLARR_PUBLIC_IP=auto    # or a static external IPv4 address
STELLARR_MEDIA_PORT=49000
```

- `STELLARR_PUBLIC_IP` sets `external-ip` in `coturn` and `STELLARR_MEDIA_PUBLIC_IP` in `stellarr-media` (`auto` resolves the IPv4 `A` record of `STELLARR_DOMAIN` at startup and aborts startup if resolution fails).
- `stellarr-media` advertises `STELLARR_PUBLIC_IP:STELLARR_MEDIA_PORT` first in its SDP host candidates, followed by the container's local IPv4 address (which `coturn` uses when relaying over loopback/bridge).
- The API issues short-lived (1-hour) HMAC-SHA1 TURN credentials derived from `/data/stellarr/secrets/turn-shared-secret` to authenticated users; the shared secret never leaves the container.

> [!WARNING]
> If `STELLARR_PUBLIC_IP` is unset, `stellarr-media` advertises only the container's internal IPv4 address. External browsers can then connect only through TURN (`coturn`), and the container logs a warning on startup.

For outbound network requirements, see [Outbound network](./install/requirements.md#outbound-network).

## Diagnostics

When voice fails to connect, verify in order:

1. The web client is loaded over `https://` (or `http://localhost`).
2. Microphone permissions are granted in the browser.
3. `GET /api/bootstrap` returns `rtc.mediaServer` with the expected hostname and port.
4. `STELLARR_PUBLIC_IP` matches the host's reachable IPv4 address (no empty-`STELLARR_PUBLIC_IP` warning in `docker compose logs`).
5. UDP `49000` (`STELLARR_MEDIA_PORT`) is forwarded on the router to the Docker host on the same external UDP port.
6. UDP `3478` (`STELLARR_TURN_PORT`) is reachable from the client network.
7. UDP `49160-49200` (`STELLARR_TURN_RELAY_RANGE`) is open in host and router firewalls.
8. `docker compose exec stellarr stellarr-init status` shows both `media` and `turn` as `ready`.
9. Browser DevTools shows `MediaSignal` events arriving over `/hubs/app`.
10. `stellarr-media` lists active sessions as `connected` with advancing `ingress` and `egress` packet counters (queried via bash `/dev/tcp` since the runtime image omits `curl`):

    ```bash
    docker compose exec stellarr bash -c '
      exec 3<>/dev/tcp/127.0.0.1/5200
      printf "GET /v1/stats HTTP/1.0\r\nAuthorization: Bearer %s\r\n\r\n" \
        "$(cat /data/stellarr/secrets/media-control-secret)" >&3
      cat <&3'
    ```
