# Configuration

`docker-compose.yml` configures the container through environment variables read from `.env` (copy `.env.example` to `.env` to start). Apply changes with `docker compose up -d`.

> [!TIP]
> Variables marked **seed** initialize database settings that the owner can change later in Cockpit (`/cockpit`) or via the [operator CLI](../admin/cli.md). Once saved in Cockpit or the CLI, the database value takes precedence over `.env`.

## Address and TLS

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_DOMAIN` | hostname or IP | empty | Public hostname or IP used for the origin, TURN realm, and self-signed certificate SAN. Falls back to `localhost` when unset. |
| `STELLARR_PUBLIC_PORT` | port | `STELLARR_HTTPS_PORT` | External HTTPS port browsers connect to (`443` behind a [reverse proxy](reverse-proxy-and-tls.md#run-behind-a-reverse-proxy)). Omitted from URLs when `443`. |
| `PUBLIC_IP` | IPv4 or `auto` | empty | Public IPv4 advertised by `coturn` and `stellarr-media`. Required for voice from outside the Docker host; `auto` resolves `STELLARR_DOMAIN`'s `A` record on startup. |
| `STELLARR_HTTPS_PORT` | port (`1–65535`) | `8443` | Host port mapped to container `443/tcp`. |
| `STELLARR_HTTP_PORT` | port or `ip:port` | `8888` | Host port mapped to container `80/tcp` (HTTP-to-HTTPS redirect only). Prefix with `127.0.0.1:` when using a local reverse proxy. |
| `STELLARR_PUBLIC_URL` | origin URL | empty | Explicit `http` or `https` origin (no credentials, query, or fragment), overriding `STELLARR_DOMAIN` + `STELLARR_PUBLIC_PORT`. |
| `STELLARR_TRUSTED_PROXIES` | IP / CIDR list | empty | Comma- or space-separated IPv4/IPv6 addresses or CIDRs of trusted [reverse proxies](reverse-proxy-and-tls.md#client-addresses). |
| `STELLARR_VERSION` | image tag | `latest` | Image tag for `ghcr.io/arkazzae/stellarr` (pin `1.2` or `1.2.3` for production). |

Notes:

- **Self-signed vs. custom certificates**: When `/data/stellarr/tls/fullchain.pem` or `privkey.pem` is missing, the container generates a self-signed certificate and renews it if `STELLARR_DOMAIN` changes or <30 days remain. Custom CA-signed certificates are never overwritten; see [Reverse proxy and TLS](reverse-proxy-and-tls.md).
- **Public origin behavior**: `STELLARR_PUBLIC_URL` (or the URL built from `STELLARR_DOMAIN` + `STELLARR_PUBLIC_PORT`) is used for password-reset and email-change links, invite links, `/.well-known/stellarr/client`, and passkey Relying Party validation. When both `STELLARR_DOMAIN` and `STELLARR_PUBLIC_URL` are unset, password-reset emails remain disabled.
- **Trusted proxies**: Listing a proxy tells the API to read real client IPs from `X-Forwarded-For` for rate limits, sessions, security events, and trusted devices. Invalid entries (hostnames, non-zero host bits such as `10.0.0.1/24`, or `0.0.0.0/0`) abort API startup.

## Voice and RTC

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_MEDIA_PORT` | port (`1–65535`) | `49000` | Host UDP port for [Stellarr Media](../stellarr-media.md); carries every call and must match the external router port. |
| `STELLARR_TURN_PORT` | port (`1–65535`) | `3478` | Published host port and advertised port for STUN/TURN (`udp` and `tcp`). |
| `STELLARR_TURNS_PORT` | port (`1–65535`) | `5349` | Published host port and advertised port for TURN over TLS/DTLS. |
| `STELLARR_TURN_TCP_ENABLED` | `true` \| `false` | `false` | Enables the TURN/TCP listener in `coturn` and advertises `?transport=tcp` ICE URLs. |
| `STELLARR_TURNS_ENABLED` | `true` \| `false` | `false` | Enables TURN over TLS/DTLS (`turns:`). Requires a browser-trusted TLS certificate. |
| `STELLARR_TURN_RELAY_RANGE` | port range | `49160-49200` | Host UDP ports mapped 1:1 to container `49160-49200/udp` for TURN media relays. |

