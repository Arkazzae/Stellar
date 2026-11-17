# Operator CLI

`stellarr` is the operator CLI bundled inside the container image. It inspects health, edits database-backed settings shared with Cockpit, manages encrypted secrets, revokes sessions, recovers locked-out accounts, and verifies or stages backups directly against `/data/stellarr`, without needing Cockpit access or a second API process.

Anyone who can run `docker exec` or mount `/data/stellarr` controls the instance. Every CLI mutation writes an audit entry under the actor `local-cli`.

```bash
docker compose exec stellarr stellarr doctor
docker compose exec stellarr stellarr config list
docker compose exec stellarr stellarr config set time-zone Europe/Warsaw
docker compose exec stellarr stellarr sessions list --user arkazz
```

> [!NOTE]
> `help`, `--help`, and `version` do not touch the database. Every other command opens the existing SQLite database and never creates or migrates it; databases missing expected schema are refused with `schema_incompatible`.

## Command overview

| Command | Purpose |
| --- | --- |
| `status`, `doctor` | Inspect database, data root, disk space, schema compatibility, and local API readiness |
| `config list \| get \| set \| validate` | View or update database-backed settings shared with Cockpit; inspect read-only startup settings |
| `secrets list \| set \| clear` | List secret status or store/clear encrypted mail and GIPHY secrets |
| `users list \| show` | Inspect accounts without exposing hashes or tokens |
| `sessions list \| revoke` | Inspect active sessions or revoke one or all sessions for a user |
| `security show \| methods \| recover \| unlink` | Inspect MFA/OIDC state, unlink an identity provider, or issue a one-time recovery credential |
| `backup verify \| restore \| cancel \| apply` | Verify full backups, stage one for restore on next start, or cancel a staged restore |

## Data root and container paths

The CLI resolves `<data-root>` in this order:

1. `--data-root <path>`
2. `STELLARR_DATA_ROOT`
3. `DataRoot` configuration value
4. `/data/stellarr`

Inside `<data-root>`, the SQLite database lives at `db/stellarr.db` and data-protection keys at `keys/`. Read-only commands never create `keys/` or write new key files. The API must have initialized `keys/` at least once before `secrets set` can encrypt values that the API can read.

## Commands

### Status and diagnostics

```bash
stellarr status
stellarr doctor
```

- `status` prints database path and size, instance name, account/session counts, and whether the local API answers at `http://127.0.0.1:5000/health/ready` (override with `STELLARR_API_URL`). Exits `0` only when healthy (`healthy: true`), or `1` otherwise.
- `doctor` validates the data root, database schema, `keys/` directory, free disk space, and local API reachability without modifying anything. When the container is stopped, an unreachable API is treated as a warning so `doctor` works during offline recovery.

### Configuration

```bash
stellarr config list
stellarr config get time-zone
stellarr config set time-zone Europe/Warsaw
stellarr config validate
```

`config list` shows each key's effective value, source (`database`, `configuration`, `environment`, or `default`), editability, and restart requirement. `config set` updates the same database settings used by Cockpit and validates using the same rules. `config validate` checks stored settings and warns about incomplete mail or integration setups without sending test emails.

**Writable database-backed keys:**

| Key | Type / Allowed values |
| --- | --- |
| `instance.name` | text (`1–80` chars) |
| `time-zone` | IANA time zone (e.g. `Europe/Warsaw`) |
| `mail.enabled` | boolean |
| `mail.from-address` | email address |
| `mail.smtp.host` | text (`≤ 255` chars) |
| `mail.smtp.port` | integer (`1–65535`) |
| `mail.smtp.username` | text (`≤ 320` chars) |
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

Booleans accept `true`/`false`, `1`/`0`, `yes`/`no`, or `on`/`off`. For nullable integration toggles (`giphy.enabled`, `mail.resend.enabled`), `value` is the saved preference while `effectiveValue` reflects whether required secrets are also present.

**Read-only startup keys:** Settings driven by `.env` or container environment variables (`public-url`, `tls.domain`, `https-port`, `oidc.*`, `rtc.*`) appear in `config list` as read-only. Attempting `config set` on them fails with `read_only_setting` and prints the environment variable to change instead.

### Secrets

```bash
stellarr secrets list
printf '%s\n' "$SMTP_PASSWORD" |
  docker compose exec -T stellarr stellarr secrets set mail.smtp-password --stdin --yes
stellarr secrets clear mail.smtp-password
```

Supported secret keys: `mail.smtp-password`, `mail.resend-api-key`, and `giphy.api-key`.

> [!WARNING]
> Pass secret values only via `--stdin` or an interactive hidden prompt. Passing a secret as a CLI argument is rejected so values never leak into process lists or shell history. Combine `--stdin` with `--yes` so stdin does not conflict with confirmation prompts.

