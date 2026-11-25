# Upgrading

Upgrading replaces the container image while preserving the `stellarr_stellarr_data` volume (`/data/stellarr`).

After upgrading from a release that used SignalR, refresh any Stellarr tabs left open so they load the client for the `stellarr.v1` WebSocket protocol.

> [!CAUTION]
> Never run `docker compose down -v` on an active instance. The `-v` flag deletes the data volume, destroying all accounts, messages, uploads, encryption keys, and secrets.

## 1. Back up the data volume

Always take a full volume backup while the container is stopped so SQLite (`db/stellarr.db`) and its WAL files are quiescent.

1. Record the currently running image version:

   ```bash
   docker inspect --format '{{ index .Config.Labels "org.opencontainers.image.version" }}' stellarr
   ```

2. Stop the container and archive `stellarr_stellarr_data`:

   ```bash
   docker compose stop
   docker run --rm -v stellarr_stellarr_data:/data -v "$PWD":/backup alpine \
     tar czf /backup/stellarr-data-$(date +%F).tar.gz -C /data .
   ```

The archive contains the SQLite database, uploaded files, master key (`keys/master.key`), shared internal secrets, client releases, and TLS private key. Store it off-host alongside a copy of your `.env` file.

> [!TIP]
> You can also create full `.zip` backups from **Cockpit → Backups** while the instance is running (or automate them; see [Automatic backups](../admin/backups.md)). Download copies off-host. To restore a Cockpit `.zip` archive later, run `stellarr backup restore` and restart the container.

## 2. Pull and start the new version

Release notes are published at <https://github.com/Arkazzae/Stellarr/releases>.

### With the installer (`install.sh`)

If you installed Stellarr with `install.sh`, re-run it to update `docker-compose.yml` and `STELLARR_VERSION` in `.env` (preserving your other `.env` settings) and recreate the container:

```bash
curl -fsSL https://stellarr.online/install.sh | bash
```

To pin a specific release tag during an installer update, pass `--version 1.2.3` (or `--yes --version 1.2.3` for unattended execution). Pass `--force` only if you want the installer to back up `.env` and regenerate it from defaults and passed flags.

### With Docker Compose manually

1. Update `STELLARR_VERSION` in `.env` (or leave it unset / `latest` to follow the latest stable release):

   ```dotenv
   STELLARR_VERSION=1.2.3
   ```

2. Update the Compose files as described in [Port changes](#port-changes), then pull the image and recreate the container. If you use TURN, pass both Compose files to each command:

   ```bash
   docker compose pull
   docker compose up -d
   ```

   If `STELLARR_VERSION` names a tag that does not exist in GHCR, Compose reports a pull error.

3. Verify the upgrade:

   ```bash
   docker compose ps
   curl -sk -o /dev/null -w '%{http_code}\n' https://localhost/login
   docker inspect --format '{{ index .Config.Labels "org.opencontainers.image.version" }}' stellarr
   ```

   Replace the example URL with your configured HTTPS origin if it differs. Confirm `docker compose ps` reports `healthy` and `curl` returns `200`. Open browser tabs detect the new `/release.json` identifier and reload into the updated web client shell automatically. If startup fails, inspect `docker compose exec stellarr stellarr-init status` and `docker compose logs`.

## Port changes

Fresh installations publish HTTPS on `443/tcp` and media on `49000/udp` and `49000/tcp`. If you previously used a different HTTPS port, keep it in `STELLARR_HTTPS_PORT` before updating a manual Compose deployment. Use the HTTPS address reported by the installer or your configured public origin.

Replace your base Compose file when upgrading manually so Docker publishes both media transports. Add TCP forwarding on the same external media port as UDP. The bundled HTTP upstream defaults to loopback on port `8888`, which a local [reverse proxy](reverse-proxy-and-tls.md#run-behind-a-reverse-proxy) can use. The installer preserves an explicitly configured HTTP binding. To restrict a previously public port `8888` to loopback, rerun the installer with `--http-port 127.0.0.1:8888`.

The installer preserves the TURN profile of an existing installation. Rerun it with `--disable-turn` to switch an existing installation to direct media only. A manual Compose deployment that needs the relay must include [the optional TURN override](../rtc-and-ports.md#optional-turn-relay). Keep its existing relay ports mapped 1:1.

## Database schema

On a new volume, the API creates the SQLite database in WAL mode, with its full-text search indexes and triggers, in `db/stellarr.db`. A database that already has the schema is opened as it is, with its servers, roles, memberships, messages, and settings intact.

> [!WARNING]
> Do not downgrade by starting an older container image directly against an upgraded volume. The operator CLI and API refuse databases whose schema does not match the running binary. Roll back by restoring the pre-upgrade volume backup instead.

## Rollback

Rolling back restores the entire data volume to the pre-upgrade snapshot; any messages, uploads, or accounts created after the backup was taken are lost.

1. Stop the stack and restore the archive:

   ```bash
   docker compose down
   docker run --rm -v stellarr_stellarr_data:/data -v "$PWD":/backup alpine \
     sh -c "find /data -mindepth 1 -delete && tar xzf /backup/stellarr-data-YYYY-MM-DD.tar.gz -C /data"
   ```

2. Set `STELLARR_VERSION` in `.env` back to the previous version tag and start the container:

   ```bash
   docker compose up -d
   ```

Existing user sessions remain valid after a raw volume rollback because `db/stellarr.db` and the master key (`keys/master.key`) are restored verbatim from the tarball. (By contrast, restoring a Cockpit `.zip` archive via `stellarr backup restore` invalidates active sessions, trusted devices, and unused single-use codes before swapping directories; see [Automatic backups](../admin/backups.md#restoring-a-backup).)
