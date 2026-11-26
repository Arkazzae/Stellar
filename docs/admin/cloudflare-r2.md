# Cloudflare R2 offsite backups

Cloudflare R2 replicates completed full `.zip` backups off-host. Configure credentials under **Cockpit → Integrations → Cloudflare R2** while managing backup intervals and local retention under **Cockpit → Backups → Backup settings** ([Automatic backups](backups.md)).

## Upload modes

| Mode | Mechanism | Recommended when |
| --- | --- | --- |
| **Direct S3 API (`s3`)** | Streams `64 MiB` S3 SigV4 multipart uploads directly to your R2 endpoint | Archives approach or exceed `100 MB` (media-rich instances) |
| **Backup Relay Worker (`worker`)** | Streams a single `PUT /object` over HTTPS to [`deploy/cloudflare/backup-relay`](../../deploy/cloudflare/backup-relay/README.md) | Small archives where you prefer a bearer token over managing R2 S3 keys |

Neither mode uses public buckets, `r2.dev` URLs, CORS policies, or Cloudflare Tunnels.

---

## Option A: Direct S3 mode (`s3`)

1. Create a **private, dedicated R2 bucket** (e.g. `stellarr-backups`) with public access disabled.
2. Issue bucket-scoped R2 S3 API credentials with **Object Read & Write** permissions.
3. In **Cockpit → Integrations → Cloudflare R2**, choose **R2 S3 API credentials**, save the access key ID and secret key, enter your S3 endpoint URL (including jurisdiction segment such as `/eu` if applicable), region (`auto`), bucket name, and prefix (`stellarr`), then enable **Send backup copies to Cloudflare R2**.

Equivalent `.env` seed configuration (Cockpit-saved values take precedence):

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

---

## Option B: Backup Relay Worker (`worker`)

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/Arkazzae/Stellarr/tree/main/deploy/cloudflare/backup-relay)

*(Deploys only the lightweight relay Worker—Stellarr itself stays on your Docker host.)*

1. Click **Deploy to Cloudflare** to clone [`deploy/cloudflare/backup-relay`](../../deploy/cloudflare/backup-relay/README.md), bind the `stellarr-backups` R2 bucket, and prompt for `BACKUP_TOKEN`.
2. Generate a random token (`openssl rand -hex 32`) and save it as `BACKUP_TOKEN`.
3. In **Cockpit → Integrations → Cloudflare R2**, select **Backup relay Worker**, paste the Worker URL and `BACKUP_TOKEN`, enable offsite copies, and save.

Equivalent `.env` configuration:

```dotenv
STELLARR_CLOUDFLARE_R2_ENABLED=true
STELLARR_CLOUDFLARE_R2_MODE=worker
STELLARR_CLOUDFLARE_R2_ENDPOINT=https://stellarr-backup-relay.ACCOUNT.workers.dev
STELLARR_CLOUDFLARE_R2_WORKER_TOKEN=YOUR_LONG_RANDOM_TOKEN
STELLARR_CLOUDFLARE_R2_PREFIX=stellarr
```

> [!NOTE]
> Cloudflare Workers cap single-request bodies (~`100 MB` on Workers Free, ~`500 MB` on Workers Paid). Switch to **Direct S3 mode (`s3`)** before archives reach your Worker plan ceiling.

---

## Verification, retention, and security

| Topic | Behavior & operator action |
| --- | --- |
| **Object key layout** | `<prefix>/<server-uuid>/backup-<timestamp>-<uuid>.zip`, streamed from disk without buffering in RAM. |
| **Integrity check** | Every upload is followed by a `HEAD` check verifying remote byte length and stored `sha256` metadata; conflicting digests return `409` without overwriting existing archives. Periodically download an archive and run `stellarr backup verify`. |
| **Remote retention** | Stellarr prunes local archives (`retention.backups-days`, default `14` days) and **never deletes remote R2 objects**. Configure an R2 object lifecycle expiration rule (e.g. `90` days) and an incomplete-multipart abort rule in Cloudflare. |
| **At-rest secrets** | Archives are encrypted in transit (HTTPS) and at rest by Cloudflare, and contain `keys/master.key`, `secrets/`, and `tls/privkey.pem`. Restrict R2 credentials strictly, or encrypt `/data/stellarr` externally (e.g. `restic`) if zero-knowledge cloud storage is required. |
| **Offline `.env` copy** | `.env` credentials (R2 keys/token, VAPID keys, OIDC client secrets) are excluded from `.zip` archives—back up `.env` separately. |
