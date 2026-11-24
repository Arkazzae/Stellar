# Reverse proxy and TLS

By default, the container terminates TLS itself using an auto-generated 365-day self-signed certificate. For remote deployments, you can either mount your own CA-signed certificate into the container or terminate TLS on an external reverse proxy.

## Mount a custom TLS certificate

Both Nginx and `coturn` read the TLS certificate chain and private key from `/data/stellarr/tls/`:

| Container path | Contents |
| --- | --- |
| `/data/stellarr/tls/fullchain.pem` | PEM leaf certificate followed by intermediate CA certificates |
| `/data/stellarr/tls/privkey.pem` | PEM private key |

CA-signed certificates are never overwritten by the container. On startup (`stellarr-tls-maintenance prepare`) and every 12 hours (`STELLARR_TLS_RENEW_CHECK_SECONDS`), the container verifies whether the certificate on disk is self-signed (matching `subject == issuer` and a valid self-signature via `openssl verify -check_ss_sig`, tracked by `/data/stellarr/tls/.stellarr-self-signed`). Self-signed RSA-3072 certificates (`DNS:<host>` or `IP:<host>` SAN) are regenerated automatically when expiring within 30 days (`STELLARR_TLS_RENEW_BEFORE_SECONDS`) or when `STELLARR_DOMAIN` changes, triggering `stellarr-init reload nginx` and `stellarr-init restart turn`. If an operator replaces a previously generated self-signed pair with a CA-signed pair, `.stellarr-self-signed` is removed automatically.

1. Place `fullchain.pem` and `privkey.pem` in a directory next to `docker-compose.yml` (for example, `./certs/`).
2. Create `docker-compose.override.yml` in the same directory:

   ```yaml
   services:
     stellarr:
       volumes:
         - ./certs:/data/stellarr/tls:ro
   ```

3. Set `STELLARR_DOMAIN` in `.env` to match the certificate's hostname, then recreate the container:

   ```bash
   docker compose up -d
   ```

4. Verify the served certificate chain:

   ```bash
   echo | openssl s_client -connect chat.example.com:8443 -servername chat.example.com 2>/dev/null | grep -E 'issuer=|Verify return code'
   ```

   Confirm that `issuer=` shows your CA and the output ends with `Verify return code: 0 (ok)`.

If a mounted CA-signed certificate is unreadable or does not match `privkey.pem`, the container logs an error and exits with status `1`. If a mounted CA-signed certificate expires within 7 days, startup logs a warning.

> [!WARNING]
> Mount the directory (`./certs:/data/stellarr/tls:ro`), not individual files. Single-file bind mounts retain the original file inode when host tools atomically replace certificate files, preventing renewals from reaching the container. Also ensure the directory contains regular files rather than symlinks pointing outside the mount (for Certbot `live/`, copy files with `cp -L` in a deploy hook).

### Reloading after certificate renewal

After updating `fullchain.pem` and `privkey.pem` on the host, reload Nginx and restart `coturn` via `stellarr-init`:

```bash
docker compose exec stellarr stellarr-init reload nginx
docker compose exec stellarr stellarr-init restart turn
```