See [RTC and ports](../rtc-and-ports.md) for relay sizing and firewall rules.

## Uploads and quotas

Byte limits accept `0` (or negative values) to disable the quota check.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_UPLOAD_USER_BYTES` | bytes | `2147483648` (2 GiB) | Per-account storage quota. |
| `STELLARR_UPLOAD_TOTAL_BYTES` | bytes | `21474836480` (20 GiB) | Instance-wide storage quota. |
| `STELLARR_MIN_FREE_BYTES` | bytes | `1073741824` (1 GiB) | Minimum free disk space required on `/data/stellarr` to accept an upload (`0` disables). |
| `STELLARR_MAX_CONCURRENT_UPLOADS` | integer (`1–32`) | `4` | Maximum concurrent uploads across the instance. |

Per-file limits by media type: images `10 MiB`, audio `25 MiB`, documents/other `25 MiB`, video `100 MiB`. Nginx enforces a `128 MiB` maximum request body size.

## Mail

Password-reset, invitation, security alert, and activity notification emails use SMTP by default. Configure the SMTP host, port, encryption (`STARTTLS`, `TLS`, or `none`), and username in Cockpit under **Settings**, or enable Resend under **Integrations**.

> [!NOTE]
> Secrets saved in Cockpit (`SMTP password`, `Resend API key`) are encrypted with the instance's data-protection keys, override `.env`, and are never returned to the browser. Clearing a secret in Cockpit falls back to the environment variable.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_MAIL_ENABLED` | `true` \| `false` | `true` | **Seed** for the mailing toggle in Cockpit settings. |
| `STELLARR_SMTP_PASSWORD` | string | empty | SMTP password fallback when none is stored in Cockpit. |
| `STELLARR_RESEND_API_KEY` | string | empty | Resend API key fallback when none is stored in Cockpit. |
| `STELLARR_MAIL_FROM` | email | empty | **Seed** for the sender address (`onboarding@resend.dev` works only via Resend testing). |
| `STELLARR_MAIL_FROM_NAME` | string | `Stellarr` | Sender display name. |

## Web Push

