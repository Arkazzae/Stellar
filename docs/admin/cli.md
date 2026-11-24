# Operator CLI

`stellarr` is the operator CLI bundled inside the container image (`/usr/local/bin/stellarr`). It inspects health, edits database-backed settings shared with Cockpit, manages encrypted secrets, revokes sessions, issues account MFA recovery credentials, and verifies or stages backup restores directly against `/data/stellarr` without needing Cockpit access or a running API process.

Anyone with `docker exec` access or read/write access to `/data/stellarr` controls the instance. Every CLI mutation records an `OperatorAuditEntries` row under the actor `local-cli` in the same SQLite transaction.

```bash
docker compose exec stellarr stellarr doctor
docker compose exec stellarr stellarr config list
docker compose exec stellarr stellarr config set time-zone Europe/Warsaw --yes
docker compose exec stellarr stellarr sessions list --user arkazz
```

> [!NOTE]
> `help`, `--help`, `version`, and `config get <startup-key>` (for any read-only startup setting) do not open or require the SQLite database. Every other command opens the existing SQLite database and never creates, migrates, or seeds it; databases missing the expected schema are rejected with `schema_incompatible` (exit status `4`).

## Command overview

| Command | Purpose |
| --- | --- |
| `status`, `doctor` | Inspect database, data root, free disk space, schema compatibility, and local API readiness |
| `config list \| get \| set \| validate` | View or update database-backed settings shared with Cockpit; inspect read-only startup settings |
| `secrets list \| set \| clear` | Inspect secret status or store/clear encrypted mail and GIPHY secrets |
| `users list \| show` | Inspect user accounts without exposing password hashes or tokens |
| `sessions list \| revoke` | List active sessions or revoke one or all sessions (and pending desktop handoffs) for a user |
| `security show \| methods \| recover \| unlink` | Inspect MFA/OIDC state, unlink an identity provider, or issue a one-time recovery credential |
| `backup verify \| restore \| cancel \| apply` | Verify full backups, stage an archive for restore on next container start, or cancel a staged restore |

## Data root and container paths

The CLI resolves `<data-root>` in this order:

1. `--data-root <path>`
2. `STELLARR_DATA_ROOT`
3. `DataRoot` configuration value
4. `/data/stellarr`

Inside `<data-root>`, the SQLite database lives at `db/stellarr.db` and the keys, including the master key (`keys/master.key`), live at `keys/`. Read-only commands never create `keys/` or write new key files. The API must have initialized `keys/` at least once before `stellarr secrets set` can encrypt secrets readable by the API.

## Commands

### Status and diagnostics

```bash
stellarr status
stellarr doctor
```

- `status`: Prints the database path and size, instance name, user and session counts, and whether the local API responds at `http://127.0.0.1:5000/health/ready` (override with `STELLARR_API_URL`). Exits `0` only when all checks pass (`healthy: true`), or `1` otherwise.
- `doctor`: Validates the data root, database schema, `keys/` directory, free disk space, and local API reachability without modifying anything. When the container or API is stopped, an unreachable API is reported as a warning rather than a failure so `doctor` works during offline recovery.

### Configuration

```bash
stellarr config list
stellarr config get time-zone
stellarr config get public-url
stellarr config set time-zone Europe/Warsaw --yes
stellarr config validate
```

`config list` displays each setting's configured value, effective value, source (`database`, `configuration`, `environment`, or `default`), editability, and whether a container restart is required. `config set` updates the same `OperationsSettings` database table used by Cockpit and enforces identical validation rules (`InstanceSettingsService`). `config validate` validates stored settings and warns about incomplete mail or integration setups without sending test emails.

#### Writable database-backed keys

| Key | Type / Allowed values |
| --- | --- |
| `instance.name` | text (`1–80` characters; updates the primary server name) |
| `time-zone` | IANA time zone (e.g. `Europe/Warsaw`) |
| `mail.enabled` | boolean |
| `mail.from-address` | email address (`≤ 320` characters; required when `mail.enabled` is `true`) |
| `mail.smtp.host` | text (`≤ 255` characters) |
| `mail.smtp.port` | integer (`1–65535`) |
| `mail.smtp.username` | text (`≤ 320` characters) |
| `mail.smtp.security` | `starttls`, `tls`, or `none` |
| `mail.resend.enabled` | boolean |
| `mail.invites.enabled`, `mail.recovery.enabled`, `mail.alerts.enabled` | boolean |
| `giphy.enabled` | boolean |
| `retention.backups-days`, `retention.logs-days`, `retention.orphans-days` | integer (`1–3650`) |
| `cleanup.enabled` | boolean |
| `mfa.requirement` | `optional`, `admins`, `everyone`, or `cockpit` |
| `mfa.trusted-device-days` | integer (`1–365`) |
| `mfa.challenge-minutes`, `mfa.email-code-minutes`, `mfa.cli-recovery-minutes` | integer (`1–60`) |
| `mfa.email-cooldown-seconds` | integer (`0–600`) |
| `mfa.email-attempts` | integer (`1–10`) |
| `mfa.email-sends-per-hour` | integer (`1–60`) |

