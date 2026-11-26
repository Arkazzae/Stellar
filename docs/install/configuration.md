# Configuration

Every operator setting is configured through a `STELLARR_*` environment variable using the same name across `.env`, `docker run -e`, and platform templates such as [Unraid](unraid.md). Built-in defaults are baked into the container image, so `.env` only needs to define values you override.

With Docker Compose, place settings in `.env` next to `docker-compose.yml`. Compose passes `.env` directly into the container via `env_file`; [`.env.example`](../../.env.example) lists every setting commented out with its default value. Empty or whitespace-only variables are treated as unset.

> [!IMPORTANT]
> The container reads runtime settings from `.env` (`env_file`). Variables exported only in your shell or passed via `docker compose --env-file` are used by Compose exclusively for `docker-compose.yml` interpolation (`STELLARR_VERSION`, published host ports, and container resource limits) and are not forwarded into the container environment unless written to `.env`.

Apply configuration changes by recreating the container:

```bash
docker compose up -d
```

> [!TIP]
> Variables marked **Seed** provide the initial value for a database-backed setting before the owner saves changes in Cockpit (`/cockpit`) or via the [operator CLI](../admin/cli.md). Once saved in the database, the stored value takes precedence over `.env`.

## Release

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_VERSION` | image tag | `latest` | Image tag of `ghcr.io/arkazzae/stellarr` used by Compose. Use an exact version such as `1.2.3` to pin a release, or a series such as `1.2` to follow its stable releases. |
| `STELLARR_TRACK` | release track | empty | Set by the installer when you choose `--version latest` or a stable series such as `1.2`. The installer records the resolved exact version separately in `STELLARR_VERSION`. |
| `STELLARR_IMAGE` | image reference | empty | Overrides the image reference used by Compose. The installer's `--build` sets this to `stellarr-local:dev`. |

## Address and TLS

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_DOMAIN` | hostname or IP | empty | Hostname or IP address users open in their browser. Sets the self-signed TLS Subject Alternative Name and public URL host, plus the realm when TURN is enabled; defaults to `localhost` when unset. |
| `STELLARR_PUBLIC_IP` | IPv4 or `auto` | empty | External IPv4 address advertised by `stellarr-media` and, when enabled, `coturn`. Use a reachable IPv4 for remote calls. With this variable unset, a numeric IPv4 `STELLARR_DOMAIN` supplies the direct SFU address. `auto` resolves the domain’s IPv4 address at container startup. |
| `STELLARR_HTTPS_PORT` | port | `443` | Host TCP port mapped to container port `443/tcp`. Used for HTTP-to-HTTPS redirects and for constructing the default public URL. |
| `STELLARR_HTTP_PORT` | port or `ip:port` | `127.0.0.1:8888` | Host TCP binding for the local HTTP reverse-proxy upstream on container port `80/tcp`. Read by Compose only. |
| `STELLARR_PUBLIC_PORT` | port | `STELLARR_HTTPS_PORT` | External HTTPS port browsers connect to (`443` when behind a [reverse proxy](reverse-proxy-and-tls.md#run-behind-a-reverse-proxy)). Omitted from generated URLs when set to `443`. |
| `STELLARR_PUBLIC_URL` | origin URL | empty | Explicit public origin (`http://` or `https://`, without credentials, query, or fragment), overriding the URL derived from `STELLARR_DOMAIN` and `STELLARR_PUBLIC_PORT`. |
| `STELLARR_ACTIVITY_DOMAIN` | DNS suffix | empty | Serves installed activities from dedicated hostnames under this suffix. The instance hostname must be outside the suffix. Route wildcard DNS and trusted wildcard TLS (for example, `*.activities.example.com`) to the same Stellarr instance. |
| `STELLARR_ACTIVITY_ORIGIN` | HTTPS origin | instance public URL | Origin of the Stellarr web client that may embed activity frames. Activity hosts use this origin's public HTTPS port. |
| `STELLARR_TRUSTED_PROXIES` | IP / CIDR list | empty | Comma- or space-separated IPv4/IPv6 addresses or CIDR networks of upstream [reverse proxies](reverse-proxy-and-tls.md#client-addresses). |

- **Certificates**: When `/data/stellarr/tls/fullchain.pem` or `privkey.pem` is absent, the container generates a 365-day self-signed certificate and renews it automatically when `STELLARR_DOMAIN` changes or fewer than 30 days remain. Mounted CA-signed certificates are never overwritten; see [Reverse proxy and TLS](reverse-proxy-and-tls.md).
- **Public URL**: Used for password-reset links, email-change confirmation links, invite URLs, `/.well-known/stellarr/client`, and Fido2 passkey origin validation. When both `STELLARR_DOMAIN` and `STELLARR_PUBLIC_URL` are unset, the instance has no public address and password-reset emails remain disabled.
- **Trusted proxies**: The API reads `X-Forwarded-For` from right to left, skipping addresses listed in `STELLARR_TRUSTED_PROXIES`, to determine the real client IP for rate limiting, session lists, security events, and trusted devices. Malformed entries (hostnames, CIDR blocks with non-zero host bits such as `10.0.0.1/24`, or `0.0.0.0/0`) abort API startup.

## Voice and RTC

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_MEDIA_PORT` | port | `49000` | Host UDP and TCP port for Stellarr Media. Forward the same external port for both transports. [Stellarr Media](../stellarr-media.md) |
| `STELLARR_MEDIA_LAN_IP` | IPv4 address | empty | Optional host LAN address advertised alongside the public address on `STELLARR_MEDIA_PORT`. Useful for callers on the local network when the router cannot loop traffic back through its public address. |
| `STELLARR_TURN_ENABLED` | `true` \| `false` | `false` | Enables the optional TURN relay. Include [the TURN overlay](../rtc-and-ports.md#optional-turn-relay) with the base Compose file to publish its ports. |
| `STELLARR_TURN_PORT` | port | `3478` | Host UDP/TCP port for STUN and TURN advertised to browsers. |
| `STELLARR_TURNS_PORT` | port | `5349` | Host TCP/UDP port for TURN over TLS and DTLS. |
| `STELLARR_TURN_RELAY_RANGE` | port range | `49160-49200` | UDP port range used by coturn for media relays and mapped 1:1 by the optional Compose override. |
| `STELLARR_TURN_TCP_ENABLED` | `true` \| `false` | `false` | Enables the `coturn` TCP listener and advertises `?transport=tcp` TURN URLs. |
| `STELLARR_TURNS_ENABLED` | `true` \| `false` | `false` | Enables TURN over TLS and DTLS (`turns:`). Requires a CA-signed certificate trusted by browsers. |

Without TURN the API returns an empty `rtc.iceServers` list and browsers use the SFU candidates in SDP. Disabled TURN transports are turned off in `/etc/turnserver.conf` (`no-tcp`, `no-tls`, `no-dtls`) as well as omitted from `/api/bootstrap`. See [RTC and ports](../rtc-and-ports.md) for relay sizing and firewall rules.

## Uploads and quotas

Set the per-account quota in **Cockpit → Settings → File storage**. The saved value applies to each account across the instance, including Discord attachment imports, and takes effect on the next upload. The value in Cockpit overrides `STELLARR_UPLOAD_USER_BYTES`, which supplies the default until a value is saved.

Set byte limits to `0` (or a negative integer) to disable the corresponding quota check.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_UPLOAD_USER_BYTES` | bytes | `2147483648` (2 GiB) | Maximum total upload storage per user account. |
| `STELLARR_UPLOAD_TOTAL_BYTES` | bytes | `21474836480` (20 GiB) | Maximum total upload storage across the entire instance. |
| `STELLARR_MIN_FREE_BYTES` | bytes | `1073741824` (1 GiB) | Minimum free disk space that must remain on `/data/stellarr` after an upload. |
| `STELLARR_MAX_CONCURRENT_UPLOADS` | integer (`1–32`) | `4` | Maximum concurrent uploads processed by the API. |

Per-file size limits by media type: images `10 MiB` (plus optional `1 MiB` preview), audio `25 MiB`, general files `25 MiB`, and video `100 MiB` (plus optional `2 MiB` poster). Nginx and the API reject request bodies larger than `128 MiB`.

## Mail

Password-reset links, email 2FA codes, email-change confirmations, security alerts, and opt-in activity notifications are sent via SMTP (configured in **Cockpit → Settings**) or Resend (enabled in **Cockpit → Integrations**).

> [!NOTE]
> Secrets saved in Cockpit (`mail.smtp-password`, `mail.resend-api-key`, `giphy.api-key`, and `cloudflare.r2.*`) are sealed with AES-256-GCM under keys derived from `/data/stellarr/keys/master.key`, override `.env`, and are never returned to browsers. Clearing a secret in Cockpit or via `stellarr secrets clear` restores the `.env` fallback.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_MAIL_ENABLED` | `true` \| `false` | `true` | **Seed** for Cockpit's master mail delivery toggle (`false` when the API runs in the development environment, outside the container). |
| `STELLARR_MAIL_FROM` | email | empty | **Seed** for the sender email address (`onboarding@resend.dev` is valid only with Resend). |
| `STELLARR_MAIL_FROM_NAME` | string | `Stellarr` | Display name used in the `From` header of outgoing emails. |
| `STELLARR_SMTP_PASSWORD` | string | empty | Fallback SMTP password when no password is stored in Cockpit. |
| `STELLARR_RESEND_API_KEY` | string | empty | Fallback Resend API key when no key is stored in Cockpit. |

## Web Push

Browser push notifications for channel mentions and direct messages remain disabled until a persistent P-256 VAPID key pair is configured. See [Activity notifications](../admin/notifications.md#web-push) for key generation instructions; rotating VAPID keys requires browsers to re-subscribe.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_PUSH_ENABLED` | `true` \| `false` | `false` | Enables Web Push delivery when a valid VAPID key pair and subject are configured. |
| `STELLARR_PUSH_SUBJECT` | `mailto:` or `https://` URI | empty | Operator contact URI sent to web push services (e.g. `mailto:admin@chat.example.com`). |
| `STELLARR_PUSH_PUBLIC_KEY` | base64url | empty | Uncompressed P-256 VAPID public key (65 bytes decoded). |
| `STELLARR_PUSH_PRIVATE_KEY` | base64url | empty | P-256 VAPID private key (32 bytes decoded). Keep secret. |

## Single sign-on (OIDC)

Enables OpenID Connect sign-in alongside local passwords. The default provider callback URL is `<public-url>/api/auth/oidc/callback`.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_OIDC_ENABLED` | `true` \| `false` | `false` | Enables the default OIDC provider (requires issuer, client ID, and client secret). |
| `STELLARR_OIDC_ISSUER` | HTTPS URL | empty | OIDC provider issuer/authority URL (must use `https://`). |
| `STELLARR_OIDC_CLIENT_ID` | string | empty | OIDC relying-party client ID. |
| `STELLARR_OIDC_CLIENT_SECRET` | string | empty | OIDC client secret (takes precedence over `STELLARR_OIDC_CLIENT_SECRET_FILE`). |
| `STELLARR_OIDC_CLIENT_SECRET_FILE` | container path | empty | Path inside the container to a file containing the OIDC client secret. |
| `STELLARR_OIDC_DISPLAY_NAME` | string | `Main account` (`en`) / `Konto główne` (`pl`) | Sign-in button label shown on `/login` and `/register`. |
| `STELLARR_OIDC_TRUST_PROVIDER_MFA` | `true` \| `false` | `false` | Accepts an OIDC sign-in with a matching `amr` claim and recent `auth_time` as satisfying Stellarr's second factor. |
| `STELLARR_OIDC_PROVIDER_MFA_AMR` | comma-separated list | `mfa` | `amr` claim values accepted when `STELLARR_OIDC_TRUST_PROVIDER_MFA=true`. |

### Additional OIDC providers

You can register multiple OIDC providers in `.env` using raw configuration keys (`Oidc__Providers__<id>__*`). Each provider `<id>` must be `1–32` lowercase letters, digits, or hyphens (excluding `default`, `login`, `callback`, `link`, `reauthenticate`, and `pending`) and uses the callback URL `<public-url>/api/auth/oidc/<id>/callback`:

```dotenv
Oidc__Providers__work__DisplayName=Work account
Oidc__Providers__work__Authority=https://id.example.com/realms/work
Oidc__Providers__work__ClientId=stellarr
Oidc__Providers__work__ClientSecretFile=/run/secrets/oidc-work
# Optional (false by default):
Oidc__Providers__work__TrustProviderMfa=true
Oidc__Providers__work__ProviderMfaAmr=mfa,hwk
```

- A provider is active only when its `Authority` is an `https://` URL and both `ClientId` and `ClientSecret` (or `ClientSecretFile`) are set. Invalid provider IDs or custom `Oidc__CallbackPath` overrides abort API startup (callback paths are fixed per provider).
- Users link identities under **Settings → Login and security → Sign-in providers**. Identities match strictly by `(issuer, subject)`, never by email address.
- When an unlinked provider identity signs in, Stellarr holds the verified identity in a 10-minute browser-bound cookie at `/link-account`, allowing the user to either sign in with an existing password to link the account or (if enabled for that provider in **Cockpit → Integrations → Sign-in providers** and the user holds an invite) create a passwordless account. See [Account security](../admin/account-security.md#linking-an-identity-provider).
- Stellarr requests `openid profile email` scopes and reads `name`, `preferred_username`, `email`, and `email_verified` from the ID token or userinfo endpoint.
- Passwordless accounts confirm sensitive account changes by re-authenticating at their linked provider (`prompt=login`, `max_age=300`); the 5-minute freshness window is measured from the provider's `auth_time` claim.

### Trusting provider MFA

By default, OIDC sign-in still prompts for Stellarr's own second factor when required by the account or instance MFA policy. When `TrustProviderMfa` is `true`, the provider's MFA satisfies both sign-in and Cockpit's fresh second-step check if:

- The ID token includes at least one configured `ProviderMfaAmr` value in its `amr` claim, and
- The ID token carries an `auth_time` timestamp within the last 5 minutes (`max_age=300` on re-authentication).

If `auth_time` is missing or older than 5 minutes, Stellarr falls back to prompting for its own second factor. Enable this only for providers that enforce a genuine second factor and report it in `amr`.

## Automatic backups and Cloudflare R2

Production containers run an automatic full backup every 24 hours by default. You can adjust the schedule live in **Cockpit → Backups → Backup settings**; see [Automatic backups](../admin/backups.md) and [Cloudflare R2 offsite backups](../admin/cloudflare-r2.md).

### Backup schedule

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_BACKUP_ENABLED` | `true` \| `false` | `true` | **Seed** for automatic full backups. |
| `STELLARR_BACKUP_INTERVAL_HOURS` | integer (`1–720`) | `24` | **Seed** for hours between completed backup runs. |
| `STELLARR_BACKUP_RETRY_MINUTES` | integer (`1–1440`) | `15` | **Seed** for minutes before retrying a failed backup or offsite upload. |
| `STELLARR_BACKUP_INITIAL_DELAY_MINUTES` | integer (`0–1440`) | `5` | **Seed** for minutes after container startup before the first scheduled backup check. |

### Cloudflare R2 integration

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_CLOUDFLARE_R2_ENABLED` | `true` \| `false` | `false` | **Seed** enabling offsite upload of completed full backups to Cloudflare R2. |
| `STELLARR_CLOUDFLARE_R2_MODE` | `s3` \| `worker` | `s3` | Upload directly via the R2 S3 API (`s3`) or via the Backup Relay Worker (`worker`). |
| `STELLARR_CLOUDFLARE_R2_ENDPOINT` | HTTPS URL | empty | R2 S3 endpoint (`https://<ACCOUNT_ID>.r2.cloudflarestorage.com`) or Backup Relay Worker URL. |
| `STELLARR_CLOUDFLARE_R2_REGION` | string | `auto` | S3 signing region used in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_BUCKET` | string | empty | Private R2 bucket name used in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_ACCESS_KEY` | string | empty | R2 S3 access key ID used in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_SECRET_KEY` | string | empty | R2 S3 secret access key used in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_WORKER_TOKEN` | string | empty | Bearer token (`BACKUP_TOKEN`) used in `worker` mode. |
| `STELLARR_CLOUDFLARE_R2_PREFIX` | string | `stellarr` | Object key prefix inside the R2 bucket (`<prefix>/<server-uuid>/backup-...`). |

## Language

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_LANGUAGE` | `en` \| `pl` | `en` | Default web client locale (before a user picks one) and the server locale for API errors, system messages, new server/channel/role names, audit entries, and emails. |

Any value other than `en` or `pl` aborts API startup. Existing database records keep the language they were created with. Browsers cache `/stellarr-instance.js` for up to 1 hour (with background Service Worker revalidation). See [Localization](../localization.md).

## Instance identity and desktop handshake

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_INSTANCE_ID` | UUID | generated | Stable instance UUID published at `/.well-known/stellarr/client`. Generated once and persisted in `/data/stellarr/instance-id` when unset. |
| `STELLARR_TIME_ZONE` | IANA time zone | `UTC` (container) | **Seed** for the instance time zone in Cockpit (e.g. `Europe/Warsaw`). |
| `STELLARR_DESKTOP_MINIMUM_VERSION` | semver | `0.1.0` | **Seed** for the minimum desktop client version allowed by the client gate (`X-Stellarr-Client: desktop-linux`). |
| `STELLARR_DESKTOP_RECOMMENDED_VERSION` | semver | `0.1.3` | **Seed** for the recommended desktop client version advertised by `/.well-known/stellarr/client`. |

## Other settings

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_GIPHY_API_KEY` | string | empty | Fallback public GIPHY web API key for GIF search when no key is saved in **Cockpit → Integrations**. |

## Logs

The API writes structured logs to container stdout/stderr (`docker compose logs`) and to Cockpit's in-memory **Logs** buffer (retaining the latest 10,000 entries). Default log output includes:

- Information, warnings and errors from every component
- One summary line per API request (`POST /api/channels/…/messages 201 in 12 ms` under `Stellarr.Api.Requests`), excluding routine `/health/ready` and `/health/live` probes. Secrets in request paths (such as invite codes and confirmation/reset tokens) and unrecognized segments on unmatched routes are masked before logging.
- Operational milestones: startup, media server epoch resets, backup jobs, and hourly maintenance.
- The one-time owner [setup code](quick-start.md#4-create-the-owner-account-and-first-server) on startup while the instance has zero accounts (`Stellarr.Api.Logging.StartupLog`), plus any object-store readiness warnings (`Stellarr.Api.Logging.ObjectStore`). Both targets write only to container stdout/stderr and are omitted from Cockpit's in-memory log buffer so authenticated browser sessions cannot read setup codes or upstream bucket error strings.

Every category logs at `Information` and above unless set otherwise. A category is the name a line is logged under, which is what Cockpit's **Logs** page shows as its source: the API's own are `Stellarr.Api.Requests` (the request lines), `Stellarr.Api.Backups`, `Stellarr.Api.Host`, `Stellarr.Api.Mail`, `Stellarr.Api.Realtime`, `Stellarr.Api.Security` and `Stellarr.Api.Secrets`; the libraries it uses log under their crate names, such as `hyper` or `rustls`, and `stellarr_api` is every other module of the API. A level is one of `Trace`, `Debug`, `Information`, `Warning`, `Error`, `Critical` or `None`. Names of ASP.NET or Entity Framework categories (`Microsoft.AspNetCore` and the like) mean nothing to this server.

A category has no `STELLARR_*` name, so it is set with the raw key `Logging__LogLevel__<category>` in `.env`, and `Logging__LogLevel__Default` sets the level for the rest:

```dotenv
Logging__LogLevel__Stellarr.Api.Requests=Warning
Logging__LogLevel__stellarr_api=Debug
Logging__LogLevel__hyper=Warning
```

## Container resource limits

Read by `docker-compose.yml` during container creation (for `docker run`, pass `--memory`, `--cpus`, and `--pids-limit`):

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_CONTAINER_MEMORY_LIMIT` | size | `4g` | Container memory limit (`mem_limit`). |
| `STELLARR_CONTAINER_CPUS` | number | `4` | Container CPU quota (`cpus`). |
| `STELLARR_CONTAINER_PIDS_LIMIT` | integer | `512` | Container process limit (`pids_limit`). |

## Advanced

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_TLS_RENEW_BEFORE_SECONDS` | seconds | `2592000` (30 days) | Renews the container-managed self-signed TLS certificate when remaining validity drops below this threshold or `STELLARR_DOMAIN` changes. |
| `STELLARR_TLS_RENEW_CHECK_SECONDS` | seconds | `43200` (12 hours) | Interval between self-signed certificate renewal checks (`stellarr-tls-maintenance watch`). |
| `STELLARR_MEDIA_ADVERTISE` | `ip:port` list | empty | Explicit comma-separated IPv4 `ip:port` ICE host candidates for `stellarr-media`, overriding the derived public, LAN and container addresses. |
| `STELLARR_MEDIA_LOG` | `tracing` filter | `info` | Rust `tracing` log filter for `stellarr-media` (e.g. `info,stellarr_media=debug`). |

> [!IMPORTANT]
> The container generates `media-control-secret` on first start and `turn-shared-secret` when TURN is first enabled. Both are stored in `/data/stellarr/secrets/`. The container reads these secrets from their files. Preserve `/data/stellarr/secrets/` in all volume backups.

The container data directory is fixed at `/data/stellarr` inside the image (`STELLARR_DATA_ROOT`). The database is `db/stellarr.db` in it, and has no setting of its own.

Two more names belong to the image and are not settings. `STELLARR_API_LISTEN` is the address the API listens on, always `127.0.0.1:5000`, where only nginx reaches it: the API takes the client's address from the last `X-Forwarded-For` entry, which only nginx writes. `STELLARR_ENVIRONMENT` is the environment the API runs in, always `Production` in the container; the development stack sets `Development`. The entrypoint assigns both whatever `.env` holds, so a value there changes nothing.

### Settings without a `STELLARR_*` name

> [!NOTE]
> Every operator-facing `STELLARR_*` variable is kept in lockstep across `api/crates/server/src/settings/variables.rs` (or `infra/{entrypoint,settings,tls-maintenance}.sh`), `docker-compose.yml`, `.env.example`, and the tables above (`infra/tests/setting-names.test.sh`). In `.env.example`, every entry must remain commented out (`# STELLARR_...=`) and must not include raw `Section__Key` settings because the installer appends `.env.example` directly to the `.env` file it writes.

Advanced internal subsystems that rarely need tuning can be configured in `.env` using raw configuration keys (`Section__Key`, where `__` stands for `:`; names are not case-sensitive). For any setting that has a `STELLARR_*` variable, the `STELLARR_*` variable always takes precedence over the raw key:

| Raw key pattern | Default | Purpose |
| --- | --- | --- |
| `Oidc__Providers__<id>__*` | none | [Additional OIDC providers](#additional-oidc-providers) (`DisplayName`, `Authority`, `ClientId`, `ClientSecret`, `ClientSecretFile`, `TrustProviderMfa`, `ProviderMfaAmr`). |
| `Logging__LogLevel__<Category>` | `Information` | Per-category [log level overrides](#logs). |
| `Stellarr__Embeds__Enabled` | `true` | Enables background fetching of OpenGraph link previews for channel messages (`MaxPerMessage=2`, `TimeoutSeconds=6`, `AllowPrivateNetworks=false`). Leave `AllowPrivateNetworks=false` unless you intentionally want members to unfurl intranet addresses. When `Enabled=false`, YouTube and Vimeo links still render player stubs without outbound requests. |
| `RateLimiting__<Policy>__PermitLimit` | varies | Per-window request limit for `Login` (`10`/min), `LoginPoll` (`120`/min), `AuthWrite` (`5`/15 min), `SecurityWrite` (`30`/15 min), `InvitePreview` (`30`/min), `Uploads` (`30`/min instance-wide), and `PinUnlock` (`5`/15 min per IP and account). |
| `Maintenance__Enabled` | `true` | Hourly housekeeping worker (`InitialDelayMinutes=5`, `IntervalHours=1`, `BatchSize=1000`, `MessageChangeRetentionDays=30`). |
| `Clients__RequireIdentification` | `true` | Requires `X-Stellarr-Client` and `X-Stellarr-Client-Version` headers on gated auth/bootstrap routes (`Allowed`: `web`, `cockpit`, `desktop-linux`, `mobile-android`). |
| `Storage__Provider` | `FileSystem` | Object storage backend (`FileSystem` under `/data/stellarr/files`, or `S3` with `Storage__Endpoint`, `Storage__Bucket`, `Storage__AccessKey`, `Storage__SecretKey`). |

## Development stack

`infra/compose/dev.yml` runs each service from a bind mount with live reload (`web` on Vite, `api` on `cargo run` under `watchexec`, `media` on `cargo run` under `watchexec`, `mailpit` for local email capture, and optional `coturn` under `--profile turn`). It does not load `env_file: .env` whole so a production `.env` in the checkout cannot collide with development services; instead it forwards selected variables (`STELLARR_LANGUAGE`, `STELLARR_TIME_ZONE`, `STELLARR_INSTANCE_ID`, desktop versions, `STELLARR_GIPHY_API_KEY`, mail sender/secrets, and Web Push keys) from the shell or `.env`.

| Variable | Default | Description |
| --- | --- | --- |
| `STELLARR_DEV_WEB_PORT` | `3000` | Host TCP port for Vite's dev server (serving both the web client at `/` and Cockpit at `/cockpit`). |
| `STELLARR_DEV_API_PORT` | `5000` | Host TCP port for the API under `cargo run`. |
| `STELLARR_DEV_MEDIA_PORT` | `49100` | Host UDP and TCP port for `stellarr-media` (advertised on `127.0.0.1`). |
| `STELLARR_DEV_MEDIA_CONTROL_PORT` | `5200` | Host TCP port for the `stellarr-media` control API on `127.0.0.1`. |
| `STELLARR_DEV_MAILPIT_PORT` | `8025` | Host TCP port for the Mailpit web UI on `127.0.0.1`. |
| `STELLARR_DEV_MAILPIT_SMTP_PORT` | `1025` | Host TCP port for Mailpit's SMTP listener on `127.0.0.1`. |
| `STELLARR_DEV_ALLOWED_HOSTS` | empty | Comma-separated additional hostnames accepted by Vite's dev server (`localhost` and IP addresses are always allowed). |
| `STELLARR_DEV_POLLING` | `true` | Enables file-watcher polling for Vite inside Docker bind mounts (macOS/Windows). |

Use a separate Compose project to start the dev stack with its own empty database volume. The default ports must be free.

```bash
docker compose -f infra/compose/dev.yml --project-name stellarr-dev-fresh up -d
```

Running `npm run dev` in `web/` outside Docker reads [`web/.env.example`](../../web/.env.example) (including `STELLARR_API_ORIGIN`, `http://127.0.0.1:5000` by default).

## Running without Docker Compose

Plain `docker run` and platform templates such as [Unraid](unraid.md) use the exact same `STELLARR_*` variable names:

- **Volume**: Mount a persistent volume or host directory at `/data/stellarr`.
- **Stop timeout**: Pass `--stop-timeout=75` so `stellarr-init` can complete its 60-second ordered shutdown before Docker sends `SIGKILL` (Docker's 10 s default is shorter than the API's 35 s stop deadline).
- **Host port variables**: Because a container cannot inspect host port mappings on its own, set the corresponding variable whenever a published host port differs from the container port: `STELLARR_HTTPS_PORT` (container `443/tcp`), `STELLARR_MEDIA_PORT` (container `49000/udp` and `49000/tcp`), and, when enabling TURN, `STELLARR_TURN_PORT` (container `3478`) and `STELLARR_TURNS_PORT` (container `5349`). Keep the optional TURN relay range `49160-49200/udp` mapped 1:1.

```bash
docker run -d --name stellarr --restart unless-stopped --stop-timeout=75 \
  --memory=4g --cpus=4 --pids-limit=512 --security-opt=no-new-privileges:true \
  -e STELLARR_DOMAIN=chat.example.com -e STELLARR_PUBLIC_IP=auto \
  -p 443:443/tcp -p 49000:49000/udp -p 49000:49000/tcp \
  -v stellarr_data:/data/stellarr \
  ghcr.io/arkazzae/stellarr:latest
```
