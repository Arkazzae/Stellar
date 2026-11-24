# Cloudflare R2 offsite backups

Cloudflare R2 is an optional offsite destination for completed full backups. Configure it under **Cockpit → Integrations → Cloudflare R2**. Backup schedules and local retention remain under **Cockpit → Backups → Backup settings** (see [Automatic backups](backups.md)).

Stellarr supports two upload modes:

| Mode | How it works | Best for |
| --- | --- | --- |
| **Direct S3 API (`s3`)** | Container streams 64 MiB multipart uploads directly to the R2 S3 endpoint | Large instances and media-heavy archives (>100 MB) |
| **Backup Relay Worker (`worker`)** | Container streams the archive over HTTPS to the Cloudflare Worker in [`deploy/cloudflare/backup-relay`](../../deploy/cloudflare/backup-relay/README.md) | Small instances where you prefer not to manage R2 S3 API keys |

Neither mode requires a Cloudflare Tunnel, public bucket, custom domain, or CORS policy.

## Option A: Direct S3 mode

1. Create a **private, dedicated backup bucket** in Cloudflare R2 with public access disabled.
2. Create bucket-scoped R2 S3 API credentials with **Object Read & Write** permissions, separate from any other storage keys.
3. In **Cockpit → Integrations → Cloudflare R2**, select **R2 S3 API credentials**, save the access key ID and secret access key first, then enter the S3 endpoint URL, region (`auto`), bucket name, and object prefix. Enable **Send backup copies to Cloudflare R2** and save.

To supply defaults or credentials in `.env` instead (values saved in Cockpit override `.env`):

```dotenv
STELLARR_CLOUDFLARE_R2_ENABLED=true
STELLARR_CLOUDFLARE_R2_MODE=s3
STELLARR_CLOUDFLARE_R2_ENDPOINT=https://ACCOUNT_ID.r2.cloudflarestorage.com
STELLARR_CLOUDFLARE_R2_REGION=auto
STELLARR_CLOUDFLARE_R2_BUCKET=stellarr-backups
STELLARR_CLOUDFLARE_R2_ACCESS_KEY=YOUR_R2_ACCESS_KEY_ID
STELLARR_CLOUDFLARE_R2_SECRET_KEY=YOUR_R2_SECRET_ACCESS_KEY
STELLARR_CLOUDFLARE_R2_PREFIX=stellarr
```

Always use your bucket's R2 S3 API endpoint (including any EU/jurisdiction segment if applicable), never a public `r2.dev` URL.

## Option B: Backup Relay Worker

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/Arkazzae/Stellarr/tree/main/deploy/cloudflare/backup-relay)

The [Deploy to Cloudflare button](https://developers.cloudflare.com/workers/platform/deploy-buttons/) deploys **only the relay Worker**. Stellarr itself continues running in your Docker container.

1. Click **Deploy to Cloudflare**. Cloudflare clones [`deploy/cloudflare/backup-relay`](../../deploy/cloudflare/backup-relay/README.md), provisions the `BACKUPS` R2 bucket (`stellarr-backups`), and prompts for `BACKUP_TOKEN`.
2. Set `BACKUP_TOKEN` to a high-entropy random secret (for example, `openssl rand -hex 32`).
3. In **Cockpit → Integrations → Cloudflare R2**, select **Backup relay Worker**, enter the deployed Worker URL and `BACKUP_TOKEN`, keep the object prefix, enable offsite copies, and save.

Or configure in `.env`:

```dotenv
STELLARR_CLOUDFLARE_R2_ENABLED=true
STELLARR_CLOUDFLARE_R2_MODE=worker
STELLARR_CLOUDFLARE_R2_ENDPOINT=https://stellarr-backup-relay.ACCOUNT.workers.dev
STELLARR_CLOUDFLARE_R2_WORKER_TOKEN=YOUR_LONG_RANDOM_TOKEN
STELLARR_CLOUDFLARE_R2_PREFIX=stellarr
```

### Worker limits

- The Worker uploads each archive in a single `PUT /object` request. Cloudflare Workers enforce request body size limits (~100 MB on Free plans, ~500 MB on Paid plans). Switch to **Direct S3 mode** if your backup archives approach or exceed your plan's limit.
- The Worker exposes `GET /health` and `PUT`, `HEAD`, and `GET` on `/object`, rejecting conflicting SHA-256 overwrites with HTTP `409`. It intentionally omits list and delete endpoints; manage retention via R2 object lifecycle rules in the Cloudflare dashboard.

## Upload behavior, retention, and security

- **Object key layout**: `<PREFIX>/<SERVER_UUID>/backup-<TIMESTAMP>-<UUID>.zip`. Archives stream directly from disk without buffering the full `.zip` in memory.
- **Post-upload verification**: After uploading, Stellarr sends a `HEAD` request to confirm the remote object size and stored `sha256` metadata. Periodically download an archive and verify it with `stellarr backup verify`.
- **Retention**: Local archives are pruned according to Cockpit's backup retention setting (`retention.backups-days`, 14 days by default). Stellarr never deletes remote R2 objects; configure an R2 object lifecycle expiration rule (e.g. 90 days), incomplete-multipart abort rule, and optional bucket locks in Cloudflare.
- **Encryption**: Backup archives are encrypted in transit (HTTPS) and at rest by Cloudflare, not client-side encrypted before upload. Because full backups include `/data/stellarr/keys/`, `/data/stellarr/secrets/`, and `/data/stellarr/tls/privkey.pem`, restrict bucket access strictly or use an external client-side encrypted backup tool (such as `restic`) if your threat model requires zero-knowledge offsite storage.
- **Store `.env` separately**: Environment variables (`STELLARR_CLOUDFLARE_R2_WORKER_TOKEN`, R2 S3 keys, VAPID keys, and OIDC client secrets in `.env`) are never included in backup archives. Keep a copy of `.env` in your offline recovery store.

## References

- [Cloudflare R2 S3 API compatibility](https://developers.cloudflare.com/r2/api/s3/api/)
- [Cloudflare R2 object lifecycles](https://developers.cloudflare.com/r2/buckets/object-lifecycles/)
- [Cloudflare R2 bucket locks](https://developers.cloudflare.com/r2/buckets/bucket-locks/)
- [Cloudflare Workers request limits](https://developers.cloudflare.com/workers/platform/limits/)
