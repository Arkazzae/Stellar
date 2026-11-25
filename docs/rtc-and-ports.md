# RTC and ports

Stellarr uses one HTTPS port for the web client and signaling, and one media port for the SFU. Browsers prefer direct WebRTC over UDP and can select direct ICE-TCP on the same port when UDP is unavailable. See [Voice and RTC protocol](./rtc-protocol.md) and [Stellarr Media](./stellarr-media.md).

## Published host ports

| Default binding | Protocol | Purpose |
| --- | --- | --- |
| `443` | TCP | HTTPS for the web client, API, WebSocket hub, and Cockpit (`STELLARR_HTTPS_PORT`) |
| `49000` | UDP and TCP | Direct WebRTC to Stellarr Media (`STELLARR_MEDIA_PORT`) |
| `127.0.0.1:8888` | TCP | HTTP upstream for a local reverse proxy (`STELLARR_HTTP_PORT`) |

The advertised media port is the host-side `STELLARR_MEDIA_PORT`. Map both protocols to the same external port. Port `443` is omitted from generated HTTPS URLs when it is the public HTTPS port.

### Optional TURN relay

Enable TURN if some callers can reach neither direct media transport. A fresh installation starts without a relay. With the installer, pass `--enable-turn`. For manual Compose deployments, use both files from the same release for startup and upgrades:

```bash
docker compose -f docker-compose.yml -f infra/compose/turn.yml up -d
```

In an installer-managed directory, the overlay is saved as `docker-compose.turn.yml`. Older releases also used that path in the repository.

The override enables `STELLARR_TURN_ENABLED` and publishes all the port mappings below. Forward only the ports for transports you enable, plus the relay range.

| Default port | Enabled transport | Variable |
| --- | --- | --- |
| `3478/udp` | STUN and UDP TURN | `STELLARR_TURN_PORT` |
| `3478/tcp` | TURN over TCP when `STELLARR_TURN_TCP_ENABLED=true` | `STELLARR_TURN_PORT` |
| `5349/tcp` and `5349/udp` | TURN over TLS and DTLS when `STELLARR_TURNS_ENABLED=true` | `STELLARR_TURNS_PORT` |
| `49160-49200/udp` | TURN media relay range, mapped 1:1 | `STELLARR_TURN_RELAY_RANGE` |

TURN over TLS requires a CA-signed certificate trusted by browsers. Disabled transports are closed in `/etc/turnserver.conf` and omitted from `/api/bootstrap`. See [Reverse proxy and TLS](./install/reverse-proxy-and-tls.md#turn-over-tls-turns).

The default relay range provides 41 UDP ports. A browser uses one bundled peer connection to the SFU, so a five-person call in which everyone relays needs about five allocations. To change the range, set `STELLARR_TURN_RELAY_RANGE` in `.env` and recreate the container with the TURN override. Compose publishes the same UDP range that coturn allocates. Update the host and router firewall rules to match.

## Container-internal ports

| Port | Service |
| --- | --- |
| `5000/tcp` | Stellarr API and hub on loopback |
| `5200/tcp` | Stellarr Media HTTP control API on loopback, authenticated with `/data/stellarr/secrets/media-control-secret` |

Nginx routes HTTP and WebSocket requests to the API or serves bundles from `/opt/stellarr/web`. See [Nginx routing and cache policy](./architecture.md#nginx-routing-and-cache-policy).

## NAT and firewall configuration

Forward public HTTPS (`443/tcp` by default or your reverse proxy’s HTTPS port) and the SFU media port (`49000/udp` and `49000/tcp` by default). The HTTP reverse proxy carries signaling. ICE, DTLS and media packets connect directly to the advertised media address, including when ICE selects TCP.

Recommended `.env` settings behind NAT:

```dotenv
STELLARR_DOMAIN=chat.example.com
STELLARR_PUBLIC_IP=auto
STELLARR_MEDIA_PORT=49000
```

Set `STELLARR_PUBLIC_IP` to the IPv4 address callers can reach. With `auto`, the container resolves the hostname’s IPv4 address at startup. A numeric IPv4 `STELLARR_DOMAIN` supplies the direct SFU address when `STELLARR_PUBLIC_IP` is unset. `STELLARR_MEDIA_ADVERTISE` replaces the automatically derived candidates. The SFU advertises that IP and port in SDP for both UDP and passive TCP. When TURN is enabled, the same public IP configures its external address. Without a reachable SFU candidate, remote browsers need a working TURN relay and a reachable path to the SFU.

## Diagnostics

1. Open the web client over HTTPS and grant microphone access.
2. Check that `GET /api/bootstrap` returns the expected SFU host and port in `rtc.mediaServer`. The browser takes its actual candidates from SDP. With TURN disabled, `rtc.iceServers` is empty.
3. Check that browsers can reach the advertised SFU address and that the host firewall permits both media transports. For internet access behind NAT, verify that the router forwards the same external media port on UDP and TCP.
4. Check that `docker compose exec stellarr stellarr-init status` shows `media` as ready. If using TURN, verify `turn` and its enabled relay ports as well.
5. In browser WebRTC diagnostics, inspect the selected ICE candidate pair and its candidate `protocol`. Verify that media packet counters advance in both directions.
6. Query active SFU sessions and their `ingress` and `egress` packet counters:

   ```bash
   docker compose exec stellarr bash -c '
     exec 3<>/dev/tcp/127.0.0.1/5200
     printf "GET /v1/stats HTTP/1.0\r\nAuthorization: Bearer %s\r\n\r\n" \
       "$(cat /data/stellarr/secrets/media-control-secret)" >&3
     cat <&3'
   ```