Secrets are encrypted with `<data-root>/keys` and stored alongside Cockpit's secrets (taking precedence over `.env`). `secrets clear` deletes the database override and reports any active `.env` fallback. Plaintext secrets, hashes, and tokens are never printed.

### Users and sessions

```bash
stellarr users list
stellarr users show arkazz
stellarr sessions list
stellarr sessions list --user arkazz
stellarr sessions revoke 4a85e7e0-bc1c-4e18-8ed2-191f1e8b0f21
stellarr sessions revoke --user arkazz --all
```

- `users list` and `sessions list` cap output at 200 and 500 rows respectively and show only administrative metadata.
- `sessions revoke` marks sessions revoked in SQLite; the API polls revocations at most once per second and closes matching WebSocket and media sessions. Revoking `--all` for a user also invalidates unredeemed desktop browser handoff tokens.

### Account security and MFA recovery

```bash
stellarr security show arkazz
stellarr security methods arkazz
stellarr security recover arkazz --reason "lost authenticator"
stellarr security unlink arkazz 3f2a6c1e-0b7d-4f5e-9a2c-8d1e4b6f7a90
```

- `security show` / `security methods`: displays the instance MFA policy, confirmed/pending methods, unused recovery-code count, trusted devices, active sessions, linked OIDC identities, and recent security events (without exposing TOTP seeds, hashes, or tokens).
- `security unlink`: removes an OIDC identity link (active sessions stay open until revoked separately).
- `security recover`: atomically revokes all MFA methods, trusted devices, active sessions, desktop handoffs, and unused recovery codes, then prints a single short-lived recovery credential (valid for `mfa.cli-recovery-minutes`, default 10 minutes). Disabled accounts cannot be recovered. See [Account recovery](account-security.md#operator-recovery) for the full user workflow.

### Backups and restore

| Command | Purpose |
| --- | --- |
| `backup verify <archive>` | Validates manifest, entry paths, SQLite integrity, and entry checksums without changing anything. |
| `backup restore <archive>` | Verifies a full backup and stages it at `restore/staged.zip` for the next container start. |
| `backup cancel` | Removes a staged restore archive before restart. |
| `backup apply` | Applies `restore/staged.zip`; run automatically by the container entrypoint before services start (refuses while the API is running). |

`<archive>` may be any path inside the container or a filename inside `/data/stellarr/backups`. Only full four-part backups can be restored automatically:

```bash
docker compose exec stellarr stellarr backup restore backup-20261114-030000-<id>.zip --yes
docker compose restart stellarr
```

See [Restoring a backup](backups.md#restoring-a-backup) for the full procedure, rollback folder layout (`restore/previous-<time>/`), and partial backup recovery.

### Running against a stopped container

Because `stellarr` reads the data volume directly, you can run diagnostics or recovery while the server is stopped:

```bash
docker compose stop stellarr
docker compose run --rm --no-deps --entrypoint stellarr stellarr doctor
docker compose start stellarr
```

Or against the named volume directly:

```bash
docker run --rm -it \
  -v stellarr_stellarr_data:/data/stellarr \
  --entrypoint stellarr \
  ghcr.io/arkazzae/stellarr:latest doctor
```

> [!CAUTION]
> Never run two full Stellarr server containers against the same `/data/stellarr` volume simultaneously. Use `--entrypoint stellarr` only for short-lived CLI commands.

## Automation, `--json`, and exit statuses

Mutating commands prompt for confirmation unless `--yes` (`-y`) is passed. In non-interactive shells without `--yes`, mutations fail with `confirmation_required`. Invalid arguments, cancelled prompts, and `Ctrl+C` leave the database untouched. Each mutation commits its database write and `OperatorAuditEntries` row (`local-cli`) in one transaction.

Pass `--json` for machine-readable output (diagnostics go to `stderr` so `stdout` stays valid JSON):

```json
{
  "ok": true,
  "command": "config.set",
  "exitCode": 0,
  "data": { "key": "time-zone", "value": "Europe/Warsaw" },
  "warnings": []
}
```

On failure, `ok` is `false` and `error` includes a snake_case `code`, `message`, and optional `details`:

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
| `3` | Resource not found (database, instance, account, session, identity, setting, secret, or backup). |
| `4` | Conflict or precondition error (read-only setting, locked/unreadable database, incompatible schema, disabled account, insufficient disk space, running API during `backup apply`, or refused backup). |
| `5` | Database file could not be opened (`database_unavailable`). |

Common `error.code` values include `usage`, `unknown_command`, `confirmation_required`, `cancelled`, `invalid_value`, `read_only_setting`, `database_not_found`, `database_locked`, `database_unavailable`, `schema_incompatible`, `setting_not_found`, `secret_not_found`, `secret_value_argument`, `secret_value_required`, `user_not_found`, `session_not_found`, `identity_not_found`, `account_disabled`, `backup_not_found`, `backup_refused`, and `instance_running`.
