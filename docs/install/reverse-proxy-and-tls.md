# Reverse proxy and TLS

By default, the container terminates TLS itself using an auto-generated self-signed certificate. You can either mount your own CA-signed certificate into the container or terminate TLS on an external reverse proxy.

## Mount a custom TLS certificate

Both Nginx and `coturn` read the certificate and private key from `/data/stellarr/tls/`:

| Container path | Contents |
| --- | --- |
| `/data/stellarr/tls/fullchain.pem` | PEM leaf certificate followed by intermediate certificates |
| `/data/stellarr/tls/privkey.pem` | PEM private key |

CA-signed certificates are never overwritten by the container. Self-signed certificates are treated as container-managed and replaced when expiring within 30 days or when `STELLARR_DOMAIN` changes.

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

If the certificate is unreadable or does not match `privkey.pem`, the container logs an error and exits with status `1`. If a mounted CA-signed certificate expires within 7 days, the container logs a warning on startup.

> [!WARNING]
> Mount the directory (`./certs:/data/stellarr/tls:ro`), not individual files. Single-file bind mounts keep the original inode across atomic replacements on the host, preventing renewals from reaching the container. Also ensure the directory holds regular files rather than symlinks pointing outside the mount (for Certbot `live/`, copy with `cp -L`).

### Reloading after certificate renewal

After updating `fullchain.pem` and `privkey.pem` on the host, reload Nginx and restart `coturn` through `stellarr-init`:

```bash
docker compose exec stellarr stellarr-init reload nginx
docker compose exec stellarr stellarr-init restart turn
```

