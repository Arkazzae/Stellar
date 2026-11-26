# Split Docker Compose deployment

Use Docker Engine 25+ and Compose 2.24+. Run the Compose commands from the root of a Stellarr checkout. Compose starts the release image as three services. Only the API opens the instance database.

| Service | Process and data |
| --- | --- |
| <img src="icons/web.png" width="28" alt=""> `web` | Nginx serves Stellarr and Cockpit, terminates TLS, and proxies to the API on loopback. |
| <img src="icons/api.png" width="28" alt=""> `api` | The Rust API runs the workers and owns the `data` volume. Run the operator CLI here. |
| <img src="icons/media.png" width="28" alt=""> `media` | The WebRTC server publishes the media port over UDP and TCP. The API reaches its authenticated control endpoint on the project's bridge. |
| <img src="icons/turn.png" width="28" alt=""> `turn` | The optional coturn service shares the media network namespace so its peer allowlist follows the media server's address. |

Unraid reads each container's `net.unraid.docker.icon` label. For other dashboards, the PNGs are in [icons/](icons/).

## Start a new instance

```bash
git clone https://github.com/Arkazzae/Stellarr.git
cd Stellarr
docker compose -f deploy/split/compose.yml up -d --wait
docker compose -f deploy/split/compose.yml logs api | grep 'Setup code'
```

Open `https://localhost/setup`. For access from another device, set the public domain and media address in the checkout's `.env`, configure trusted HTTPS, and forward the [public ports](../../docs/install/requirements.md#behind-nat). To select a release, set `STELLARR_VERSION` in that file.

The split stack uses the project name `stellarr-split` and its own volumes. It uses the same default host ports as the single-container stack. To run both on one host, select free values for `STELLARR_HTTP_PORT`, `STELLARR_HTTPS_PORT`, and `STELLARR_MEDIA_PORT`. To move an existing instance, take a full Cockpit backup, restore it into the new API, and verify the messages and files before switching the public address.

## Enable TURN

Include both files on each Compose command when the instance needs a relay. The overlay enables TURN in the API and publishes its listener and relay ports.

```bash
docker compose -f deploy/split/compose.yml -f deploy/split/turn.yml up -d --wait
```

Set `STELLARR_TURN_TCP_ENABLED=true` for TURN/TCP or `STELLARR_TURNS_ENABLED=true` for TURN over TLS. Forward the corresponding [TURN ports](../../docs/rtc-and-ports.md#optional-turn-relay). To switch back to direct media only, stop this project with both files, then start it with the base file.

## Operate the instance

- Read per-service status and logs with `docker compose -f deploy/split/compose.yml ps` and `docker compose -f deploy/split/compose.yml logs api media`. The Cockpit process view can see the API container's processes.
- Use `docker compose -f deploy/split/compose.yml exec --user stellarr api stellarr doctor` for the API. After staging a full backup restore, restart the API service to apply it.
- For a running instance, use **Cockpit → Backups** to download a full application archive. For a filesystem copy, stop the project, save its named volumes and the checkout's `.env`, then start it again. The `web_data` volume holds the web certificate. Each service credential uses its own volume.
- For a certificate you manage, mount the directory containing `fullchain.pem` and `privkey.pem` read-only at `/data/stellarr/tls` on **web**. Web validates and copies the pair at startup. After replacing it, restart web and then turn if enabled. The local self-signed pair is also checked at web startup. [TLS configuration](../../docs/install/reverse-proxy-and-tls.md)
- To update, make a backup, pull the desired release and run `up -d --wait` with the same Compose files. Keep the checkout with the deployment because the services mount its entrypoint.
