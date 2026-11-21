# RTC and ports

This page covers host and container ports, NAT/TURN forwarding, relay sizing, and call troubleshooting. See [Voice and RTC protocol](./rtc-protocol.md) for signaling and audio/video profiles, and [Stellarr Media](./stellarr-media.md) for the Rust SFU media server.

## Published host ports

| Default port | Protocol | Process | Purpose |
| --- | --- | --- | --- |
| `8888` | TCP | Nginx | HTTP redirect to HTTPS (`STELLARR_HTTP_PORT`) |
| `8443` | TCP | Nginx | HTTPS: web client, API, SignalR WebSocket, and Cockpit (`STELLARR_HTTPS_PORT`) |
| `3478` | UDP, TCP | `coturn` | STUN and TURN (`STELLARR_TURN_PORT`) |
| `5349` | UDP, TCP | `coturn` | TURN over TLS/DTLS (`STELLARR_TURNS_PORT`) |
| `49000` | UDP | `stellarr-media` | Media server port (`STELLARR_MEDIA_PORT`); carries every call |
| `49160-49200` | UDP | `coturn` | TURN relay port range (`STELLARR_TURN_RELAY_RANGE`) |

- `STELLARR_HTTPS_PORT` is included in HTTP-to-HTTPS redirects (omitted when `443`). Requests from an upstream proxy carrying `X-Forwarded-Proto: https` are not redirected.
- `STELLARR_TURN_PORT`, `STELLARR_TURNS_PORT`, and `STELLARR_MEDIA_PORT` control both published host ports and the ports advertised to browsers. Forward the **same** external UDP port to `STELLARR_MEDIA_PORT` on your router.
- `STELLARR_TURN_TCP_ENABLED` defaults to `false` (disabled transports are turned off in `/etc/turnserver.conf` via `no-tcp`, `no-tls`, `no-dtls` and omitted from `/api/bootstrap`).

> [!NOTE]
> `STELLARR_TURNS_ENABLED` defaults to `false` because browsers reject self-signed certificates on `turns:` URLs. Enable it only after mounting a CA-signed certificate in `/data/stellarr/tls/`.

### Relay port sizing

The default relay range (`49160-49200/udp`) provides 41 UDP ports. Each browser opens one peer connection to `stellarr-media`, requiring at most 1 TURN allocation per relayed participant (~5 ports for a 5-person call), relayed internally to the container's local UDP socket.

To expand the range for larger calls or TURN/TCP, update `min-port`/`max-port` in `infra/turnserver.conf.template`, `ports` in `docker-compose.yml`, `EXPOSE` in `Dockerfile`, and your host/router firewall rules.

## Container-internal ports

| Port | Service |
| --- | --- |
| `5000/tcp` | ASP.NET Core API and SignalR (`127.0.0.1`) |
| `5200/tcp` | `stellarr-media` HTTP control API (`127.0.0.1`, bearer token in `secrets/media-control-secret`) |

Nginx routes HTTP and WebSocket paths to ASP.NET Core or serves static bundles directly; see [Nginx routing](./architecture.md#nginx-routing).

## NAT and firewall configuration

HTTP reverse proxies only carry HTTP and WebSocket traffic. WebRTC STUN, TURN, and SFU UDP packets bypass HTTP proxies and must reach the Docker host directly.

Forward these ports on your router to the Docker host:

- TCP `80`/`443` (or your external `STELLARR_HTTP_PORT` / `STELLARR_HTTPS_PORT`)
- UDP `3478` (and TCP `3478` if `STELLARR_TURN_TCP_ENABLED=true`)
- UDP/TCP `5349` (if `STELLARR_TURNS_ENABLED=true`)
- UDP `49160-49200`
- UDP `49000` (or `STELLARR_MEDIA_PORT`) on the **same external port**

Recommended `.env` settings behind NAT:

```dotenv
STELLARR_DOMAIN=chat.example.com
STELLARR_PUBLIC_IP=auto    # or a static public IPv4 address
STELLARR_MEDIA_PORT=49000
```

- `STELLARR_PUBLIC_IP` sets `external-ip` in `coturn` and the public host candidate in `stellarr-media` (`auto` resolves `STELLARR_DOMAIN`'s `A` record at startup and fails if none is returned).
- `stellarr-media` advertises `STELLARR_PUBLIC_IP:STELLARR_MEDIA_PORT` followed by the container's local IPv4 address (for internal `coturn` relaying).
- The API issues short-lived (1-hour) HMAC-derived TURN credentials to signed-in users; the shared secret never leaves the container.

> [!WARNING]
> If `STELLARR_PUBLIC_IP` is unset, `stellarr-media` advertises only the container's internal IPv4 address. External browsers can then connect only through TURN (`coturn`), and the container logs a warning on startup.

For details on what outbound traffic Stellarr needs, see [Outbound network](./install/requirements.md#outbound-network).

## Diagnostics

When voice fails to connect, check in order:

1. The client is served over `https://` (or `http://localhost`).
2. Microphone permissions are granted in the browser.
3. `GET /api/bootstrap` returns `rtc.mediaServer` with the expected host and port.
4. `STELLARR_PUBLIC_IP` matches the host's reachable IPv4 address (no empty-`STELLARR_PUBLIC_IP` warning in logs).
5. `49000/udp` (`STELLARR_MEDIA_PORT`) is forwarded to the host on the same external UDP port.
6. `3478/udp` (`STELLARR_TURN_PORT`) is reachable from the client network.
7. `49160-49200/udp` (`STELLARR_TURN_RELAY_RANGE`) is open in host and router firewalls.
8. `docker compose exec stellarr stellarr-init status` shows `media` and `turn` as `ready`.
9. Clients receive `MediaSignal` events on `/hubs/app`.
10. `stellarr-media` lists active sessions as `connected` with advancing `ingress` and `egress` counters (using bash's `/dev/tcp` since the image omits `curl`):

    ```bash
    docker compose exec stellarr bash -c '
      exec 3<>/dev/tcp/127.0.0.1/5200
      printf "GET /v1/stats HTTP/1.0\r\nAuthorization: Bearer %s\r\n\r\n" \
        "$(cat /data/stellarr/secrets/media-control-secret)" >&3
      cat <&3'
    ```
