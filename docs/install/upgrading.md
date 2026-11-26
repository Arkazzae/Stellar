# Upgrading

Upgrading swaps the stateless container image while keeping all state in the `stellarr_stellarr_data` volume (`/data/stellarr`).

> [!CAUTION]
> Never pass `-v` (`docker compose down -v`) when stopping or recreating an instance—it deletes `stellarr_stellarr_data` along with all accounts, messages, files, keys, and secrets.

## 1. Back up `/data/stellarr`

Archive the volume while the container is stopped so SQLite (`db/stellarr.db`) and its WAL files are cleanly checkpointed:

```bash
# Record the currently running image version
docker inspect --format '{{ index .Config.Labels "org.opencontainers.image.version" }}' stellarr

# Stop the container and create an off-host tarball
docker compose stop
docker run --rm -v stellarr_stellarr_data:/data -v "$PWD":/backup alpine \
  tar czf /backup/stellarr-data-$(date +%F).tar.gz -C /data .
```

Keep a copy of `.env` beside the archive. You can also download live full `.zip` archives from **Cockpit → Backups** (see [Automatic backups](../admin/backups.md)).

### Encrypted DM history

Allow disk space for a one-time SQLite rebuild. Keep a separate copy of `/data/stellarr/pin-recovery/master.key` when using full ZIP backups. After upgrading, ask members with encrypted DMs to open an updated client, complete the PIN migration, save their generated recovery code, and compare the new safety code with verified contacts. A member can migrate from an unlocked browser or sign in on a new device and use the old PIN once. Take a new full backup after migrations. Pre-migration volume copies and ZIPs may still expose the earlier history. [Account key backups](../admin/backups.md#save-the-instance-recovery-key-separately)

## 2. Pull and start the new release

### Option A: Installer-managed instances (`install.sh`)

Re-run the installer to update `docker-compose.yml` and `STELLARR_VERSION` while preserving your `.env` customizations:

```bash
curl -fsSL https://raw.githubusercontent.com/Arkazzae/Stellarr/main/install.sh | bash
```

Pass `--version 1.2.3` (with `--yes` for unattended scripts) to pin an exact version. Avoid `--force` unless you intentionally want `.env` backed up and regenerated from defaults.

### Option B: Manual Docker Compose

1. Set `STELLARR_VERSION` in `.env` (or keep `latest`), review [Port changes](#port-changes), and recreate the stack (include `-f infra/compose/turn.yml` if TURN is enabled):

   ```bash
   docker compose pull
   docker compose up -d
   ```

2. Verify container readiness and release version:

   ```bash
   docker compose ps
   curl -sk -o /dev/null -w '%{http_code}\n' https://localhost/login
   docker inspect --format '{{ index .Config.Labels "org.opencontainers.image.version" }}' stellarr
   ```

   Confirm `docker compose ps` reports `healthy` and `curl` returns `200`. Open browser tabs poll `/release.json` and reload into the updated web client shell automatically.

## Port changes

- **Media transports:** Ensure `docker-compose.yml` publishes both `49000/udp` and `49000/tcp` (or `STELLARR_MEDIA_PORT`) and that your router forwards both protocols to the same port.
- **Loopback HTTP upstream:** Container port `80` binds to `127.0.0.1:8888` by default for local [reverse proxies](reverse-proxy-and-tls.md#run-behind-a-reverse-proxy). Re-run `install.sh --http-port 127.0.0.1:8888` to lock down an older public `8888` binding.
- **TURN overlay:** `install.sh` preserves existing TURN settings (pass `--disable-turn` to switch to direct SFU media only). Manual Compose deployments using TURN must pass [the TURN override](../rtc-and-ports.md#optional-turn-relay) on every `pull` and `up`.

## Database schema and rollbacks

> [!WARNING]
> Never downgrade by booting an older image directly against an upgraded volume—the API and CLI reject mismatched schemas (`schema_incompatible`). Always roll back by restoring your pre-upgrade backup.

To roll back to a pre-upgrade volume tarball (discarding writes made after the snapshot):

```bash
docker compose down
docker run --rm -v stellarr_stellarr_data:/data -v "$PWD":/backup alpine \
  sh -c "find /data -mindepth 1 -delete && tar xzf /backup/stellarr-data-YYYY-MM-DD.tar.gz -C /data"
# Set STELLARR_VERSION back to the previous tag in .env, then start:
docker compose up -d
```

| Recovery method | Active sessions & trusted devices | When to use |
| --- | --- | --- |
| **Stopped volume tarball** (`tar xzf`) | Preserved verbatim (`db/stellarr.db` + `keys/master.key` restored as-is) | Immediate rollback after a failed container upgrade |
| **Cockpit full `.zip` restore** (`stellarr backup restore`) | Invalidated on startup so post-backup tokens cannot replay ([details](../admin/backups.md#restoring-a-backup)) | Point-in-time disaster recovery or host migration |
