# Configuration

Every operator-facing setting uses a `STELLARR_*` variable with the same name across `.env`, `docker run -e`, and platform templates such as [Unraid](unraid.md). Built-in defaults live inside the image, so `.env` only needs the settings you override; empty or whitespace-only values count as unset.

With Docker Compose, edit `.env` beside `docker-compose.yml` ([`.env.example`](../../.env.example) lists all 61 settings commented out) and recreate the container:

```bash
docker compose up -d
```

> [!IMPORTANT]
> Compose passes `.env` into the container via `env_file`. Exporting variables only in your shell (or via `docker compose --env-file`) affects `docker-compose.yml` interpolation (`STELLARR_VERSION`, host port mappings, and container limits) without reaching processes inside the container unless written to `.env`.

### Configuration precedence

| Priority (highest first) | Source | Notes |
| --- | --- | --- |
| **1. Sealed secrets & database settings** | **Cockpit** (`/cockpit`) or [`stellarr` CLI](../admin/cli.md) | Stored in SQLite (`OperationsSettings` and AES-256-GCM `InstanceSecrets`). Takes effect without restarting. |
| **2. `STELLARR_*` environment variables** | `.env` / container environment | Variables marked **Seed** initialize database-backed settings until an operator saves a value in Cockpit or the CLI. |
| **3. Raw `Section__Key` keys** | `.env` / container environment | Tunes internal subsystems that have no `STELLARR_*` alias ([see below](#settings-without-a-stellarr_-name)). |
| **4. Image defaults** | `settings.sh` & `defaults.rs` | Applied whenever a setting is omitted or blank. |

---

## Release

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_VERSION` | image tag | `latest` | Tag of `ghcr.io/arkazzae/stellarr` pulled by Compose (`1.2.3` to pin a release, `1.2` to track a series). |
| `STELLARR_TRACK` | release track | empty | Recorded by `install.sh` when tracking `latest` or a minor series while `STELLARR_VERSION` pins the resolved version. |
| `STELLARR_IMAGE` | image reference | empty | Overrides the full image reference (`stellarr-local:dev` when running `install.sh --build`). |

## Address and TLS

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_DOMAIN` | hostname or IP | empty | Browser-facing hostname or IP. Sets the self-signed TLS SAN, public URL host, and TURN realm (`localhost` when unset). |
| `STELLARR_PUBLIC_IP` | IPv4 or `auto` | empty | External IPv4 advertised by `stellarr-media` and `coturn`. `auto` resolves `STELLARR_DOMAIN` at startup; unset falls back to a numeric IPv4 `STELLARR_DOMAIN`. |
| `STELLARR_HTTPS_PORT` | port | `443` | Host TCP port mapped to container `443/tcp`. Used for HTTP→HTTPS redirects and the default public URL. |
| `STELLARR_HTTP_PORT` | port or `ip:port` | `127.0.0.1:8888` | Host TCP binding for container `80/tcp` (local reverse-proxy upstream). Read by Compose only. |
| `STELLARR_PUBLIC_PORT` | port | `STELLARR_HTTPS_PORT` | External HTTPS port browsers connect to (`443` behind a [reverse proxy](reverse-proxy-and-tls.md#run-behind-a-reverse-proxy); omitted from URLs when `443`). |
| `STELLARR_PUBLIC_URL` | origin URL | empty | Explicit public origin (`http://` or `https://`, no path/query/fragment), overriding `STELLARR_DOMAIN` + `STELLARR_PUBLIC_PORT`. |
| `STELLARR_ACTIVITY_DOMAIN` | DNS suffix | empty | Installed activities use the instance's HTTPS address by default. To use dedicated hostnames, set a suffix (e.g. `activities.example.com`), keep the instance hostname outside it, and route wildcard DNS and trusted wildcard TLS to Stellarr. |
| `STELLARR_ACTIVITY_ORIGIN` | HTTPS origin | instance public URL | Web client origin allowed to embed dedicated activity hosts. The hosts use this origin's public HTTPS port. |
| `STELLARR_TRUSTED_PROXIES` | IP / CIDR list | empty | Comma- or space-separated IPv4/IPv6 addresses or CIDRs of upstream [reverse proxies](reverse-proxy-and-tls.md#client-addresses). |

- **Self-signed & custom TLS:** Absent `/data/stellarr/tls/{fullchain,privkey}.pem` triggers automatic 365-day RSA-3072 self-signed generation (renewed below 30 days or on `STELLARR_DOMAIN` change). Operator-mounted CA certificates are never overwritten; see [Reverse proxy and TLS](reverse-proxy-and-tls.md).
- **Public origin usages:** Drives invite links, password-reset and email-confirmation links, `/.well-known/stellarr/client`, and WebAuthn passkey RP verification. Password-reset emails remain disabled when both `STELLARR_DOMAIN` and `STELLARR_PUBLIC_URL` are unset.
- **Proxy validation:** Malformed `STELLARR_TRUSTED_PROXIES` entries (hostnames, non-zero CIDR host bits like `10.0.0.1/24`, or `0.0.0.0/0`) abort API startup immediately.

## Voice and RTC

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_MEDIA_PORT` | port | `49000` | Host UDP and TCP port for [Stellarr Media](../stellarr-media.md). Forward the same port for both protocols. |
| `STELLARR_MEDIA_LAN_IP` | IPv4 address | empty | Optional host LAN IPv4 advertised alongside `STELLARR_PUBLIC_IP` when routers lack NAT hairpinning. |
| `STELLARR_TURN_ENABLED` | `true` \| `false` | `false` | Enables `coturn`. Pair with [the TURN Compose overlay](../rtc-and-ports.md#optional-turn-relay) to publish relay ports. |
| `STELLARR_TURN_PORT` | port | `3478` | Host UDP/TCP port for STUN and TURN. |
| `STELLARR_TURNS_PORT` | port | `5349` | Host TCP/UDP port for TURN over TLS/DTLS. |
| `STELLARR_TURN_RELAY_RANGE` | port range | `49160-49200` | UDP media relay port range mapped 1:1 on the host. |
| `STELLARR_TURN_TCP_ENABLED` | `true` \| `false` | `false` | Enables `coturn` TCP listener and `?transport=tcp` TURN URLs. |
| `STELLARR_TURNS_ENABLED` | `true` \| `false` | `false` | Enables `turns:` over TLS/DTLS (requires a browser-trusted certificate). |

When TURN is disabled, `/api/bootstrap` returns `rtc.iceServers: []` and browsers connect directly using SDP host candidates. Disabled TURN transports are closed in `/etc/turnserver.conf` (`no-tcp`, `no-tls`, `no-dtls`). See [RTC and ports](../rtc-and-ports.md).

## Uploads and quotas

Per-account storage limits can be updated live in **Cockpit → Settings → File storage** (covering chat uploads and Discord attachment imports). Set any byte limit to `0` (or negative) to disable that check.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_UPLOAD_USER_BYTES` | bytes | `2147483648` (2 GiB) | **Seed** for maximum upload storage per account. |
| `STELLARR_UPLOAD_TOTAL_BYTES` | bytes | `21474836480` (20 GiB) | Maximum total upload storage across the instance. |
| `STELLARR_MIN_FREE_BYTES` | bytes | `1073741824` (1 GiB) | Minimum free disk space preserved on `/data/stellarr`. |
| `STELLARR_MAX_CONCURRENT_UPLOADS` | integer (`1–32`) | `4` | Maximum concurrent multipart uploads processed by the API. |

Per-file ceilings: images `10 MiB` (`+1 MiB` preview), audio `25 MiB`, general files `25 MiB`, video `100 MiB` (`+2 MiB` poster), HTTP body cap `128 MiB`.

## Mail

Outgoing mail (password resets, email 2FA codes, email-change confirmations, security alerts, and opt-in activity notifications) uses either SMTP (**Cockpit → Settings**) or Resend (**Cockpit → Integrations**).

> [!NOTE]
> Secrets stored in Cockpit or via `stellarr secrets set` (`mail.smtp-password`, `mail.resend-api-key`, `giphy.api-key`, `cloudflare.r2.*`) are sealed with AES-256-GCM under `/data/stellarr/keys/master.key` and override `.env`. Clearing a stored secret restores the `.env` fallback.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_MAIL_ENABLED` | `true` \| `false` | `true` | **Seed** for master mail delivery toggle (`false` in native `Development` mode). |
| `STELLARR_MAIL_FROM` | email | empty | **Seed** for sender address (`onboarding@resend.dev` works only with Resend). |
| `STELLARR_MAIL_FROM_NAME` | string | `Stellarr` | Display name in the `From` header. |
| `STELLARR_SMTP_PASSWORD` | string | empty | Fallback SMTP password when unset in Cockpit. |
| `STELLARR_RESEND_API_KEY` | string | empty | Fallback Resend API key when unset in Cockpit. |

## Web Push

Browser push alerts remain off until a persistent P-256 VAPID key pair is configured. Generate keys once per instance—rotating keys invalidates existing browser subscriptions; see [Activity notifications](../admin/notifications.md#web-push).

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_PUSH_ENABLED` | `true` \| `false` | `false` | Enables Web Push when valid VAPID keys and subject are present. |
| `STELLARR_PUSH_SUBJECT` | `mailto:` or `https://` | empty | Operator contact URI sent to push services. |
| `STELLARR_PUSH_PUBLIC_KEY` | base64url | empty | Uncompressed P-256 public key (`65` bytes decoded). |
| `STELLARR_PUSH_PRIVATE_KEY` | base64url | empty | P-256 private key (`32` bytes decoded). Keep secret. |

## Single sign-on (OIDC)

Configures the default OpenID Connect provider alongside local passwords. Register callback URL `<public-url>/api/auth/oidc/callback` at your provider.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_OIDC_ENABLED` | `true` \| `false` | `false` | Enables the default OIDC provider. |
| `STELLARR_OIDC_ISSUER` | HTTPS URL | empty | Provider issuer/authority URL (`https://` required). |
| `STELLARR_OIDC_CLIENT_ID` | string | empty | OIDC relying-party client ID. |
| `STELLARR_OIDC_CLIENT_SECRET` | string | empty | Client secret (takes precedence over `STELLARR_OIDC_CLIENT_SECRET_FILE`). |
| `STELLARR_OIDC_CLIENT_SECRET_FILE` | container path | empty | Path inside the container holding the client secret (read once at startup). |
| `STELLARR_OIDC_DISPLAY_NAME` | string | `Main account` / `Konto główne` | Button label on `/login` and `/register`. |
| `STELLARR_OIDC_TRUST_PROVIDER_MFA` | `true` \| `false` | `false` | Accepts matching `amr` + fresh `auth_time` (`≤ 5 min`) as satisfying Stellarr MFA. |
| `STELLARR_OIDC_PROVIDER_MFA_AMR` | comma list | `mfa` | `amr` claim values accepted when `STELLARR_OIDC_TRUST_PROVIDER_MFA=true`. |

### Additional OIDC providers

Register extra providers in `.env` with `Oidc__Providers__<id>__*` (`<id>` is `1–32` lowercase letters, digits, or hyphens; callback URL `<public-url>/api/auth/oidc/<id>/callback`):

```dotenv
Oidc__Providers__work__DisplayName=Work SSO
Oidc__Providers__work__Authority=https://id.example.com/realms/work
Oidc__Providers__work__ClientId=stellarr
Oidc__Providers__work__ClientSecretFile=/run/secrets/oidc-work
Oidc__Providers__work__TrustProviderMfa=true
Oidc__Providers__work__ProviderMfaAmr=mfa,hwk
```

- Reserved `<id>` names (`default`, `login`, `callback`, `link`, `reauthenticate`, `pending`) or custom `Oidc__CallbackPath` overrides abort startup.
- Accounts match strictly by `(issuer, subject)`—never by email. Unlinked sign-ins land on `/link-account` (`10 min` cookie) to link an existing account or, when enabled in **Cockpit → Integrations**, register a passwordless invited account; see [Account security](../admin/account-security.md#linking-an-identity-provider).

## Automatic backups and Cloudflare R2

Production containers run a full `.zip` backup every 24 hours by default. Schedules and R2 targets can be managed live in Cockpit; see [Automatic backups](../admin/backups.md) and [Cloudflare R2](../admin/cloudflare-r2.md).

### Backup schedule

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_BACKUP_ENABLED` | `true` \| `false` | `true` | **Seed** for scheduled full backups. |
| `STELLARR_BACKUP_INTERVAL_HOURS` | integer (`1–720`) | `24` | **Seed** for hours between completed runs. |
| `STELLARR_BACKUP_RETRY_MINUTES` | integer (`1–1440`) | `15` | **Seed** for retry delay after a failed backup or R2 upload. |
| `STELLARR_BACKUP_INITIAL_DELAY_MINUTES` | integer (`0–1440`) | `5` | **Seed** for delay after container boot before checking the schedule. |

### Cloudflare R2 integration

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_CLOUDFLARE_R2_ENABLED` | `true` \| `false` | `false` | **Seed** enabling offsite backup uploads to Cloudflare R2. |
| `STELLARR_CLOUDFLARE_R2_MODE` | `s3` \| `worker` | `s3` | Direct S3 multipart (`s3`) or Backup Relay Worker (`worker`). |
| `STELLARR_CLOUDFLARE_R2_ENDPOINT` | HTTPS URL | empty | R2 S3 API endpoint or deployed Worker URL. |
| `STELLARR_CLOUDFLARE_R2_REGION` | string | `auto` | S3 signing region in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_BUCKET` | string | empty | Private R2 bucket name in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_ACCESS_KEY` | string | empty | R2 S3 access key ID in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_SECRET_KEY` | string | empty | R2 S3 secret access key in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_WORKER_TOKEN` | string | empty | Bearer token (`BACKUP_TOKEN`) in `worker` mode. |
| `STELLARR_CLOUDFLARE_R2_PREFIX` | string | `stellarr` | Key prefix (`<prefix>/<server-uuid>/backup-...zip`). |

## Language, identity, and integrations

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_LANGUAGE` | `en` \| `pl` \| `de` | `en` | Default web client locale and server locale for API errors, system notices, starter names, audit rows, and emails. Unshipped codes use English. See [Localization](../localization.md). |
| `STELLARR_INSTANCE_ID` | UUID | generated | Instance UUID served at `/.well-known/stellarr/client` (persisted in `/data/stellarr/instance-id`). |
| `STELLARR_TIME_ZONE` | IANA time zone | `UTC` | **Seed** for instance time zone in Cockpit (e.g. `Europe/Warsaw`). |
| `STELLARR_DESKTOP_MINIMUM_VERSION` | semver | `0.1.0` | **Seed** for minimum allowed desktop client version (`X-Stellarr-Client: desktop-linux`). |
| `STELLARR_DESKTOP_RECOMMENDED_VERSION` | semver | `0.1.3` | **Seed** for recommended desktop client version in discovery metadata. |
| `STELLARR_GIPHY_API_KEY` | string | empty | Fallback GIPHY API key when none is saved in **Cockpit → Integrations**. |

## Container resource limits

Interpolated by `docker-compose.yml` when creating the container (with `docker run`, pass `--memory`, `--cpus`, and `--pids-limit` directly):

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_CONTAINER_MEMORY_LIMIT` | size | `4g` | Container memory ceiling (`mem_limit`). |
| `STELLARR_CONTAINER_CPUS` | number | `4` | Container CPU quota (`cpus`). |
| `STELLARR_CONTAINER_PIDS_LIMIT` | integer | `512` | Container process limit (`pids_limit`). |

## Advanced

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_TLS_RENEW_BEFORE_SECONDS` | seconds | `2592000` (30 d) | Remaining validity threshold that triggers self-signed TLS renewal. |
| `STELLARR_TLS_RENEW_CHECK_SECONDS` | seconds | `43200` (12 h) | Polling interval for `stellarr-tls-maintenance watch`. |
| `STELLARR_MEDIA_ADVERTISE` | `ip:port` list | empty | Explicit comma-separated IPv4 ICE host candidates for `stellarr-media`, overriding derived addresses. |
| `STELLARR_MEDIA_LOG` | `tracing` filter | `info` | Rust `tracing` filter for `stellarr-media` (e.g. `info,stellarr_media=debug`). |

> [!IMPORTANT]
> Internal bearer tokens (`media-control-secret` and, when TURN is enabled, `turn-shared-secret`) are generated once under `/data/stellarr/secrets/` (`0600`). Preserve `/data/stellarr/secrets/` in all volume backups.

Fixed container-owned variables (overwritten by the entrypoint even if set in `.env`):

- **`STELLARR_DATA_ROOT`** (`/data/stellarr`): fixed volume mount point inside the image (`db/stellarr.db` lives inside it).
- **`STELLARR_API_LISTEN`** (`127.0.0.1:5000`): loopback bind reachable only by container Nginx, which appends the trusted `X-Forwarded-For` hop.
- **`STELLARR_ENVIRONMENT`** (`Production`): enforces production cookies, rate limits, and client gates (`Development` in the dev stack).

## Logs

Structured logs stream to container stdout/stderr (`docker compose logs`) and to Cockpit's 10,000-line in-memory ring buffer:

- **Request summaries** (`Stellarr.Api.Requests`): one line per API call (`POST /api/channels/…/messages 201 in 12 ms`). Routine `/health/*` probes are omitted, and invite codes, reset tokens, and unmatched path segments are masked.
- **Stdout-only security targets**: the initial [setup code](quick-start.md#4-create-the-owner-account-and-first-server) (`Stellarr.Api.Logging.StartupLog`) and object-store startup diagnostics (`Stellarr.Api.Logging.ObjectStore`) never enter Cockpit's browser-accessible buffer.
- **Categories & levels**: API categories (`Stellarr.Api.{Requests,Backups,Host,Mail,Realtime,Security,Secrets}` and `stellarr_api`) plus Rust crate targets (`hyper`, `rustls`) default to `Information`. Valid levels: `Trace`, `Debug`, `Information`, `Warning`, `Error`, `Critical`, `None`.

Override levels per category in `.env` with `Logging__LogLevel__<Category>` (or `Logging__LogLevel__Default`):

```dotenv
Logging__LogLevel__Stellarr.Api.Requests=Warning
Logging__LogLevel__stellarr_api=Debug
Logging__LogLevel__hyper=Warning
```

### Settings without a `STELLARR_*` name

> [!NOTE]
> `infra/tests/setting-names.test.sh` enforces exact parity of all 61 `STELLARR_*` names across `variables.rs`, `infra/*.sh`, `docker-compose.yml`, `.env.example`, and the tables above. Keep `.env.example` strictly to commented `# STELLARR_...=` lines because `install.sh` appends it verbatim when generating `.env`.

Low-level subsystems use case-insensitive `Section__Key` environment variables (`STELLARR_*` always wins when both exist):

| Raw key pattern | Default | Purpose |
| --- | --- | --- |
| `Oidc__Providers__<id>__*` | none | [Additional OIDC providers](#additional-oidc-providers) (`DisplayName`, `Authority`, `ClientId`, `ClientSecret`, `ClientSecretFile`, `TrustProviderMfa`, `ProviderMfaAmr`). |
| `Logging__LogLevel__<Category>` | `Information` | Per-category [log level overrides](#logs). |
| `Stellarr__Embeds__Enabled` | `true` | Background OpenGraph link previews (`MaxPerMessage=2`, `TimeoutSeconds=6`, `AllowPrivateNetworks=false`). |
| `RateLimiting__<Policy>__PermitLimit` | varies | Per-window caps: `Login` (`10`/min), `LoginPoll` (`120`/min), `AuthWrite` (`5`/15 min), `SecurityWrite` (`30`/15 min), `InvitePreview` (`30`/min), `Uploads` (`30`/min), `PinUnlock` (`5`/15 min). |
| `Maintenance__Enabled` | `true` | Hourly housekeeping worker (`InitialDelayMinutes=5`, `IntervalHours=1`, `BatchSize=1000`, `MessageChangeRetentionDays=30`). |
| `Clients__RequireIdentification` | `true` | Enforces `X-Stellarr-Client` and `X-Stellarr-Client-Version` headers (`Allowed`: `web`, `cockpit`, `desktop-linux`, `mobile-android`). |
| `Storage__Provider` | `FileSystem` | Attachment backend: `FileSystem` (`/data/stellarr/files`) or `S3` (`Storage__{Endpoint,Bucket,AccessKey,SecretKey}`). |

---

## Development stack

`infra/compose/dev.yml` runs services from bind mounts with live reload (`web` on Vite, `api` and `media` under `watchexec`, `mailpit`, and optional `coturn` under `--profile turn`). It selectively forwards safe variables from `.env` rather than loading a production `env_file` whole.

| Variable | Default | Description |
| --- | --- | --- |
| `STELLARR_DEV_WEB_PORT` | `3000` | Host TCP port for Vite (`/` and `/cockpit`). |
| `STELLARR_DEV_API_PORT` | `5000` | Host TCP port for `stellarr-api`. |
| `STELLARR_DEV_MEDIA_PORT` | `49100` | Host UDP and TCP port for `stellarr-media` on `127.0.0.1`. |
| `STELLARR_DEV_MEDIA_CONTROL_PORT` | `5200` | Host TCP port for the `stellarr-media` control API on `127.0.0.1`. |
| `STELLARR_DEV_MAILPIT_PORT` | `8025` | Host TCP port for the Mailpit web UI on `127.0.0.1`. |
| `STELLARR_DEV_MAILPIT_SMTP_PORT` | `1025` | Host TCP port for Mailpit SMTP on `127.0.0.1`. |
| `STELLARR_DEV_ALLOWED_HOSTS` | empty | Extra hostnames permitted by the Vite dev server. |
| `STELLARR_DEV_POLLING` | `true` | File-watcher polling inside Docker bind mounts (macOS/Windows). |
| `STELLARR_ACTIVITY_DEV_ENABLED` | `false` | Enables private activity pairing and voice-channel previews in `Development`. |
| `STELLARR_ACTIVITY_DEV_ORIGIN` | `http://localhost:3000` | Origin of the dev web server for activity testing. |
| `STELLARR_ACTIVITY_DEV_DOMAIN` | `activity.localhost` | Wildcard DNS suffix for dev activity frames (`*.activity.localhost`). |

To launch an isolated empty database volume alongside an existing dev checkout:

```bash
docker compose -f infra/compose/dev.yml --project-name stellarr-dev-fresh up -d
```

Native `npm run dev -w stellarr-web` reads [`web/.env.example`](../../web/.env.example) (`STELLARR_API_ORIGIN=http://127.0.0.1:5000` default).

## Running without Docker Compose

Plain `docker run` and platform templates like [Unraid](unraid.md) accept the same `STELLARR_*` variables when three container contracts are preserved:

1. **Persistent volume:** mount `/data/stellarr`.
2. **Stop timeout (`--stop-timeout=75`):** Docker's 10 s default is shorter than the API's 35 s drain window and `stellarr-init`'s 60 s ordered shutdown deadline.
3. **Matching host port variables:** containers cannot inspect host port mappings, so whenever a published host port differs from its container port, pass the matching `STELLARR_HTTPS_PORT`, `STELLARR_MEDIA_PORT`, `STELLARR_TURN_PORT`, or `STELLARR_TURNS_PORT` variable (and keep `49160-49200/udp` mapped 1:1 when TURN is enabled).

```bash
docker run -d --name stellarr --restart unless-stopped --stop-timeout=75 \
  --memory=4g --cpus=4 --pids-limit=512 --security-opt=no-new-privileges:true \
  -e STELLARR_DOMAIN=chat.example.com -e STELLARR_PUBLIC_IP=auto \
  -p 443:443/tcp -p 49000:49000/udp -p 49000:49000/tcp \
  -v stellarr_data:/data/stellarr \
  ghcr.io/arkazzae/stellarr:latest
```