Optional browser push notifications for mentions and direct messages. See [Activity notifications](../admin/notifications.md#web-push) for generating a persistent VAPID key pair.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_PUSH_ENABLED` | `true` \| `false` | `false` | Enables Web Push when a valid VAPID key pair and subject are configured. |
| `STELLARR_PUSH_SUBJECT` | `mailto:` or `https://` URI | empty | Operator contact URI presented to push services (for example, `mailto:admin@example.com`). |
| `STELLARR_PUSH_PUBLIC_KEY` | base64url string | empty | Uncompressed P-256 VAPID public key (65 bytes decoded). |
| `STELLARR_PUSH_PRIVATE_KEY` | base64url string | empty | P-256 VAPID private key (32 bytes decoded). Treat as a secret. |

## Single sign-on (OIDC)

OpenID Connect sign-in is optional; local username/password sign-in remains available. The callback URL is `<public-url>/api/auth/oidc/callback`.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_OIDC_ENABLED` | `true` \| `false` | `false` | Enables the primary OIDC provider (requires issuer, client ID, and client secret). |
| `STELLARR_OIDC_ISSUER` | HTTPS URL | empty | OIDC provider issuer URL. |
| `STELLARR_OIDC_CLIENT_ID` | string | empty | OIDC client ID. |
| `STELLARR_OIDC_CLIENT_SECRET` | string | empty | OIDC client secret (takes precedence over `STELLARR_OIDC_CLIENT_SECRET_FILE`). |
| `STELLARR_OIDC_CLIENT_SECRET_FILE` | container path | empty | Path to a mounted file containing the OIDC client secret. |
| `STELLARR_OIDC_DISPLAY_NAME` | string | `Main account` | Sign-in button label on the login screen. |
| `STELLARR_OIDC_TRUST_PROVIDER_MFA` | `true` \| `false` | `false` | Counts a provider login confirmed with a matching `amr` claim as Stellarr's second step. |
| `STELLARR_OIDC_PROVIDER_MFA_AMR` | comma-separated list | `mfa` | `amr` claim values treated as provider MFA when `STELLARR_OIDC_TRUST_PROVIDER_MFA=true`. |

### Additional OIDC providers

You can configure extra OIDC providers in `docker-compose.override.yml` under `environment`, keyed by a provider ID of `1–32` lowercase letters, digits, or hyphens (excluding `default`, `login`, and `callback`). Each provider gets its own login button and callback URL `<public-url>/api/auth/oidc/<id>/callback`:

```yaml
services:
  stellarr:
    environment:
      Oidc__Providers__work__DisplayName: Work account
      Oidc__Providers__work__Authority: https://id.example.com/realms/work
      Oidc__Providers__work__ClientId: stellarr
      Oidc__Providers__work__ClientSecretFile: /run/secrets/oidc-work
      # Optional, off by default:
      Oidc__Providers__work__TrustProviderMfa: "true"
      Oidc__Providers__work__ProviderMfaAmr: mfa,hwk
```

- A provider is enabled only when its issuer is HTTPS and both client ID and secret are configured; an invalid provider ID stops the API at startup.
- Users link their accounts to a provider from **Settings → Login and security → Sign-in providers** (see [Account security](../admin/account-security.md#linking-an-identity-provider)). Identities are matched strictly by `(issuer, subject)`, never by email address.

### Trusting provider MFA

By default, an OIDC sign-in still prompts for Stellarr's own second step if MFA is active on the account or instance policy. Enabling `TrustProviderMfa` allows the provider's own MFA check to satisfy Stellarr's second step (including Cockpit):

- The provider's ID token must contain one of the configured `amr` values and an `auth_time` within the last 5 minutes (Stellarr sends `max_age=300` when re-authenticating).
- If `auth_time` is missing or stale, Stellarr falls back to its own second-step prompt. Enable this only for providers that enforce a real second factor and report it accurately in `amr`.

## Automatic backups and Cloudflare R2

Production Docker Compose enables daily automatic full backups by default. Schedule settings can be overridden at runtime in **Cockpit → Backups → Backup settings**; see [Automatic backups](../admin/backups.md) and [Cloudflare R2 offsite backups](../admin/cloudflare-r2.md).

### Backup schedule

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_BACKUP_ENABLED` | `true` \| `false` | `true` | **Seed** enabling automatic full backups. |
| `STELLARR_BACKUP_INTERVAL_HOURS` | integer | `24` | Hours between completed backup cycles. |
| `STELLARR_BACKUP_RETRY_MINUTES` | integer | `15` | Delay before retrying a failed offsite upload. |
| `STELLARR_BACKUP_INITIAL_DELAY_MINUTES` | integer | `5` | Delay after container startup before the first scheduled backup runs. |

### Cloudflare R2 integration

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_CLOUDFLARE_R2_ENABLED` | `true` \| `false` | `false` | Enables uploading completed backups to Cloudflare R2. |
| `STELLARR_CLOUDFLARE_R2_MODE` | `s3` \| `worker` | `s3` | Upload mode: direct S3 API (`s3`) or Backup Relay Worker (`worker`). |
| `STELLARR_CLOUDFLARE_R2_ENDPOINT` | HTTPS URL | empty | R2 S3 endpoint (`https://<ACCOUNT_ID>.r2.cloudflarestorage.com`) or Worker URL. |
| `STELLARR_CLOUDFLARE_R2_REGION` | string | `auto` | R2 region (`s3` mode). |
| `STELLARR_CLOUDFLARE_R2_BUCKET` | string | empty | Private R2 bucket name (`s3` mode). |
| `STELLARR_CLOUDFLARE_R2_ACCESS_KEY` | string | empty | R2 S3 access key ID (`s3` mode). |
| `STELLARR_CLOUDFLARE_R2_SECRET_KEY` | string | empty | R2 S3 secret access key (`s3` mode). |
| `STELLARR_CLOUDFLARE_R2_WORKER_TOKEN` | string | empty | Bearer token for the Backup Relay Worker (`worker` mode). |
| `STELLARR_CLOUDFLARE_R2_PREFIX` | string | `stellarr` | Object key prefix inside the bucket. |

## Language

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_LANGUAGE` | `en` \| `pl` | `en` | Default web client locale (for users without a saved browser preference) and API language for error messages, system messages, default server/role/channel names, audit entries, and emails. |

Unknown values abort API startup. Existing database records keep the language in which they were created. Browsers cache `/stellarr-instance.js` for up to one hour, so a changed default may take up to an hour to reach existing tabs. See [Localization](../localization.md).

## Instance identity and desktop handshake

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_INSTANCE_ID` | UUID | generated | Persistent instance UUID reported at `/.well-known/stellarr/client` (auto-generated in `/data/stellarr/instance-id` when unset). |
| `STELLARR_INSTANCE_NAME` | string | `Stellarr` | Instance name reported before the primary server exists (afterward, uses the primary server's name; editable in Cockpit). |
| `STELLARR_TIME_ZONE` | IANA time zone | `UTC` | **Seed** for the instance time zone in Cockpit settings (for example, `Europe/Warsaw`). |
| `STELLARR_DESKTOP_MINIMUM_VERSION` | semver | `0.1.0` | Minimum allowed desktop client version; older clients are rejected by the client gate. |
| `STELLARR_DESKTOP_RECOMMENDED_VERSION` | semver | `0.1.3` | Recommended desktop client version advertised in `/.well-known/stellarr/client`. |

## Other settings

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `GIPHY_API_KEY` | string | empty | GIPHY key for GIF search. Sent to signed-in browsers (unless disabled in user privacy settings); can also be set and tested in **Cockpit → Integrations**, which takes precedence without a restart. |
| `STELLARR_EF_COMMAND_LOG_LEVEL` | .NET log level | `Warning` | Log level for `Microsoft.EntityFrameworkCore.Database.Command` (`Information` logs SQL queries). |

## Logs

The API writes logs to container stdout/stderr (`docker compose logs`) and to Cockpit's **Logs** view (which retains the latest 10,000 lines in memory). By default it logs:

- Warnings and errors from all components
- One summary line per API request (`POST /api/channels/…/messages 201 in 12 ms` under `Stellarr.Api.Requests`), excluding routine container health checks
- Operational lifecycle events (startup, media server resets, backups, maintenance)
- The [setup code](quick-start.md#4-create-the-owner-account) for the owner account, at each start while the instance has no account, as a warning so that it shows even with the log set to `Warning`

Framework HTTP diagnostics (`Microsoft.AspNetCore`), Entity Framework SQL (`Microsoft.EntityFrameworkCore.Database.Command`), and outgoing HTTP client calls default to `Warning` and above. Override any category in `docker-compose.override.yml`:

```yaml
services:
  stellarr:
    environment:
      Logging__LogLevel__Microsoft.AspNetCore: Information
      Logging__LogLevel__Stellarr.Api.Requests: Warning
```

## Container resource limits

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_CONTAINER_MEMORY_LIMIT` | size | `4g` | Container memory limit (`mem_limit`). |
| `STELLARR_CONTAINER_CPUS` | number | `4` | Container CPU quota (`cpus`). |
| `STELLARR_CONTAINER_PIDS_LIMIT` | integer | `512` | Container process limit (`pids_limit`). |

## Advanced container environment variables

The container entrypoint and TLS maintenance scripts also read the variables below. Set them in `docker-compose.override.yml` when needed.

| Variable | Default | Description |
| --- | --- | --- |
| `STELLARR_DATA_ROOT` | `/data/stellarr` | Root directory for persistent state inside the container. |
| `TLS_RENEW_BEFORE_SECONDS` | `2592000` (30 days) | Renew managed self-signed certificates when remaining validity falls below this threshold or when `STELLARR_DOMAIN` changes. |
| `TLS_RENEW_CHECK_SECONDS` | `43200` (12 hours) | Interval between self-signed certificate renewal checks. |
| `STELLARR_MEDIA_ADVERTISE` | empty | Comma-separated `ip:port` list overriding the derived `stellarr-media` ICE host candidates. |
| `STELLARR_MEDIA_LOG` | `info` | `tracing` filter for `stellarr-media` (for example, `info,stellarr_media=debug`). |
| `STELLARR_MEDIA_CONTROL_SECRET` | `/data/stellarr/secrets/media-control-secret` | Overrides the shared bearer secret between the API and `stellarr-media`. |
| `TURN_SHARED_SECRET` | `/data/stellarr/secrets/turn-shared-secret` | Overrides the shared HMAC secret between the API and `coturn`. |

> [!IMPORTANT]
> On first start, generated secrets are written to `/data/stellarr/secrets/`. Always include them when backing up `/data/stellarr`.

## Development stack (`docker-compose.dev.yml`)

`docker-compose.dev.yml` runs each service from bind mounts with live reload (`STELLARR_TIME_ZONE` defaults to `Europe/Warsaw` and `STELLARR_INSTANCE_NAME` to `Stellarr Dev`).

| Variable | Default | Description |
| --- | --- | --- |
| `STELLARR_DEV_WEB_PORT` | `3000` | Host port for Vite's dev server serving both the web client and Cockpit (`/cockpit`). |
| `STELLARR_DEV_API_PORT` | `5000` | Host port for the ASP.NET Core API (`dotnet watch`). |
| `STELLARR_DEV_MEDIA_PORT` | `49100` | Host UDP port for `stellarr-media` (advertised on `127.0.0.1`). |
| `STELLARR_DEV_MEDIA_CONTROL_PORT` | `5200` | Host loopback TCP port (`127.0.0.1:5200`) for the `stellarr-media` control API. |
| `STELLARR_DEV_ALLOWED_HOSTS` | empty | Comma-separated extra hostnames allowed by Vite's dev server (`localhost` and IPs are always allowed). |
| `STELLARR_API_ORIGIN` | `http://127.0.0.1:5000` | Upstream API origin proxied by Vite for `/api`, `/assets`, `/health`, `/hubs`, and `/.well-known/stellarr/client` (`http://api:5000` in Compose). |
| `STELLARR_DEV_POLLING` | `true` | Enables filesystem polling in Vite when file-watch events do not cross bind mounts. |

## Running without Docker Compose

`docker-compose.yml` maps friendly `STELLARR_*` variables from `.env` to the container's internal environment names. When running plain `docker run` or a custom platform template (such as [Unraid](unraid.md)):

- **Persistent volume**: Mount a volume or host directory at `/data/stellarr`.
- **Stop timeout**: Pass `--stop-timeout=75` so `stellarr-init` has time to shut down services gracefully (Docker's 10-second default cuts shutdown short).
- **Language**: Set **both** `STELLARR_LANGUAGE` (web client) and `Stellarr__Language` (API) to `en` or `pl`.
- **Port mappings and advertised variables**: A container cannot inspect host port mappings, so set these container variables to match your published host ports:

| Purpose | Container port | Default host port | Container variable | Compose `.env` variable |
| --- | --- | --- | --- | --- |
| HTTPS | `443/tcp` | `8443` | `PUBLIC_HTTPS_PORT` | `STELLARR_HTTPS_PORT` |
| Voice media | `49000/udp` | `49000` | `Rtc__MediaPort` | `STELLARR_MEDIA_PORT` |
| TURN | `3478/udp`, `3478/tcp` | `3478` | `Rtc__TurnPort` | `STELLARR_TURN_PORT` |
| TURNS | `5349/tcp`, `5349/udp` | `5349` | `Rtc__TurnsPort` | `STELLARR_TURNS_PORT` |

The HTTP redirect (`80/tcp`, default host `8888`) needs no container variable, and the TURN relay range (`49160-49200/udp`) must map 1:1 with the same port numbers on host and container. For all other optional variables, inspect [`docker-compose.yml`](../../docker-compose.yml) to see the underlying container key name.
