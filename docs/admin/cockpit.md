# Stellarr Cockpit

Stellarr Cockpit is the web operator panel served at `/cockpit` on the instance origin (for example, `https://chat.example.com/cockpit`). It compiles into the same static web build as the chat client and shares its settings and sealed secrets with the [`stellarr` CLI](cli.md).

## Access and MFA rules

Every `/api/cockpit/*` route checks `InstanceOwner` and admits only the owner of the primary server created during `/setup` (all other accounts receive HTTP `403`).

When `mfa.requirement` is `cockpit`, `admins`, or `everyone`, Cockpit requires a session that passed a real second step (`cockpit-mfa-required`); trusted-device cookies do not qualify. Whenever the owner account has a second factor enrolled, sensitive mutations (creating or disabling accounts, changing roles, transferring or deleting servers, editing secrets or mail/MFA/backup/R2 settings, and downloading backups) require a second step completed within the last 5 minutes (`fresh-mfa-required`). Every Cockpit write records an operator audit entry in the same SQLite transaction.

## Pages

| Page | Path | Controls and data |
| --- | --- | --- |
| Overview | `/cockpit` | Activity trends, API uptime, 60-minute API and SQLite availability bars, request/hub charts, and [`stellarr-init`](../architecture.md#container-services-stellarr-init) status for `media`, `api`, `turn`, `nginx`, and `tls-maintenance`. |
| Users | `/cockpit/users` | Account creation and disablement, active session revocation, role assignments, [MFA recovery](account-security.md#operator-recovery), and OIDC unlinking (guarded against removing an account's last sign-in method). |
| Servers | `/cockpit/servers` | Server list, member and channel counts, ownership transfer, and server deletion. |
| Runtime | `/cockpit/runtime` | `/proc` process metrics (requests, errors, CPU, RSS and mapped memory, threads, open hub sockets, active voice rooms, SQLite latency) and per-minute service availability. |
| Logs | `/cockpit/logs` | Searchable 10,000-line in-memory ring buffer (`info`, `warn`, `error`) with request trace IDs and TSV export. |
| Backups | `/cockpit/backups` | On-demand `.db` snapshots and full `.zip` archives, SHA-256 verification, archive downloads, and [backup schedules](backups.md). |
| Integrations | `/cockpit/integrations` | Resend mail API, GIPHY search toggle and key check, per-provider OIDC invite sign-up switches, and [Cloudflare R2](cloudflare-r2.md) (`s3` or `worker`) with connection tests. |
| Settings | `/cockpit/settings` | Instance name, time zone, retention (`backups-days`, `logs-days`, `orphans-days`) and hourly cleanup, SMTP transport, [MFA policy](account-security.md#instance-mfa-policy-operators), desktop client version floors, and Cockpit's UI language (`en` or `pl`). |

## Secrets and log filtering

Secrets saved in Cockpit (`mail.smtp-password`, `mail.resend-api-key`, `giphy.api-key`, `cloudflare.r2.access-key`, `cloudflare.r2.secret-key`, and `cloudflare.r2.worker-token`) are sealed with AES-256-GCM under `/data/stellarr/keys/master.key`. Stored secrets override `.env` without a restart and are never returned to browsers; clearing one falls back to `.env`.

Cockpit's **Logs** buffer keeps up to `10,000` lines (`2 KiB` max per line) and omits three things:

1. `/health/ready` and `/health/live` probes unless they log a warning or error.
2. Secret path segments (invite codes, reset/confirmation tokens) and unknown words on unmatched URLs, which `Stellarr.Api.Requests` masks before logging.
3. Startup setup codes (`Stellarr.Api.Logging.StartupLog`) and object-store startup warnings (`Stellarr.Api.Logging.ObjectStore`), which go only to container stdout/stderr.

## Language

Cockpit saves its UI locale (`en` or `pl`) in `localStorage` (`stellarr.cockpit.locale`), separate from the chat client's language and `STELLARR_LANGUAGE`.
