# Automatic backups

Production Docker Compose runs an automatic full backup every 24 hours, starting five minutes after startup. Configure the schedule and retention in **Cockpit → Backups → Backup settings**. To keep offsite copies, enable [Cloudflare R2](cloudflare-r2.md) under **Cockpit → Integrations**.

## What a backup includes

Cockpit creates two kinds of backups:

- **Database-only backup**: A single SQLite online snapshot (`.db`).
- **Full backup**: A `.zip` archive with a `manifest.json` and four parts:
  1. SQLite online snapshot (`stellarr.db`)
  2. Uploaded objects (`objects/`)
  3. Data-protection keys (`keys/`)
  4. Instance files (`secrets/`, `tls/`, `releases/`)

> [!IMPORTANT]
> Automatic backups always create **full backups** and fail if any referenced upload is missing. Because `.env` is never stored inside archives, keep a separate copy of `.env` alongside your backups.

## Schedule and configuration

- **Cockpit**: Changes under **Backups → Backup settings** apply within one minute without a restart. Updating the policy requires a fresh second factor if MFA is enabled on the owner account. Use **Restore deployment defaults** to fall back to `.env`.
- **Environment defaults**: Set defaults in `.env` and run `docker compose up -d`. The container turns automatic backups on unless `STELLARR_BACKUP_ENABLED=false`; the API started with `dotnet run` outside the container leaves them off.

```dotenv
STELLARR_BACKUP_ENABLED=true
STELLARR_BACKUP_INTERVAL_HOURS=24
STELLARR_BACKUP_RETRY_MINUTES=15
STELLARR_BACKUP_INITIAL_DELAY_MINUTES=5
```

How the scheduler works:

- Intervals run in UTC from the **last successful completion**, not on a wall-clock cron. A missed run triggers once after restart without catching up skipped cycles.
- Manual and scheduled backups share one lock, so a manual run defers the scheduled job. Automatic entries appear in the audit log under the owner account tagged `[automatic]`.
- With an offsite destination enabled, a cycle succeeds only after the offsite upload also finishes. Failed uploads retry the same local `.zip` at the retry interval (tracked in `${DataRoot}/automatic-backup.json`).
- Stellarr does not send email alerts on backup failure. Monitor **Cockpit → Backups** (and container logs) or watch completion timestamps externally.
- Run only one API process per `/data/stellarr` volume so the process-local lock stays exclusive.

## Restoring a backup

The [operator CLI](cli.md) restores **full backups** automatically. Test restores periodically on an isolated instance before relying on them.

1. Copy the archive into `/data/stellarr/backups/` if Cockpit does not already list it. For archives downloaded from R2, verify the SHA-256 against the remote `sha256` metadata first:

   ```bash
   docker compose cp ./backup-20261114-030000-<id>.zip stellarr:/data/stellarr/backups/
   ```

2. Stage the restore. The CLI checks `manifest.json`, entry paths, SQLite integrity, and every entry's CRC-32 and size, then copies the archive to `restore/staged.zip`:

   ```bash
   docker compose exec stellarr stellarr backup restore backup-20261114-030000-<id>.zip --yes
   ```

3. Restart the container. Before services start, the entrypoint replaces `db/`, `files/`, `keys/`, `secrets/`, `tls/`, and `releases/`, moving the old folders to `restore/previous-<time>/`:

   ```bash
   docker compose restart stellarr
   ```

   (`logs/`, `backups/`, and `automatic-backup.json` are preserved.)

4. Verify owner login, message history, downloads, avatars, integrations, and creating a fresh backup. Delete `restore/previous-<time>/` once verified; moving those folders back undoes the restore.

### Restore safeguards

| Behavior | Detail |
| --- | --- |
| Status record | `restore/last-restore.json` logs what the last container start did. |
| Cancel before restart | Run `docker compose exec stellarr stellarr backup cancel`. |
| Corrupted staged archive | Live data stays untouched, the archive moves to `restore/refused-<time>.zip`, and the container boots normally. |
| Interrupted folder swap | If rollback also fails, startup creates `restore/broken` and refuses to boot until you repair the volume and delete that marker. |
| Disk space | `/data/stellarr` needs room for live data, `restore/staged.zip`, and unpacked files at once. `previous-*` and `refused-*` files are not auto-pruned. |
| Sub-mounts | Automatic restore refuses if subdirectories such as `files/` are separate mounts. Restore manually instead. |
| Version match | Restore onto the same Stellarr release that wrote the backup. |

### Restoring a partial or database-only backup

A standalone `.db` file or an archive missing any of the four full-backup parts must be restored manually with the container stopped:

1. Stop the container (`docker compose stop`) and keep a copy of `/data/stellarr`.
2. Place the `.db` file at `db/stellarr.db` and remove any leftover `db/stellarr.db-wal` or `db/stellarr.db-shm`.
3. Copy `keys/` alongside the database so encrypted TOTP seeds and Cockpit secrets remain readable.
4. Copy `objects/` into `files/`, keeping relative paths.
5. Copy `instance/` into `/data/stellarr/`, keeping relative paths, and keep `secrets/` and `tls/` readable by owner only.
6. Start the container (`docker compose start`) and verify data and login.

## Consistency and recovery expectations

- SQLite uses an online snapshot taken before listing objects. Files uploaded mid-backup are not covered by a cross-system transaction, and missing referenced objects are recorded in `manifest.json` (and rejected for automatic backups).
- Entry CRC-32 and uncompressed sizes are checked both during staging and while unpacking at startup.
- A 24-hour interval gives roughly a 24-hour maximum data-loss window (plus backup duration). Measure actual recovery time during a test drill.