Run `docker compose exec stellarr stellarr-init status` to confirm both services report `ready`. See [Container services](../architecture.md#container-services-stellarr-init).

> [!NOTE]
> `coturn` does not reload TLS certificates on `SIGHUP`, so restarting `turn` drops active TURN relay allocations and causes relayed voice participants to briefly reconnect.

### TURN over TLS (`turns:`)

Setting `STELLARR_TURNS_ENABLED=true` enables TURN over TLS/DTLS on `STELLARR_TURNS_PORT` (`5349` by default) using `/data/stellarr/tls/fullchain.pem` and `privkey.pem`. Browsers reject self-signed certificates on `turns:` connections, so enable `STELLARR_TURNS_ENABLED` only when mounting a CA-signed certificate directly into the container.

## Run behind a reverse proxy

When an external reverse proxy terminates TLS on port `443` and forwards HTTP to the container's HTTP port (`8888` by default), configure `.env`:

```dotenv
STELLARR_DOMAIN=chat.example.com
STELLARR_PUBLIC_PORT=443
STELLARR_HTTP_PORT=127.0.0.1:8888
```

Binding `STELLARR_HTTP_PORT` to `127.0.0.1` prevents external clients from bypassing the reverse proxy on port `8888`.

### Proxy requirements

| Requirement | Reason |
| --- | --- |
| `X-Forwarded-Proto: https` | Prevents container Nginx from redirecting HTTP requests to `https://<host>:<STELLARR_HTTPS_PORT>/` and ensures secure cookies are issued. |
| Preserve `Host` (`$http_host`) | Used as the fallback origin when `STELLARR_PUBLIC_URL` is unset and for `/.well-known/stellarr/client` discovery. |
| Append client IP to `X-Forwarded-For` | Lets the API resolve real client IPs once the proxy is listed in `STELLARR_TRUSTED_PROXIES`. |
| Forward `Upgrade` and `Connection` headers | Required for `stellarr.v1` WebSocket connections under `/hubs/app` (keep `Origin` intact as well). |
| Allow request bodies up to `128 MiB` | Video uploads can be up to `100 MiB`; container Nginx and the API allow up to `128 MiB`. |
| Read timeout above `20 s` | The realtime hub sends keep-alives every `15 s` and WebSockets ping every `20 s`. |

### Example: Caddy

Caddy provisions and renews TLS certificates automatically, forwards WebSocket upgrades and `X-Forwarded-*` headers by default, and needs only a body-size ceiling for video uploads:

```txt
chat.example.com {
    request_body {
        max_size 128MB
    }
    reverse_proxy 127.0.0.1:8888
}
```

If Caddy runs in the same Docker network as Stellarr, proxy to `stellarr:80` instead of `127.0.0.1:8888`. Remember to list Caddy's connecting IP in `STELLARR_TRUSTED_PROXIES` (see [Client addresses](#client-addresses)).

### Example: Nginx

Place the `map` block in the `http` context:

```nginx
map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      close;
}

server {
    listen 443 ssl;
    server_name chat.example.com;
    ssl_certificate     /etc/ssl/chat.example.com/fullchain.pem;
    ssl_certificate_key /etc/ssl/chat.example.com/privkey.pem;
    client_max_body_size 128m;

    location / {
        proxy_pass http://127.0.0.1:8888;
        proxy_http_version 1.1;
        proxy_set_header Host $http_host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_read_timeout 3600s;
    }
}
```

If the reverse proxy runs in the same Docker network, proxy to `http://stellarr:80` instead of `http://127.0.0.1:8888`.

Verify the proxy setup by running `curl https://chat.example.com/health` (returns `{"status":"ready"}`) and confirming in browser DevTools that `/hubs/app` upgrades with HTTP `101 Switching Protocols`.

### Client addresses

Behind a reverse proxy, every TCP connection to the container originates from the proxy's IP address. Until you list that proxy in `STELLARR_TRUSTED_PROXIES`:

- Active sessions, security events, and trusted devices record the proxy's IP instead of each user's IP.
- Every user shares a single per-IP rate-limit bucket, allowing repeated failed logins from one user to rate-limit everyone.

Set `STELLARR_TRUSTED_PROXIES` to the IPv4/IPv6 addresses or CIDR networks of the proxies connecting to the container (comma- or space-separated):

```dotenv
STELLARR_TRUSTED_PROXIES=172.18.0.5
```

How client IP resolution works:

- Container Nginx appends the immediate peer address to `X-Forwarded-For`, and the API walks that header from **right to left**, skipping any address matched by `STELLARR_TRUSTED_PROXIES`. The first non-trusted IP becomes the client address.
- Your proxy must **append** the connecting client IP (as Nginx `$proxy_add_x_forwarded_for` does) rather than forwarding an untrusted `X-Forwarded-For` header unchanged. Spoofed headers sent by the client sit to the left and are ignored.
- If an upstream CDN or outer load balancer sits in front of your reverse proxy, include its CIDR ranges in `STELLARR_TRUSTED_PROXIES` as well.
- Invalid entries (hostnames, CIDR ranges with non-zero host bits such as `10.0.0.1/24`, or `0.0.0.0/0`) abort API startup.

#### Finding and pinning the proxy address

| Proxy placement | Address seen by the container |
| --- | --- |
| Container on the same Docker network (`http://stellarr:80`) | The proxy container's IP on that Docker network |
| Host process or container on another network connecting via `127.0.0.1:8888` | The gateway IP of Stellarr's Docker network |

To inspect the gateway IP of Compose's network:

```bash
docker network inspect stellarr_default --format '{{range .IPAM.Config}}{{.Gateway}}{{end}}'
```

Or check empirically: leave `STELLARR_TRUSTED_PROXIES` empty, sign in through the proxy, and run:

```bash
docker compose exec stellarr stellarr sessions list --user <your-username>
```

The newest session lists the connecting IP (`from 172.18.0.5`). Set `STELLARR_TRUSTED_PROXIES` to that address, run `docker compose up -d`, sign in again, and confirm the new session records your real client IP.

Because Docker assigns container IPs and network subnets dynamically across network recreations (`docker compose down`), assign a static `ipv4_address` to your proxy container or pin the Compose network subnet in `docker-compose.override.yml` (then run `docker compose down && docker compose up -d`):

```yaml
networks:
  default:
    ipam:
      config:
        - subnet: 172.30.0.0/24
```

> [!WARNING]
> Trust only the specific proxy IP whenever possible. Trusting the Docker bridge gateway trusts every connection arriving through Docker's host port forwarding, including other local processes or containers. On Docker Desktop (macOS and Windows), forwarded connections may arrive from the bridge gateway or `192.168.65.1`; avoid trusting broad gateway ranges on shared machines.

## What bypasses the HTTP reverse proxy

An HTTP reverse proxy forwards only HTTP and WebSocket traffic. WebRTC STUN, TURN, and SFU media packets (`UDP 49000`, `UDP 3478`, `UDP 49160-49200`, plus optional TCP `3478` and TCP/UDP `5349`) bypass the HTTP proxy and must reach the Docker host directly.

> [!IMPORTANT]
> Publish the WebRTC UDP and TURN ports on the Docker host and forward them on your router. `STELLARR_DOMAIN` and `STELLARR_PUBLIC_IP` must resolve to the host reachable on those ports, not an HTTP-only CDN proxy or tunnel. See the port tables in [Requirements](requirements.md#ports) and [RTC and ports](../rtc-and-ports.md#published-host-ports).
