# Compose services

Build the web, API, media and optional TURN images from one Stellarr checkout. Each container runs its service as PID 1 under UID 999 with all Linux capabilities dropped. These images are not published: the revision you check out is the version you run. For the prebuilt single-container release, use the [quick start](../../docs/install/quick-start.md).

| Service | Runtime and storage |
| --- | --- |
| `web` | Nginx serves the client and Cockpit, terminates TLS and proxies requests to the API. The certificate lives in `web_data`. |
| `api` | The Rust API owns the SQLite database and attachments in `data`. It shares the web network namespace so it can listen on loopback. The operator CLI ships in this image. |
| `media` | The WebRTC server publishes one port over UDP and TCP. The API reaches its authenticated control endpoint on the Compose network. |
| `turn` | Optional coturn relay. It shares the media network namespace to restrict relay peers to the media server. |
| `credentials` | Runs once at startup with no network and only the CHOWN capability. It creates or reuses the service keys. The consumers mount their keys read-only. |

## Start

Use Docker Engine 25+ with Compose 2.24+, and run every command from the checkout root. Compose reads the checkout's `.env` only when no `-f` option points it elsewhere, so name the Compose files in `.env` itself:

```bash
cp .env.example .env
echo 'COMPOSE_FILE=deploy/services/compose.yml' >> .env
docker compose up -d --build --wait
docker compose logs api | grep 'Setup code'
```

Set the public hostname and media address in the same `.env` if other devices will connect. [Address and port settings](../../docs/install/requirements.md#behind-nat) Then open `/setup` on your HTTPS address. The local default is `https://localhost/setup`. The first build compiles the API, the media server and the web client, so it takes a while and needs several gigabytes of disk for Docker's build cache.

If you pass `-f` options instead, add `--env-file .env` to every command. Without it, Compose ignores the settings in `.env` and starts the services with empty ones.

Compose names the project `stellarr-services`. Its default ports match the single-container deployment. To run both on one host, choose free values for `STELLARR_HTTP_PORT`, `STELLARR_HTTPS_PORT` and `STELLARR_MEDIA_PORT`.

**Existing installations using the former `deploy/split` files:** Keep the original project name so Docker reuses your database, web certificate and credential volumes. For the old default, add `COMPOSE_PROJECT_NAME=stellarr-split` to `.env` next to `COMPOSE_FILE`. If you set a custom project name, keep that name. Name the TURN overlay in `COMPOSE_FILE` if TURN was enabled, then run `docker compose up -d --build --wait`. The `credentials` job gives group 999 read access to the existing service keys. The old `turn_tls` volume is no longer used.

## Enable TURN

Name both files in `.env` and start the project again. Set `STELLARR_TURN_TCP_ENABLED=true` for TURN/TCP or `STELLARR_TURNS_ENABLED=true` for TLS. Forward the enabled [TURN ports](../../docs/rtc-and-ports.md#optional-turn-relay).

```bash
COMPOSE_FILE=deploy/services/compose.yml:deploy/services/turn.yml
```

To return to direct media, run `docker compose down` while `.env` still names both files. Then name only the base file and start again.

## Operate

- Run `docker compose ps` to see health and `docker compose logs api media` to read service logs.
- Run `docker compose exec api stellarr doctor` for the API. Restart the API to apply a staged backup restore.
- Web makes a self-signed certificate on first start. It checks that certificate only when it starts and renews it within 30 days of expiry, so restart web at least once every eleven months, or supply a trusted pair.
- To supply trusted TLS, bind-mount your certificate directory read-only at `/data/stellarr/tls` on web. Grant UID or GID 999 read access to the pair and traversal of that directory. Web checks the pair on startup. After renewal, restart web, then TURN if enabled. [TLS configuration](../../docs/install/reverse-proxy-and-tls.md)
- For an online backup or a move from the single-container deployment, use a full Cockpit backup and verify the restored messages and files before switching the public address. For a filesystem backup, stop the project and save its named volumes and `.env`.
- To upgrade, take a backup, check out the desired revision and run `docker compose up -d --build --wait` again.
