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

The archive contains the SQLite database, uploaded objects, session keys, shared secrets, and TLS private key. Store it securely off-host.

> [!TIP]
> You can also create backups from Cockpit (**Backups**) while the instance is running. A database-only backup produces a single SQLite file; a full backup creates a `.zip` archive containing the database, uploaded objects, data-protection keys, and instance files (`secrets/`, `tls/`, desktop releases) with a `manifest.json`. Because Cockpit stores backups in `/data/stellarr/backups` on the same volume, download copies off-host and keep a copy of your `.env` file alongside them. To restore a Cockpit full backup, stage it with `stellarr backup restore` and restart the container; see [Restoring a backup](../admin/backups.md#restoring-a-backup).

## 2. Pull and start the new version

Release notes are published at <https://github.com/Arkazzae/Stellarr/releases>.

1. Update `STELLARR_VERSION` in `.env` (or keep `latest` to track the newest stable release):

   ```bash
   STELLARR_VERSION=1.2.3
   ```

2. Pull the image and recreate the container:

   ```bash
   docker compose pull
   docker compose up -d
   ```

   If `STELLARR_VERSION` references a tag that does not exist in GHCR, `docker compose` attempts a local build and fails if no `Dockerfile` is present.

3. Verify the upgrade:

   ```bash
   docker compose ps
   curl -sk -o /dev/null -w '%{http_code}\n' https://localhost:8443/login
   docker inspect --format '{{ index .Config.Labels "org.opencontainers.image.version" }}' stellarr
   ```

   Confirm that `docker compose ps` reports `healthy` (which checks both `/health` and `/login`) and `curl` returns `200`.

## Database schema

The API creates the SQLite database, in WAL mode, on the first start of a new data volume. Later starts only make sure its search index and triggers exist; they do not change the tables of an existing database.

> [!WARNING]
> Roll back by restoring the volume backup taken before the upgrade, not by starting an older container image against the same data volume.

## Rollback

Rolling back restores the entire data volume to the backup snapshot; any messages, uploads, or accounts created since the backup are lost.

1. Stop the stack and restore the archive:

   ```bash
   docker compose down
   docker run --rm -v stellarr_stellarr_data:/data -v "$PWD":/backup alpine \
     sh -c "find /data -mindepth 1 -delete && tar xzf /backup/stellarr-data-YYYY-MM-DD.tar.gz -C /data"
   ```

2. Set `STELLARR_VERSION` in `.env` to the previous version tag and start the container:

   ```bash
   docker compose up -d
   ```

Existing user sessions remain valid because the data-protection keys are restored with the archive.
