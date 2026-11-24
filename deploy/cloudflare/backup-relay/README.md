# Stellarr backup relay Worker

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/Arkazzae/Stellarr/tree/main/deploy/cloudflare/backup-relay)

Optional Cloudflare Worker that receives completed Stellarr backup archives over HTTPS and writes them to a private Cloudflare R2 bucket via a native Worker R2 binding. Use this deployment mode when you want Cloudflare to provision the bucket automatically without creating R2 S3 API keys.

Stellarr itself continues to run in its Docker container on your host; this Worker serves strictly as an authenticated offsite backup target.

## One-click deployment

1. Click **Deploy to Cloudflare**. Cloudflare clones this directory into your GitHub or GitLab account, reads `wrangler.jsonc`, provisions the R2 bucket bound as `BACKUPS`, and prompts for `BACKUP_TOKEN`.
2. Generate a random token (for example, `openssl rand -hex 32`) and save it as `BACKUP_TOKEN`.
3. Once deployed, copy the Worker URL (for example, `https://stellarr-backup-relay.<account>.workers.dev`).
4. In Stellarr, open **Cockpit → Integrations → Cloudflare R2**, select **Backup relay Worker**, enter the Worker URL and bearer token, verify the object prefix, enable offsite copies, and save.

The R2 bucket name defaults to `stellarr-backups`. Edit `r2_buckets[0].bucket_name` in `wrangler.jsonc` before deploying if you prefer a different bucket name.

## Manual deployment with Wrangler

```sh
npm install
npx wrangler r2 bucket create stellarr-backups
npx wrangler secret put BACKUP_TOKEN
npm run deploy
```

For local testing, copy `.dev.vars.example` to `.dev.vars`, set `BACKUP_TOKEN`, and run `npm run dev`.

## HTTP protocol

All `/object` endpoints require `Authorization: Bearer <BACKUP_TOKEN>`.

| Method | Path | Description |
| --- | --- | --- |
| `GET` | `/health` | Unauthenticated liveness check (`200 OK`). |
| `PUT` | `/object` | Uploads a backup archive. Requires `X-Stellarr-Key` (object key) and `X-Stellarr-Sha256` (lowercase hex SHA-256 digest). |
| `HEAD` | `/object` | Returns `X-Stellarr-Size` and `X-Stellarr-Sha256` for post-upload verification by Stellarr. |
| `GET` | `/object` | Downloads an archive by `X-Stellarr-Key` (or `?key=`) for manual recovery. |

Uploading an existing object key with the same SHA-256 digest is idempotent and returns `200 OK` without rewriting the object. Uploading an existing key with a different digest returns `409 Conflict` so existing archives cannot be overwritten. The Worker exposes no list or delete endpoints; manage retention and lifecycle rules directly on the R2 bucket in the Cloudflare dashboard.

Object keys follow the `<prefix>/<server-uuid>/<backup-file>.zip` layout used by direct S3 uploads. Store `BACKUP_TOKEN` alongside your offline `.env` recovery records and rotate it if compromised.

## Verification

```sh
npm run check   # TypeScript type-check (tsc --noEmit)
npm run deploy  # wrangler deploy
```

This directory is intentionally self-contained with its own `package.json` because Cloudflare's Deploy button clones the subdirectory independently of the root workspace.
