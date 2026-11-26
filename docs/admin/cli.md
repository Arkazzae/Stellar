# Operator CLI

`stellarr` (`/usr/local/bin/stellarr` inside the container) inspects instance health, edits database settings shared with Cockpit, manages sealed secrets, revokes sessions, issues MFA recovery credentials, and verifies or stages backup restores directly on `/data/stellarr`—even when Cockpit is unreachable or the API is stopped.

Anyone with `docker exec` access or read/write access to `/data/stellarr` has full administrative control. Every CLI write commits an `OperatorAuditEntries` row (`local-cli` actor) inside the same SQLite transaction.

```bash
docker compose exec stellarr stellarr doctor
docker compose exec stellarr stellarr config list
docker compose exec stellarr stellarr config set time-zone Europe/Warsaw --yes
docker compose exec stellarr stellarr sessions list --user arkazz
```

> [!NOTE]
> `help`, `--help`, `version`, and `config get <startup-key>` run without opening SQLite. All other commands open the existing database in `db/stellarr.db` without creating, migrating, or seeding it; incompatible schemas exit with `schema_incompatible` (`4`).

## Command summary

| Command group | Subcommands | Purpose |
| --- | --- | --- |
| **Diagnostics** | `status`, `doctor` | Check data root, SQLite schema, `keys/master.key`, free disk space, and local API readiness |
| **Configuration** | `config list \| get \| set \| validate` | Inspect or update Cockpit-shared DB settings and read-only startup variables |
| **Secrets** | `secrets list \| set \| clear` | Check status or write/remove AES-256-GCM sealed integration secrets |
| **Accounts** | `users list \| show` | Inspect accounts (up to 200 rows) without exposing hashes or tokens |
| **Sessions** | `sessions list \| revoke` | List active sessions (up to 500 rows) or revoke one/all sessions for an account |
| **Security** | `security show \| methods \| recover \| unlink` | Inspect MFA/OIDC state, unlink an identity provider, or issue a one-time MFA recovery credential |
| **Backups** | `backup verify \| restore \| cancel \| apply` | Verify `.zip` archives, stage a restore for next startup, or cancel a staged restore |

## Data root and wrapper behavior

`<data-root>` resolves in priority order: `--data-root <path>` → `STELLARR_DATA_ROOT` → `DataRoot` config → `/data/stellarr`.

- **Wrapper (`infra/stellarr-cli.sh`):** Sources `/usr/local/lib/stellarr/settings.sh` before invoking `/opt/stellarr/cli/stellarr` so `docker exec` sessions derive `public-url` identically to `stellarr-api`. If `.env` contains a malformed startup variable, the wrapper prints a warning to `stderr` without blocking offline CLI repair.
- **Master key (`keys/master.key`):** Read-only commands never touch `keys/`. `secrets set` requires `keys/` to exist (`data_protection_keys_missing` if the API has never initialized `<data-root>`) and creates `keys/master.key` (`0600`) only after confirmation when absent.

---

## Commands in detail

### Diagnostics (`status` and `doctor`)

```bash
stellarr status
stellarr doctor
```

- `status` prints database size, instance name, user/session counts, and local API health (`http://127.0.0.1:5000/health/ready`, overridable via `STELLARR_API_URL`), exiting `0` only when every check passes.
- `doctor` validates `<data-root>`, SQLite schema, `keys/`, free space, and API reachability—treating an offline API as a warning rather than an error so offline recovery succeeds.

### Configuration

```bash
stellarr config list
stellarr config get time-zone
stellarr config get public-url
stellarr config set time-zone Europe/Warsaw --yes
stellarr config validate
```

`config list` reports each setting's stored value, effective value, source (`database`, `configuration`, `environment`, or `default`), mutability, and restart requirement. Booleans accept `true`/`false`, `1`/`0`, `yes`/`no`, or `on`/`off`.

#### Writable database keys (`OperationsSettings`)

| Category | Keys | Allowed values |
| --- | --- | --- |
| **General** | `instance.name`, `time-zone` | `1–80` chars (renames primary server); valid IANA time zone (e.g. `Europe/Warsaw`) |
| **Mail & SMTP** | `mail.enabled`, `mail.from-address`, `mail.smtp.host`, `mail.smtp.port`, `mail.smtp.username`, `mail.smtp.security`, `mail.resend.enabled`, `mail.invites.enabled`, `mail.recovery.enabled`, `mail.alerts.enabled` | Booleans; email `≤ 320` chars; host `≤ 255` chars; port `1–65535`; security `starttls` \| `tls` \| `none` |
| **Integrations** | `giphy.enabled` | Boolean (`(auto)` before explicit save; effective value checks key presence) |
| **Retention** | `cleanup.enabled`, `retention.backups-days`, `retention.logs-days`, `retention.orphans-days` | Boolean; integers `1–3650` |
| **MFA policy** | `mfa.requirement`, `mfa.trusted-device-days`, `mfa.challenge-minutes`, `mfa.email-code-minutes`, `mfa.cli-recovery-minutes`, `mfa.email-cooldown-seconds`, `mfa.email-attempts`, `mfa.email-sends-per-hour` | `optional` \| `admins` \| `everyone` \| `cockpit`; days `1–365`; minutes `1–60`; cooldown `0–600`; attempts `1–10`; sends `1–60` |

