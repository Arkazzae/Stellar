# Upgrading

Upgrading replaces the container image while preserving the `stellarr_stellarr_data` volume.

> [!CAUTION]
> Never run `docker compose down -v` on a production instance. The `-v` flag deletes the data volume, including all messages, uploads, accounts, and secrets.

## 1. Back up the data volume

Always take a full volume backup while the container is stopped so SQLite (`stellarr.db`) and its WAL files are consistent.

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

The archive contains the SQLite database, uploaded objects, session keys, shared secrets, and TLS private key. Store it securely off-host alongside a copy of your `.env` file.

> [!TIP]
> You can also create full `.zip` backups from **Cockpit → Backups** while the instance is running (or automate them; see [Automatic backups](../admin/backups.md)). Download copies off-host. To restore a Cockpit archive later, use `stellarr backup restore` and restart the container.

## 2. Pull and start the new version

Release notes are published at <https://github.com/Arkazzae/Stellarr/releases>.

1. Update `STELLARR_VERSION` in `.env` (or keep `latest` to follow stable releases):

   ```dotenv
   STELLARR_VERSION=1.2.3
   ```

2. Pull the image and recreate the container:

   ```bash
   docker compose pull
   docker compose up -d
   ```

   If `STELLARR_VERSION` names a tag that does not exist in GHCR, `docker compose` attempts a local build and fails if no `Dockerfile` is present.

3. Verify the upgrade:

   ```bash
   docker compose ps
   curl -sk -o /dev/null -w '%{http_code}\n' https://localhost:8443/login
   docker inspect --format '{{ index .Config.Labels "org.opencontainers.image.version" }}' stellarr
   ```

   Confirm `docker compose ps` reports `healthy` (every internal service passes readiness checks) and `curl` returns `200`. If startup fails, inspect `docker compose exec stellarr stellarr-init status` and `docker compose logs`.

## Database schema

On a new volume, the API initializes SQLite in WAL mode from the data model. On subsequent starts and upgrades, startup applies idempotent schema preparation steps (such as auxiliary tables, full-text indexes, and triggers) while keeping existing servers, roles, memberships, messages, and settings intact.

> [!WARNING]
> Do not downgrade by starting an older image directly against an upgraded volume. Roll back by restoring the pre-upgrade volume backup instead.

## Rollback

Rolling back restores the entire data volume to the pre-upgrade snapshot; any messages, uploads, or accounts created after the backup are lost.

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

Existing user sessions remain valid because the data-protection keys are restored with the archive.
