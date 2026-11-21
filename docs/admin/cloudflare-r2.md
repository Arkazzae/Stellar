# Cloudflare R2 offsite backups

Cloudflare R2 is an optional offsite destination for completed full backups. Configure it under **Cockpit → Integrations → Cloudflare R2**. Backup schedules and local retention stay under **Cockpit → Backups → Backup settings** (see [Automatic backups](backups.md)).

Stellarr supports two connection modes:

| Mode | How it works | Best for |
| --- | --- | --- |
| **Direct S3 API** | Container streams 64 MiB multipart uploads directly to R2 | Large instances and attachments (>100 MB archives) |
| **Backup Relay Worker** | Container streams to the Worker in [`deploy/cloudflare/backup-relay`](../../deploy/cloudflare/backup-relay/README.md) | Small instances without managing R2 S3 credentials |

Neither mode requires a Cloudflare Tunnel, public bucket, custom domain, or CORS rules.

## Option A: Direct S3 mode

1. Create a **private, dedicated backup bucket** in Cloudflare R2 with public access disabled.
2. Create bucket-scoped R2 S3 credentials with object read/write access, separate from any live storage keys.
3. In **Cockpit → Integrations → Cloudflare R2**, choose **R2 S3 API credentials**, save the access key and secret key first, then enter the endpoint, region (`auto`), bucket, and prefix. Turn on **Send backup copies to Cloudflare R2** and save.

To supply defaults or credentials in `.env` instead (Cockpit values override `.env`):

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

Use your bucket's R2 S3 endpoint (including any jurisdiction hostname), never an `r2.dev` public URL.

## Option B: Backup Relay Worker

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/Arkazzae/Stellarr/tree/main/deploy/cloudflare/backup-relay)

The [Deploy to Cloudflare button](https://developers.cloudflare.com/workers/platform/deploy-buttons/) deploys **only the relay Worker**. Stellarr itself continues running in your Docker container.

1. Click **Deploy to Cloudflare**. Cloudflare clones [`deploy/cloudflare/backup-relay`](../../deploy/cloudflare/backup-relay/README.md), provisions the `BACKUPS` R2 bucket, and prompts for `BACKUP_TOKEN`.
2. Set `BACKUP_TOKEN` to a strong random secret (for example, `openssl rand -hex 32`).
3. In **Cockpit → Integrations → Cloudflare R2**, choose **Backup relay Worker**, enter the Worker URL and `BACKUP_TOKEN`, keep the prefix, enable offsite copies, and save.

Or configure in `.env`:

```dotenv
STELLARR_CLOUDFLARE_R2_ENABLED=true
STELLARR_CLOUDFLARE_R2_MODE=worker
STELLARR_CLOUDFLARE_R2_ENDPOINT=https://stellarr-backup-relay.ACCOUNT.workers.dev
STELLARR_CLOUDFLARE_R2_WORKER_TOKEN=YOUR_LONG_RANDOM_TOKEN
STELLARR_CLOUDFLARE_R2_PREFIX=stellarr
```

### Worker limits

- The Worker uploads the whole archive in one `PUT /object` request. Cloudflare Workers limit request body size (about 100 MB on Free plans, 500 MB on Paid plans; verify current Cloudflare limits). Use **Direct S3 mode** if your archives exceed that.
- The Worker exposes `PUT`, `HEAD`, and `GET` on `/object` and rejects conflicting SHA-256 overwrites with `409`. It has no list or delete route, so configure lifecycle rules and bucket locks in Cloudflare.

## Upload behavior, retention, and security

- **Object names**: `<PREFIX>/<SERVER_UUID>/backup-<TIMESTAMP>-<UUID>.zip`. Archives stream from disk rather than loading into RAM.
- **Verification**: After upload, Stellarr sends a `HEAD` request to confirm file size and stored `sha256` metadata. Because this does not read back remote bytes, download and checksum a sample regularly.
- **Retention**: Local copies follow Cockpit retention (14 days by default). Stellarr never deletes remote R2 objects, so add an R2 lifecycle expiration rule (for example, 90 days) and optionally bucket locks plus incomplete-multipart cleanup.
- **Encryption**: Archives are encrypted in transit and at rest by Cloudflare, not client-side encrypted. They contain uploads, session keys, and encrypted Cockpit secrets. Wrap them with a client-side encrypted tool such as `restic` if your threat model requires keys outside Cloudflare.
- **Keep `.env` separately**: Environment variables (`STELLARR_CLOUDFLARE_R2_WORKER_TOKEN`, S3 keys, and other `.env` secrets) are excluded from backups. Keep `.env` in a separate recovery store.

## References

Cloudflare documentation reviewed on 2026-11-13:

- [S3 API compatibility](https://developers.cloudflare.com/r2/api/s3/api/)
- [.NET SDK settings and signing compatibility](https://developers.cloudflare.com/r2/examples/aws/aws-sdk-net/)
- [Object lifecycles](https://developers.cloudflare.com/r2/buckets/object-lifecycles/)
- [Bucket locks](https://developers.cloudflare.com/r2/buckets/bucket-locks/)
- [Upload methods and multipart behavior](https://developers.cloudflare.com/r2/objects/upload-objects/)
- [Deploy to Cloudflare buttons](https://developers.cloudflare.com/workers/platform/deploy-buttons/)
- [Workers limits](https://developers.cloudflare.com/workers/platform/limits/)