#### Read-only startup keys (controlled via `.env`)

| Startup key | Backing `.env` variable(s) |
| --- | --- |
| `public-url` | `STELLARR_PUBLIC_URL` / `STELLARR_DOMAIN` / `STELLARR_PUBLIC_PORT` |
| `tls.domain`, `rtc.public-host` | `STELLARR_DOMAIN` |
| `https-port`, `rtc.media-port` | `STELLARR_HTTPS_PORT`, `STELLARR_MEDIA_PORT` |
| `oidc.enabled`, `oidc.authority`, `oidc.client-id`, `oidc.trust-provider-mfa`, `oidc.provider-mfa-amr`, `oidc.providers` | `STELLARR_OIDC_*` and `Oidc__Providers__<id>__*` |
| `rtc.turn-enabled`, `rtc.turn-port`, `rtc.turns-port`, `rtc.turns-enabled`, `rtc.turn-tcp-enabled` | `STELLARR_TURN_*` and `STELLARR_TURNS_*` |

Calling `config set` on a startup key exits `4` (`read_only_setting`) and prints the `.env` variable to edit instead.

### Secrets

Supported names: `mail.smtp-password`, `mail.resend-api-key`, `giphy.api-key`, `cloudflare.r2.access-key`, `cloudflare.r2.secret-key`, `cloudflare.r2.worker-token`.

```bash
stellarr secrets list
printf '%s\n' "$SMTP_PASSWORD" |
  docker compose exec -T stellarr stellarr secrets set mail.smtp-password --stdin --yes
stellarr secrets clear mail.smtp-password --yes
```

> [!WARNING]
> Positional secret values are rejected (`secret_value_argument`) to keep credentials out of `ps` and shell history. Supply secrets via `--stdin` (always paired with `--yes`) or an interactive hidden prompt. If a stored row cannot be decrypted with `keys/master.key`, `secrets list` exits `4` (`secret_unreadable`) until overwritten with `secrets set` or removed with `secrets clear`.

### Users, sessions, and MFA recovery

```bash
# Inspect and revoke sessions (terminates live WebSockets & E2EE devices within 1 s)
stellarr users show arkazz
stellarr sessions list --user arkazz
stellarr sessions revoke 4a85e7e0-bc1c-4e18-8ed2-191f1e8b0f21 --yes
stellarr sessions revoke --user arkazz --all --yes

# Inspect security methods, unlink OIDC, or issue a 10-minute one-time MFA recovery credential
stellarr security show arkazz
stellarr security recover arkazz --reason "lost authenticator" --yes
stellarr security unlink arkazz 3f2a6c1e-0b7d-4f5e-9a2c-8d1e4b6f7a90 --yes
```

See [Operator recovery](account-security.md#operator-recovery) for what `security recover` resets and how the user redeems the printed credential.

### Backups and offline execution

```bash
# Verify and stage a full backup from /data/stellarr/backups/ for next startup
docker compose exec stellarr stellarr backup verify backup-20261114-030000-<id>.zip
docker compose exec stellarr stellarr backup restore backup-20261114-030000-<id>.zip --yes
docker compose restart stellarr
```

To run CLI diagnostics or recovery while the main container is stopped:

```bash
docker compose stop stellarr
docker compose run --rm --no-deps --entrypoint stellarr stellarr doctor
docker compose start stellarr
```

> [!CAUTION]
> Never start two full Stellarr server containers against the same `/data/stellarr` volume concurrently. Use `--entrypoint stellarr` strictly for short-lived CLI commands. See [Restoring a backup](backups.md#restoring-a-backup).

---

## Automation, `--json`, and exit codes

Mutating commands prompt for confirmation unless `--yes` (`-y`) is passed (non-TTY runs without `--yes` exit `2` with `confirmation_required`). Pass `--json` to emit a single machine-readable JSON envelope on `stdout` while sending prompts and warnings to `stderr`:

```json
{
  "ok": true,
  "command": "config.set",
  "exitCode": 0,
  "data": { "key": "time-zone", "value": "Europe/Warsaw" },
  "warnings": []
}
```

| Exit code | Meaning | Representative `error.code` values |
| --- | --- | --- |
| `0` | Command succeeded (`status` / `doctor` healthy) | — |
| `1` | Unexpected error or unhealthy check | `unexpected_error`, `secret_write_failed` |
| `2` | Invalid input, missing confirmation, or cancelled | `usage`, `unknown_command`, `invalid_value`, `invalid_user`, `invalid_session_id`, `invalid_secret`, `invalid_reason`, `secret_value_argument`, `secret_value_required`, `confirmation_required`, `cancelled` |
| `3` | Target resource not found | `database_not_found`, `instance_not_found`, `user_not_found`, `session_not_found`, `identity_not_found`, `setting_not_found`, `secret_not_found`, `backup_not_found` |
| `4` | Conflict or failed precondition | `read_only_setting`, `database_locked`, `database_unreadable`, `database_error`, `schema_incompatible`, `account_disabled`, `permission_denied`, `data_protection_keys_missing`, `keys_unreadable`, `secret_unreadable`, `secret_not_configuration`, `not_enough_space`, `instance_running`, `backup_refused`, `restore_not_staged`, `restore_refused`, `restore_broken` |
| `5` | Storage / filesystem unavailable | `database_unavailable`, `free_space_unknown` |