`stellarr-init status` then reports both as `ready`. See [Container services](../architecture.md#container-services-stellarr-init).

> [!NOTE]
> Restarting `turn` drops active TURN relay allocations, causing relayed voice participants to briefly reconnect.

### TURN over TLS (`turns:`)

Setting `STELLARR_TURNS_ENABLED=true` enables TURN over TLS/DTLS on `STELLARR_TURNS_PORT` (`5349` by default) using the same certificate pair. Browsers reject self-signed certificates on `turns:` connections, so enable it only with a CA-signed certificate.

## Run behind a reverse proxy

When an external reverse proxy terminates TLS on port `443` and forwards HTTP to the container's HTTP port (`8888` by default), set in `.env`:

```dotenv
STELLARR_DOMAIN=chat.example.com
STELLARR_PUBLIC_PORT=443
STELLARR_HTTP_PORT=127.0.0.1:8888
```

Binding `STELLARR_HTTP_PORT` to `127.0.0.1` prevents external clients from bypassing the proxy on port `8888`.

### Proxy requirements

| Requirement | Reason |
| --- | --- |
| `X-Forwarded-Proto: https` | Without this header, container Nginx redirects HTTP requests to `https://<host>:<STELLARR_HTTPS_PORT>/`. |
| Preserve `Host` (`$http_host`) | Used as the fallback origin when `STELLARR_PUBLIC_URL` is unset. |
| Append client IP to `X-Forwarded-For` | Lets the API resolve real client IPs once the proxy is listed in `STELLARR_TRUSTED_PROXIES`. |
| Forward `Upgrade` and `Connection` headers | Required for SignalR WebSocket connections under `/hubs/app`. |
| Allow request bodies up to `128 MiB` | Video uploads can be up to `100 MiB`; container Nginx allows up to `128 MiB`. |
| Read timeout above `20 s` | SignalR sends keep-alives every `15 s` and WebSockets ping every `20 s`. |

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

Verify the setup by checking `curl https://chat.example.com/health` (returns `{"status":"ready"}`) and confirming in browser DevTools that `/hubs/app` upgrades with status `101 Switching Protocols`.

### Client addresses

Behind a proxy, every request reaches Stellarr from the proxy's address. Until you list that proxy in `STELLARR_TRUSTED_PROXIES`:

- Active sessions, security events, and trusted devices all show the proxy's IP instead of each user's IP.
- Every user shares the same per-IP rate-limit bucket, so one user failing sign-in repeatedly can lock out everyone else.

Configure `STELLARR_TRUSTED_PROXIES` with the IPv4/IPv6 addresses or CIDR networks of the proxies connecting to the container (comma- or space-separated):

```dotenv
STELLARR_TRUSTED_PROXIES=172.18.0.5
```

How Stellarr resolves the client IP:

- Container Nginx appends the connecting address to `X-Forwarded-For`, and the API walks that header from **right to left**, skipping any address listed in `STELLARR_TRUSTED_PROXIES`. The first unlisted IP becomes the client address.
- Your proxy must **append** the client IP it observed (as Nginx `$proxy_add_x_forwarded_for` does) rather than forwarding an untrusted header unchanged. Client-spoofed values sit to the left and are ignored.
- If a CDN or upstream proxy sits in front of your reverse proxy, list its CIDR ranges too; otherwise its edge IP is recorded as the client IP.
- Invalid entries (hostnames, non-zero host bits such as `10.0.0.1/24`, or `0.0.0.0/0`) abort API startup with an error naming the entry.

#### Finding and pinning the proxy address

| Proxy placement | Address seen by the container |
| --- | --- |
| Container on the same Docker network (`http://stellarr:80`) | The proxy container's IP on that network |
| Host process or container on another network hitting `127.0.0.1:8888` | The gateway IP of Stellarr's Docker network |

To inspect the gateway IP of Compose's network:

```bash
docker network inspect stellarr_default --format '{{range .IPAM.Config}}{{.Gateway}}{{end}}'
```

Or check empirically: leave `STELLARR_TRUSTED_PROXIES` empty, sign in through the proxy, and run:

```bash
docker compose exec stellarr stellarr sessions list --user <your-username>
```

Your newest session appears first and shows the connecting IP (`from 172.18.0.5`). Set `STELLARR_TRUSTED_PROXIES` to that address, run `docker compose up -d`, sign in again, and confirm the new session shows your real IP.

Because Docker assigns container IPs and network subnets dynamically across recreations (`docker compose down` / container updates), give the proxy container a static `ipv4_address` or pin the Compose network subnet in `docker-compose.override.yml` (then run `docker compose down && docker compose up -d`):

```yaml
networks:
  default:
    ipam:
      config:
        - subnet: 172.30.0.0/24
```

> [!WARNING]
> Only trust the exact proxy address when possible. Listing the Docker network gateway trusts every connection arriving through Docker's host forwarding, including other local programs or containers on different networks. `STELLARR_HTTP_PORT` can be bound to `127.0.0.1`, whereas `STELLARR_HTTPS_PORT` binds all host interfaces. On Docker Desktop (macOS and Windows), forwarded connections may arrive from either the gateway or `192.168.65.1`, so avoid trusting broad gateway ranges on shared machines.

## What does not go through the proxy

An HTTP reverse proxy only handles HTTP and WebSocket traffic. WebRTC STUN, TURN, and media server traffic use UDP (and dedicated TURN TCP/TLS listeners) and must connect directly to the Docker host:

| Default port | Protocol | Purpose |
| --- | --- | --- |
| `49000` | UDP | Stellarr Media, which carries every call (`STELLARR_MEDIA_PORT`) |
| `3478` | UDP | STUN and TURN |
| `49160-49200` | UDP | TURN relay ports |
| `3478` | TCP | TURN over TCP (`STELLARR_TURN_TCP_ENABLED=true`) |
| `5349` | TCP, UDP | TURN over TLS/DTLS (`STELLARR_TURNS_ENABLED=true`) |

> [!IMPORTANT]
> Publish these UDP/TURN ports on the Docker host and forward them on your router. `STELLARR_DOMAIN` and `PUBLIC_IP` must resolve to the host reachable on these ports, not an HTTP-only CDN or tunnel. See [Requirements](requirements.md) and [RTC and ports](../rtc-and-ports.md).