Booleans accept `true`/`false`, `1`/`0`, `yes`/`no`, or `on`/`off`. For nullable integration switches (`giphy.enabled`, `mail.resend.enabled`), `value` is the saved preference (`(auto)` before being explicitly set) while `effectiveValue` reflects whether the required API key is also configured.

#### Read-only startup keys

Startup settings controlled by `.env` appear in `config list` and can be queried via `stellarr config get <key>` even before the database exists:

| Startup key | Environment variable(s) |
| --- | --- |
| `public-url` | `STELLARR_PUBLIC_URL` / `STELLARR_DOMAIN` / `STELLARR_PUBLIC_PORT` |
| `tls.domain`, `rtc.public-host` | `STELLARR_DOMAIN` |
| `https-port` | `STELLARR_HTTPS_PORT` |
| `oidc.enabled` | `STELLARR_OIDC_ENABLED` |
| `oidc.authority` | `STELLARR_OIDC_ISSUER` |
| `oidc.client-id` | `STELLARR_OIDC_CLIENT_ID` |
| `oidc.trust-provider-mfa` | `STELLARR_OIDC_TRUST_PROVIDER_MFA` |
| `oidc.provider-mfa-amr` | `STELLARR_OIDC_PROVIDER_MFA_AMR` |
| `oidc.providers` | `Oidc__Providers__<id>__*` |
| `oidc.callback-path` | `Oidc__CallbackPath` |
| `rtc.turn-port` | `STELLARR_TURN_PORT` |
| `rtc.turns-port` | `STELLARR_TURNS_PORT` |
| `rtc.turns-enabled` | `STELLARR_TURNS_ENABLED` |
| `rtc.turn-tcp-enabled` | `STELLARR_TURN_TCP_ENABLED` |
| `rtc.media-port` | `STELLARR_MEDIA_PORT` |

Attempting `config set` on a startup key fails with `read_only_setting` (exit status `4`) and prints the `.env` variable to update.

Because `docker exec` launches processes without the variables derived by `stellarr-entrypoint`, the `/usr/local/bin/stellarr` wrapper (`infra/stellarr-cli.sh`) sources `/usr/local/lib/stellarr/settings.sh` before starting the CLI so `public-url` resolves identically to the running API. If `.env` contains an invalid startup value, the wrapper prints the validation warning to `stderr` and still allows the CLI to run for offline repair.

### Secrets

```bash
stellarr secrets list
printf '%s\n' "$SMTP_PASSWORD" |
  docker compose exec -T stellarr stellarr secrets set mail.smtp-password --stdin --yes
stellarr secrets clear mail.smtp-password --yes
```

Supported secret names: `mail.smtp-password`, `mail.resend-api-key`, and `giphy.api-key`.

> [!WARNING]
> Pass secret values only via `--stdin` or an interactive hidden prompt. Passing a secret as a positional argument is rejected (`secret_value_argument`) so secrets never leak into `ps` output or shell history. Always pair `--stdin` with `--yes` so stdin is not consumed by the confirmation prompt.

Secrets are encrypted with `<data-root>/keys` and stored in the `InstanceSecrets` table, overriding `.env`. `secrets clear` removes the database override and reports whether an `.env` fallback is active. Plaintext secret values are never printed.

### Users and sessions

```bash
stellarr users list
stellarr users show arkazz
stellarr sessions list
stellarr sessions list --user arkazz
stellarr sessions revoke 4a85e7e0-bc1c-4e18-8ed2-191f1e8b0f21 --yes
stellarr sessions revoke --user arkazz --all --yes
```

- `users list` and `sessions list` return up to 200 and 500 rows respectively and expose only administrative metadata (never password hashes, session cookies, or tokens).
- `sessions revoke` marks sessions revoked in SQLite (`UserSessions.RevokedAt`) and deactivates any end-to-end encryption devices bound to those sessions. The running API polls revocations every second (the `realtime-sessions` worker) and closes matching hub connections and media sessions. Passing `--all` for a user also invalidates unredeemed desktop browser handoffs.

### Account security and MFA recovery

```bash
stellarr security show arkazz
stellarr security methods arkazz
stellarr security recover arkazz --reason "lost authenticator" --yes
stellarr security unlink arkazz 3f2a6c1e-0b7d-4f5e-9a2c-8d1e4b6f7a90 --yes
```

