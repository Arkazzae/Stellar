# Automatic backups

Production containers run an automatic full backup every 24 hours by default, starting 5 minutes after startup. Configure the schedule and local retention in **Cockpit → Backups → Backup settings**. To upload offsite copies automatically, configure [Cloudflare R2](cloudflare-r2.md) under **Cockpit → Integrations**.

## What a backup includes

Cockpit supports two backup formats:

- **Database-only backup**: A single online SQLite snapshot (`.db`).
- **Full backup**: A `.zip` archive containing `manifest.json` and four sections:
  1. Online SQLite snapshot (`stellarr.db`)
  2. Uploaded objects (`objects/`)
  3. Keys, including the master key that seals secrets (`keys/`)
  4. Instance files (`instance/`: `secrets/`, `tls/`, `releases/`)

> [!IMPORTANT]
> Automatic scheduled backups always produce **full backups** and fail if any referenced upload is missing on disk. Because `.env` is never stored inside backup archives, maintain a separate copy of `.env` in your recovery store.

## Schedule and configuration

- **Cockpit**: Changes saved under **Backups → Backup settings** take effect within one minute without a container restart. Modifying the backup policy requires a fresh second factor when MFA is enabled on the owner account. Click **Restore deployment defaults** to revert to `.env` values.
- **Environment defaults**: Set seed defaults in `.env` and apply with `docker compose up -d`. Production containers enable automatic backups unless `STELLARR_BACKUP_ENABLED=false`; running the API binary directly, outside the container, leaves automatic backups off by default.

```dotenv
STELLARR_BACKUP_ENABLED=true
STELLARR_BACKUP_INTERVAL_HOURS=24
STELLARR_BACKUP_RETRY_MINUTES=15
STELLARR_BACKUP_INITIAL_DELAY_MINUTES=5
```

Scheduler behavior:

- Intervals are measured in UTC from the **last successful completion**, not on a fixed wall-clock cron. If the container was stopped across a scheduled window, a single run triggers after startup (`STELLARR_BACKUP_INITIAL_DELAY_MINUTES`) without running multiple catch-up jobs.
- Manual and scheduled backups share an exclusive lock (`BackupJobs`), so a manual backup defers the next scheduled cycle. Automatic backup events are recorded in the audit log under the owner account with the `[automatic]` tag.
- When an offsite destination (Cloudflare R2) is enabled, a backup cycle counts as complete only after the offsite upload also succeeds. If the upload fails, the worker retries uploading the existing local `.zip` archive every `STELLARR_BACKUP_RETRY_MINUTES` (tracked in `/data/stellarr/automatic-backup.json`).
- Stellarr does not send email alerts on backup failure. Monitor **Cockpit → Backups** and container logs, or check backup completion timestamps externally.
- Run only one API container per `/data/stellarr` volume so the backup lock and SQLite WAL stay exclusive.

## Restoring a backup

The [operator CLI](cli.md) verifies and stages **full backups** for automated restoration on container startup. Test restores periodically on an isolated instance before relying on them in production.

1. Copy the `.zip` archive into `/data/stellarr/backups/` if it is not already present locally. For archives downloaded from Cloudflare R2, verify the SHA-256 checksum against the object's stored `sha256` metadata first:

   ```bash
   docker compose cp ./backup-20261114-030000-<id>.zip stellarr:/data/stellarr/backups/
   ```

2. Stage the restore. The CLI verifies `manifest.json`, entry paths, SQLite integrity, and every entry's CRC-32 and uncompressed size, then copies the archive to `/data/stellarr/restore/staged.zip`:

   ```bash
   docker compose exec stellarr stellarr backup restore backup-20261114-030000-<id>.zip --yes
   ```

3. Restart the container. Before starting any service or reading shared secrets, `stellarr-entrypoint` runs `stellarr backup apply --yes`, which swaps `db/`, `files/`, `keys/`, `secrets/`, `tls/`, and `releases/` into place and moves the previous directories into `/data/stellarr/restore/previous-<timestamp>/`:

   ```bash
   docker compose restart stellarr
   ```

   *(`logs/`, `backups/`, and `automatic-backup.json` are preserved across a restore.)*

4. Verify owner login, message history, attachments, avatars, integrations, and creating a new backup. Once verified, delete `/data/stellarr/restore/previous-<timestamp>/` to reclaim disk space; moving those folders back while the container is stopped undoes the restore.

### Restore safeguards

| Behavior | Detail |
| --- | --- |
| Status record | `/data/stellarr/restore/last-restore.json` records the outcome of the last startup restore attempt. |
| Cancel before restart | Run `docker compose exec stellarr stellarr backup cancel --yes` to remove `restore/staged.zip`. |
| Corrupted staged archive | Live data remains untouched, the archive is moved to `restore/refused-<timestamp>.zip`, and the container boots normally with its existing data. |
| Interrupted folder swap | If applying fails mid-swap and automatic rollback also fails, startup moves the archive to `restore/broken-<timestamp>.zip`, writes `restore/broken`, and refuses to boot until you repair `/data/stellarr` and remove `restore/broken`. |
| Disk space | `/data/stellarr` must have enough free space to hold the current live data, `restore/staged.zip`, and the unpacked archive simultaneously. `previous-*` and `refused-*` folders are never auto-pruned. |
| Sub-mounts | Automated restore refuses to run if subdirectories such as `files/` are separate mount points. Use the manual restore procedure below instead. |
| Version match | Restore onto the same Stellarr release that created the backup archive. |

### Restoring a partial or database-only backup

A standalone `.db` snapshot or an archive missing any of the four full-backup sections must be restored manually while the container is stopped:

1. Stop the container (`docker compose stop`) and back up the current contents of `/data/stellarr`.
2. Copy the `.db` snapshot to `/data/stellarr/db/stellarr.db` and delete any existing `db/stellarr.db-wal` and `db/stellarr.db-shm` files.
3. Restore `keys/` alongside `db/` so encrypted TOTP seeds and Cockpit secrets remain decryptable.
4. Copy `objects/` into `/data/stellarr/files/`, preserving relative paths.
5. Copy `instance/` into `/data/stellarr/`, preserving relative paths and `0600` owner-only permissions on `secrets/` and `tls/privkey.pem`.
6. Start the container (`docker compose start`) and verify health and sign-in.

## Consistency and recovery expectations

- SQLite backups use the online backup API before enumerating object storage files. Attachments uploaded while a backup is in progress are not covered by a cross-system snapshot; missing referenced objects are recorded in `manifest.json` (and cause automatic backups to fail and retry).
- Entry CRC-32 checksums and uncompressed byte lengths are verified both during `stellarr backup restore` staging and during `stellarr backup apply` extraction at startup.
- A 24-hour backup interval yields a maximum data-loss window (RPO) of roughly 24 hours plus backup duration. Measure your recovery time (RTO) during a test restore drill.
