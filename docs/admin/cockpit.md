# Stellarr Cockpit

Stellarr Cockpit is the standalone web operator panel served at `/cockpit` on your instance origin (for example, `https://chat.example.com/cockpit`). It ships inside the same static web build as the chat client and shares its database settings and sealed secrets with the [`stellarr` CLI](cli.md).

## Access control and MFA enforcement

Every `/api/cockpit/*` endpoint enforces `InstanceOwner`—only the owner of the primary server created during `/setup` is admitted (all other accounts receive HTTP `403`).

| Condition | Requirement | Error code when unmet |
| --- | --- | --- |
| `mfa.requirement` is `cockpit`, `admins`, or `everyone` | Session must have passed a real second factor (trusted-device cookies do not qualify) | `403 cockpit-mfa-required` |
| Owner account has any 2FA method enrolled + sensitive write | Second factor completed within the **last 5 minutes** | `403 fresh-mfa-required` |

Sensitive mutations gated by the 5-minute fresh MFA window include creating or disabling accounts, editing roles, transferring or deleting servers, managing activities, modifying secrets or mail/MFA/backup/R2 policies, and downloading backup archives. Every Cockpit write records an `operator` audit log entry in the same SQLite transaction.

## Pages

| Page | Route | Capabilities |
| --- | --- | --- |
| **Overview** | `/cockpit` | Activity trends, API uptime, 60-minute API and SQLite availability bars, request/hub charts, and [`stellarr-init`](../architecture.md#container-services-stellarr-init) health for `media`, `api`, `turn`, `nginx`, and `tls-maintenance`. |
| **Users** | `/cockpit/users` | Create or disable accounts, inspect and revoke sessions, manage roles, trigger [MFA recovery](account-security.md#operator-recovery), and unlink OIDC identities (guarded against removing an account's last sign-in method). |
| **Servers** | `/cockpit/servers` | Instance-wide server directory with member/channel counts, ownership transfer, and server deletion. |
| **Runtime** | `/cockpit/runtime` | `/proc` process telemetry (requests, errors, CPU, RSS/mapped memory, threads, open hub sockets, active voice rooms, SQLite latency) and per-minute service availability. |
| **Logs** | `/cockpit/logs` | Searchable, filterable 10,000-line in-memory ring buffer (`info`, `warn`, `error`) with request trace IDs and TSV export. |
| **Activities** | `/cockpit/activities` | Install static multiplayer activities from GitHub repository/release URLs, review permissions and external hosts, enable/disable/update/uninstall entries, and issue dev preview pairing codes ([Activity SDK](../../sdk/README.md)). |
| **Backups** | `/cockpit/backups` | On-demand `.db` snapshots and full `.zip` archives, SHA-256 verification, archive downloads, and [automated backup schedules](backups.md). |
| **Integrations** | `/cockpit/integrations` | Resend email API, GIPHY toggle and key validation, Web Push VAPID status, per-provider OIDC invite sign-up toggles, and [Cloudflare R2](cloudflare-r2.md) (`s3` or `worker`) with connection tests. |
| **Settings** | `/cockpit/settings` | Instance name, time zone, file storage quota, retention windows (`backups-days`, `logs-days`, `orphans-days`) and hourly cleanup, SMTP server, [MFA policy](account-security.md#instance-mfa-policy-operators), desktop client version floors, and Cockpit UI locale (`en` / `pl`). |

## Sealed secrets and log redaction

### Secrets

Secrets saved in Cockpit (`mail.smtp-password`, `mail.resend-api-key`, `giphy.api-key`, `cloudflare.r2.access-key`, `cloudflare.r2.secret-key`, and `cloudflare.r2.worker-token`) are encrypted at rest with AES-256-GCM under keys derived from `/data/stellarr/keys/master.key`. Stored secrets override `.env` immediately without a restart and are never sent back to browsers; clearing a secret restores its `.env` fallback.

### Log ring-buffer filtering

Cockpit's **Logs** buffer retains up to `10,000` lines (`2 KiB` ceiling per line) and deliberately excludes three categories of sensitive or noisy output:

1. Routine `/health/ready` and `/health/live` probes (unless they log a warning or error).
2. Invite codes, password-reset/email-confirmation tokens, and unrecognized path segments on unmatched routes (masked before logging by `Stellarr.Api.Requests`).
3. Initial setup codes (`Stellarr.Api.Logging.StartupLog`) and object-store startup diagnostics (`Stellarr.Api.Logging.ObjectStore`), which write exclusively to container stdout/stderr.

## Interface language

Cockpit persists its UI locale (`en` or `pl`) in `localStorage` under `stellarr.cockpit.locale`, independent of both the chat client's user locale and `STELLARR_LANGUAGE`.