- `security show` / `security methods`: Displays the instance MFA policy, confirmed and pending MFA methods, unused recovery-code count, trusted devices, active sessions, linked OIDC identities, and recent security events without exposing TOTP seeds, hashes, or codes.
- `security unlink`: Removes an OIDC identity link from an account (existing sessions remain open until revoked). If the account has no password, no passkey, and no other linked identity at a configured provider, the CLI warns that unlinking will remove the account's last sign-in method.
- `security recover`: Atomically revokes all MFA methods, trusted devices, active sessions, desktop handoffs, and unused recovery codes (keeping OIDC links intact), then prints a single one-time recovery credential valid for `mfa.cli-recovery-minutes` (`10` minutes by default). `--reason` is optional (`3–500` characters when provided; `invalid_reason` otherwise). Disabled accounts cannot be recovered (`account_disabled`). See [Account recovery](account-security.md#operator-recovery).

### Backups and restore

| Command | Purpose |
| --- | --- |
| `backup verify <archive>` | Validates `manifest.json`, entry paths, SQLite integrity, and every entry's CRC-32 and size without modifying data. |
| `backup restore <archive>` | Verifies a full backup archive and stages it at `restore/staged.zip` for the next container start. |
| `backup cancel` | Removes `restore/staged.zip` before a container restart. |
| `backup apply` | Restores `restore/staged.zip` immediately; executed automatically by `stellarr-entrypoint` before services start (refuses to run while the API responds). |

`<archive>` can be an absolute/relative path inside the container or a filename inside `/data/stellarr/backups/`:

```bash
docker compose exec stellarr stellarr backup restore backup-20261114-030000-<id>.zip --yes
docker compose restart stellarr
```

See [Restoring a backup](backups.md#restoring-a-backup) for the full restore workflow and safeguards.

### Running against a stopped container

Because `stellarr` accesses `/data/stellarr` directly, you can run diagnostics or recovery while the main container is stopped:

```bash
docker compose stop stellarr
docker compose run --rm --no-deps --entrypoint stellarr stellarr doctor
docker compose start stellarr
```

Or against the named Docker volume directly:

```bash
docker run --rm -it \
  -v stellarr_stellarr_data:/data/stellarr \
  --entrypoint stellarr \
  ghcr.io/arkazzae/stellarr:latest doctor
```

> [!CAUTION]
> Never run two full Stellarr containers against the same `/data/stellarr` volume simultaneously. Use `--entrypoint stellarr` only for short-lived CLI invocations.

## Automation, `--json`, and exit statuses

Mutating commands preview the change and prompt for confirmation unless `--yes` (`-y`) is passed. When `stdin` is not a TTY and `--yes` is omitted, mutations fail immediately with `confirmation_required` (exit status `2`). Invalid arguments, declined prompts, and `Ctrl+C` leave the database untouched.

Pass `--json` for machine-readable output (prompts and diagnostics go to `stderr` so `stdout` always contains a single JSON document):

```json
{
  "ok": true,
  "command": "config.set",
  "exitCode": 0,
  "data": { "key": "time-zone", "value": "Europe/Warsaw" },
  "warnings": []
}
```

On failure, `ok` is `false` and `error` contains a stable snake_case `code`, `message`, and optional `details`:

```json
{
  "ok": false,
  "command": "config.set",
  "exitCode": 2,
  "error": {
    "code": "invalid_value",
    "message": "This time zone is not available on the server.",
    "details": { "fields": { "timeZone": ["..."] } }
  }
}
```

| Exit status | Meaning |
| --- | --- |
| `0` | Success (`status` and `doctor` passed). |
| `1` | Unexpected error, or `status`/`doctor` reported an unhealthy check. |
| `2` | Invalid arguments or value, missing/declined confirmation, or cancellation. |
| `3` | Resource not found (database, instance server, user, session, identity link, setting, secret, or backup archive). |
| `4` | Conflict or failed precondition (read-only setting, locked/unreadable database, incompatible schema, disabled account, insufficient disk space, running API during `backup apply`, or invalid backup archive). |
| `5` | Database file could not be opened (`database_unavailable`). |

Stable `error.code` values: `usage`, `unknown_command`, `confirmation_required`, `cancelled`, `invalid_value`, `invalid_user`, `invalid_session_id`, `invalid_reason`, `invalid_secret`, `read_only_setting`, `secret_not_configuration`, `secret_value_argument`, `secret_value_required`, `secret_write_failed`, `data_protection_keys_missing`, `database_not_found`, `database_locked`, `database_unavailable`, `schema_incompatible`, `instance_not_found`, `setting_not_found`, `secret_not_found`, `user_not_found`, `session_not_found`, `identity_not_found`, `account_disabled`, `backup_not_found`, `backup_refused`, `not_enough_space`, `restore_not_staged`, `restore_refused`, `restore_broken`, and `instance_running`.
