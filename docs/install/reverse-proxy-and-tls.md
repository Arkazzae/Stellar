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
> Mount the directory (`./certs:/data/stellarr/tls:ro`), not individual files. Single-file bind mounts retain the original inode across atomic file replacements on the host, preventing renewals from propagating into the container. Also ensure the directory contains regular files rather than symlinks pointing outside the mount (such as Certbot `live/` symlinks; copy them with `cp -L`).

### Reloading after certificate renewal

After updating `fullchain.pem` and `privkey.pem` on the host, reload Nginx and restart `coturn` through the container's process manager:

```bash
docker compose exec stellarr stellarr-init reload nginx
docker compose exec stellarr stellarr-init restart turn
```

`stellarr-init status` then shows both as `ready`; see [Services in the container](platforms.md#services-in-the-container).

> [!NOTE]
> Restarting `turn` drops any active TURN relay allocations, causing relayed voice participants to briefly reconnect.

### TURN over TLS (`turns:`)

Setting `STELLARR_TURNS_ENABLED=true` enables TURN over TLS/DTLS on `STELLARR_TURNS_PORT` (`5349` by default) using the same certificate pair. Browsers reject self-signed certificates on `turns:` connections, so enable it only with a CA-signed certificate.

## Run behind a reverse proxy

When an external reverse proxy terminates TLS on port `443` and forwards HTTP to the container's HTTP port (`8888` by default), set in `.env`:

```bash
STELLARR_DOMAIN=chat.example.com
STELLARR_PUBLIC_PORT=443
STELLARR_HTTP_PORT=127.0.0.1:8888
```

Binding `STELLARR_HTTP_PORT` to `127.0.0.1` prevents external clients from bypassing the proxy on port `8888`.

### Proxy requirements

| Requirement | Reason |
| --- | --- |
| `X-Forwarded-Proto: https` | Without this header, container Nginx redirects HTTP requests to `https://<host>:<STELLARR_HTTPS_PORT>/`. |
| Preserve `Host` (`$http_host`) | Used to construct invite URLs and the discovery origin when `STELLARR_PUBLIC_URL` is unset. |
| Append the client's address to `X-Forwarded-For` | The API takes each person's address from it once the proxy is listed in `STELLARR_TRUSTED_PROXIES`; see [Client addresses](#client-addresses). |
| Forward `Upgrade` and `Connection` headers | Required for SignalR WebSocket connections under `/hubs/app`. |
| Allow request bodies up to `128 MiB` | Video uploads can be up to 100 MiB; container Nginx allows up to 128 MiB. |
| Read timeout above 20 seconds | SignalR sends a keep-alive every 15 seconds and WebSockets ping every 20 seconds. |

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

Behind a proxy, every request reaches the container from the proxy. Until the proxy is listed in `STELLARR_TRUSTED_PROXIES`, the API takes the proxy's address for everyone behind it: the session list, security events and trusted devices show the proxy, and everyone shares one budget of each per-address rate limit, so one person who tries to sign in ten times in a minute locks everyone else out of signing in for the rest of that minute.

List the proxy by the address the container sees it connect from:

```bash
STELLARR_TRUSTED_PROXIES=172.18.0.5
```

Container Nginx appends the address that connected to it to `X-Forwarded-For`, and the API reads the header from the right. As long as the address it has reached is a listed proxy, it moves to the entry before it, the address that proxy saw; the first address that is not a listed proxy is the client's. So the proxy has to end the header with the address it saw, as Nginx does with `$proxy_add_x_forwarded_for` in the example above, rather than pass on the header it received unchanged; check that your proxy does. Then:

- A header sent by anyone that is not a listed proxy changes nothing.
- Whatever a client writes into `X-Forwarded-For` itself sits before the address its proxy appended, so it is never read.
- When another proxy sits in front of yours, such as a CDN, its address is the one recorded unless it is listed too.

The value is a list of IPv4 and IPv6 addresses and CIDR networks, separated by commas or spaces. A host name, a network with bits set past its prefix (`10.0.0.1/24` instead of `10.0.0.0/24`) or one that covers every address stops the API at start, and the container log names the entry.

#### Which address to list

The address the container sees is not always the one the proxy has:

| Where the proxy runs | Address the container sees |
| --- | --- |
| In a container on the same Docker network as Stellarr, proxying to `http://stellarr:80` | The proxy container's address on that network |
| On the Docker host, proxying to the published port (`http://127.0.0.1:8888`) | The gateway of Stellarr's network |
| In a container on another Docker network, proxying to the published port on the host | The gateway of Stellarr's network |

The gateway of the network Docker Compose created for Stellarr:

```bash
docker network inspect stellarr_default --format '{{range .IPAM.Config}}{{.Gateway}}{{end}}'
```

To check, leave `STELLARR_TRUSTED_PROXIES` empty, sign in through the proxy and list your sessions:

```bash
docker compose exec stellarr stellarr sessions list --user <your-username>
```

The session you just opened is listed first, and ends with the address the container sees (`from 172.18.0.5`). Set `STELLARR_TRUSTED_PROXIES` to it, run `docker compose up -d`, sign in again, and the new session shows your own address.

Docker assigns these addresses when it creates a network or a container, so they can change: a proxy container's when it is recreated, a network's gateway when the network is (`docker compose down` removes it). Give the proxy container a fixed `ipv4_address`, or fix the subnet of Stellarr's network in `docker-compose.override.yml`, which makes its gateway the subnet's first address (`172.30.0.1` here). An existing network keeps its subnet until it is recreated, so run `docker compose down` and then `docker compose up -d` after adding it:

```yaml
networks:
  default:
    ipam:
      config:
        - subnet: 172.30.0.0/24
```

> [!WARNING]
> Every connection from a listed address is believed. The gateway is also where Docker makes other connections to the published ports come from: those from other programs on the host and from containers on other networks. Listing the gateway trusts them all, so keep the published ports out of their reach. `STELLARR_HTTP_PORT` can be bound to `127.0.0.1` as above, but `STELLARR_HTTPS_PORT` takes a port number only and stays published on every address of the host. Where that matters, put the proxy on Stellarr's Docker network and list its own address instead. Docker Desktop (macOS, Windows) forwards published ports itself, and connections arrive from the gateway or from `192.168.65.1` depending on the address they were made to, so listing either one there trusts far more than the proxy.

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
