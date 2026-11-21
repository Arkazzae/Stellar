# Configuration

Every setting is an environment variable named `STELLARR_*`, and it has that one name everywhere: in `.env`, in `docker run -e`, in a platform template such as [Unraid](unraid.md), and on this page. The image holds every default, so an installation sets only what it changes.

With Docker Compose, the settings go in `.env` beside `docker-compose.yml`. Compose hands that file to the container as it is; [`.env.example`](../../.env.example) lists every setting, commented out, with its default, and the installer appends it to the `.env` it writes. A variable that is empty or only whitespace counts as not set.

The container reads its settings from the file `.env` alone. A variable exported in the shell, or a file given with `docker compose --env-file`, does not reach it. They change only what `docker-compose.yml` itself fills in: the image tag, the published ports (which Compose also tells the container) and the resource limits.

A change takes effect when the container is created again:

```bash
docker compose up -d
```

> [!TIP]
> Variables marked **seed** give the first value of a database setting that the owner then changes in Cockpit (`/cockpit`) or with the [operator CLI](../admin/cli.md). Once saved there, the database value wins over `.env`.

## Release

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_VERSION` | image tag | `latest` | Tag of `ghcr.io/arkazzae/stellarr` that Compose runs. Pin a release such as `1.2` or `1.2.3` for a real installation. |

## Address and TLS

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_DOMAIN` | hostname or IP | empty | The name or address people open the instance at. It names the self-signed certificate and the TURN realm and builds the public address; without it they use `localhost`. |
| `STELLARR_PUBLIC_IP` | IPv4 or `auto` | empty | The IPv4 address `coturn` and `stellarr-media` advertise. Needed for voice from any device but the Docker host; `auto` resolves the `A` record of `STELLARR_DOMAIN` at start. |
| `STELLARR_HTTPS_PORT` | port | `8443` with Compose, `443` without | The host port of container port `443/tcp`. Compose publishes it and tells the container, which uses it for the HTTP redirect and the public address. |
| `STELLARR_HTTP_PORT` | port or `ip:port` | `8888` | The host port of `80/tcp`, which only redirects to HTTPS. Prefix it with `127.0.0.1:` for a reverse proxy on the same host. Compose alone reads it. |
| `STELLARR_PUBLIC_PORT` | port | `STELLARR_HTTPS_PORT` | The HTTPS port browsers use: `443` behind a [reverse proxy](reverse-proxy-and-tls.md#run-behind-a-reverse-proxy). Left out of addresses when it is `443`. |
| `STELLARR_PUBLIC_URL` | origin URL | empty | The public address as a whole (`http` or `https`, no credentials, query or fragment), in place of the one built from `STELLARR_DOMAIN` and `STELLARR_PUBLIC_PORT`. |
| `STELLARR_TRUSTED_PROXIES` | IP / CIDR list | empty | IPv4 and IPv6 addresses or networks of the [reverse proxies](reverse-proxy-and-tls.md#client-addresses) in front of the container, separated by commas or spaces. |

- **Certificates**: When `/data/stellarr/tls/fullchain.pem` or `privkey.pem` is missing, the container generates a self-signed certificate and renews it when `STELLARR_DOMAIN` changes or fewer than 30 days remain. A CA-signed certificate is never overwritten; see [Reverse proxy and TLS](reverse-proxy-and-tls.md).
- **Public address**: The public address is used for password-reset and email-change links, invite links, `/.well-known/stellarr/client` and passkey validation. Without `STELLARR_DOMAIN` and `STELLARR_PUBLIC_URL` the instance has none, and password-reset mail stays off.
- **Trusted proxies**: A listed proxy's `X-Forwarded-For` gives the client's own address to the rate limits, sessions, security events and trusted devices. An entry the API cannot use (a hostname, a network with host bits set such as `10.0.0.1/24`, or `0.0.0.0/0`) stops the API at start.

## Voice and RTC

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_MEDIA_PORT` | port | `49000` | The host UDP port of [Stellarr Media](../stellarr-media.md), which carries every call. The router forwards the same external port. |
| `STELLARR_TURN_PORT` | port | `3478` | The host port of STUN and TURN, UDP and TCP, which browsers are told. |
| `STELLARR_TURNS_PORT` | port | `5349` | The host port of TURN over TLS and DTLS. |
| `STELLARR_TURN_RELAY_RANGE` | port range | `49160-49200` | The host UDP ports of TURN's relays. Compose maps them 1:1 to the same container range, so the range keeps its length. |
| `STELLARR_TURN_TCP_ENABLED` | `true` \| `false` | `false` | TURN over TCP: the `coturn` listener and the `?transport=tcp` addresses. |
| `STELLARR_TURNS_ENABLED` | `true` \| `false` | `false` | TURN over TLS and DTLS (`turns:`). Browsers accept it only with a certificate they trust. |

A transport that is off is closed in `coturn`, not only left out of `/api/bootstrap`. See [RTC and ports](../rtc-and-ports.md) for relay sizing and firewall rules.

## Uploads and quotas

The byte limits take `0`, or a negative value, to turn the check off.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_UPLOAD_USER_BYTES` | bytes | `2147483648` (2 GiB) | Storage each account may use. |
| `STELLARR_UPLOAD_TOTAL_BYTES` | bytes | `21474836480` (20 GiB) | Storage the instance may use. |
| `STELLARR_MIN_FREE_BYTES` | bytes | `1073741824` (1 GiB) | Free space an upload must leave on `/data/stellarr`. |
| `STELLARR_MAX_CONCURRENT_UPLOADS` | integer (`1–32`) | `4` | Uploads the instance takes at once. |

Each file also has a limit by type: images `10 MiB`, audio `25 MiB`, documents and other files `25 MiB`, video `100 MiB`. Nginx refuses a request body over `128 MiB`.

## Mail

Password-reset, invitation, security-alert and activity mail goes through SMTP. Its host, port, encryption (`STARTTLS`, `TLS` or `none`) and username are set in Cockpit under **Settings**; Resend replaces SMTP when it is turned on under **Integrations**.

> [!NOTE]
> A secret saved in Cockpit (the SMTP password, the Resend API key) is encrypted with the instance's data-protection keys, wins over `.env` and never goes back to a browser. Clearing it in Cockpit brings the variable's value back.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_MAIL_ENABLED` | `true` \| `false` | `true` | **Seed** of Cockpit's mail switch. |
| `STELLARR_MAIL_FROM` | email | empty | **Seed** of the sender address. `onboarding@resend.dev` works only through Resend. |
| `STELLARR_MAIL_FROM_NAME` | string | `Stellarr` | The sender's display name. |
| `STELLARR_SMTP_PASSWORD` | string | empty | The SMTP password, when none is saved in Cockpit. |
| `STELLARR_RESEND_API_KEY` | string | empty | The Resend API key, when none is saved in Cockpit. |

## Web Push

Browser notifications for mentions and direct messages, off until a persistent VAPID key pair is set. [Activity notifications](../admin/notifications.md#web-push) describes how to make one; a new pair makes every browser subscribe again.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_PUSH_ENABLED` | `true` \| `false` | `false` | Turns Web Push on, once the key pair and the subject are valid. |
| `STELLARR_PUSH_SUBJECT` | `mailto:` or `https://` URI | empty | The operator's contact for push services, such as `mailto:admin@chat.example.com`. |
| `STELLARR_PUSH_PUBLIC_KEY` | base64url | empty | The uncompressed P-256 VAPID public key (65 bytes decoded). |
| `STELLARR_PUSH_PRIVATE_KEY` | base64url | empty | The P-256 VAPID private key (32 bytes decoded). A secret. |

## Single sign-on (OIDC)

Sign-in through an OpenID Connect provider, beside local passwords. The callback address is `<public-url>/api/auth/oidc/callback`.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_OIDC_ENABLED` | `true` \| `false` | `false` | Turns the provider on; it needs the issuer, the client ID and a client secret. |
| `STELLARR_OIDC_ISSUER` | HTTPS URL | empty | The provider's issuer. |
| `STELLARR_OIDC_CLIENT_ID` | string | empty | The client ID. |
| `STELLARR_OIDC_CLIENT_SECRET` | string | empty | The client secret; wins over `STELLARR_OIDC_CLIENT_SECRET_FILE`. |
| `STELLARR_OIDC_CLIENT_SECRET_FILE` | container path | empty | A mounted file that holds the client secret. |
| `STELLARR_OIDC_DISPLAY_NAME` | string | `Main account`, or `Konto główne` on a Polish instance | The label of the sign-in button. |
| `STELLARR_OIDC_TRUST_PROVIDER_MFA` | `true` \| `false` | `false` | Counts a sign-in the provider confirmed with a matching `amr` as Stellarr's second step. |
| `STELLARR_OIDC_PROVIDER_MFA_AMR` | comma-separated list | `mfa` | The `amr` values that count when `STELLARR_OIDC_TRUST_PROVIDER_MFA` is `true`. |

### Additional OIDC providers

Further providers have no `STELLARR_*` names; each takes the raw keys below, in `.env` like any other setting. A provider is keyed by an ID of `1–32` lowercase letters, digits or hyphens, other than `default`, `login`, `callback`, `link`, `reauthenticate` and `pending`, and gets its own sign-in button and the callback address `<public-url>/api/auth/oidc/<id>/callback`:

```bash
Oidc__Providers__work__DisplayName=Work account
Oidc__Providers__work__Authority=https://id.example.com/realms/work
Oidc__Providers__work__ClientId=stellarr
Oidc__Providers__work__ClientSecretFile=/run/secrets/oidc-work
# Optional, off by default:
Oidc__Providers__work__TrustProviderMfa=true
Oidc__Providers__work__ProviderMfaAmr=mfa,hwk
```

- A provider is on only when its issuer is HTTPS and its client ID and secret are set; an ID that breaks the rule above stops the API at start.
- People link their accounts to a provider under **Settings → Login and security → Sign-in providers** (see [Account security](../admin/account-security.md#linking-an-identity-provider)). An identity matches by `(issuer, subject)` alone, never by email address.
- Someone the provider confirms whose identity is linked to no account can link it by signing in with their password on the page that follows, or, with an invite, create an account through the provider when the owner allows it under **Cockpit → Integrations → Sign-in providers** (off by default). See [Signing in with an identity no account is linked to](../admin/account-security.md#signing-in-with-an-identity-no-account-is-linked-to).
- Stellarr asks for the `openid profile email` scopes and reads `name`, `preferred_username`, `email` and `email_verified` from the ID token or the userinfo endpoint to fill in a new account; only an address the provider marks as verified counts as verified.
- An account without a password confirms a sensitive change by signing in at its provider again, so the provider must return `auth_time` and should honour `prompt=login` and `max_age`: the 5 minutes for the change count from `auth_time`.

### Trusting provider MFA

By default an OIDC sign-in still asks for Stellarr's own second step when the account or the instance's policy has one. With `TrustProviderMfa`, the provider's own check counts as that step, in Cockpit too:

- The ID token must hold one of the listed `amr` values and an `auth_time` within the last 5 minutes (Stellarr sends `max_age=300` when it asks again).
- Without a recent `auth_time`, Stellarr asks for its own second step. Turn this on only for a provider that requires a real second factor and reports it in `amr`.

## Automatic backups and Cloudflare R2

A production instance makes a full backup every day by default. **Cockpit → Backups → Backup settings** changes the schedule while it runs; see [Automatic backups](../admin/backups.md) and [Cloudflare R2 offsite backups](../admin/cloudflare-r2.md).

### Backup schedule

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_BACKUP_ENABLED` | `true` \| `false` | `true` | **Seed** of automatic full backups. |
| `STELLARR_BACKUP_INTERVAL_HOURS` | integer | `24` | Hours between finished backup runs. |
| `STELLARR_BACKUP_RETRY_MINUTES` | integer | `15` | Minutes before a failed offsite upload is tried again. |
| `STELLARR_BACKUP_INITIAL_DELAY_MINUTES` | integer | `5` | Minutes after the start before the first scheduled backup. |

### Cloudflare R2 integration

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_CLOUDFLARE_R2_ENABLED` | `true` \| `false` | `false` | Uploads finished backups to Cloudflare R2. |
| `STELLARR_CLOUDFLARE_R2_MODE` | `s3` \| `worker` | `s3` | Directly through the S3 API (`s3`), or through the Backup Relay Worker (`worker`). |
| `STELLARR_CLOUDFLARE_R2_ENDPOINT` | HTTPS URL | empty | The bucket's S3 endpoint (`https://<ACCOUNT_ID>.r2.cloudflarestorage.com`), or the Worker's address. |
| `STELLARR_CLOUDFLARE_R2_REGION` | string | `auto` | The region, in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_BUCKET` | string | empty | The private bucket, in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_ACCESS_KEY` | string | empty | The S3 access key ID, in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_SECRET_KEY` | string | empty | The S3 secret access key, in `s3` mode. |
| `STELLARR_CLOUDFLARE_R2_WORKER_TOKEN` | string | empty | The Worker's bearer token, in `worker` mode. |
| `STELLARR_CLOUDFLARE_R2_PREFIX` | string | `stellarr` | The prefix of the objects in the bucket. |

## Language

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_LANGUAGE` | `en` \| `pl` | `en` | The web client's language for anyone who has not chosen one, and the language of error and system messages, a new server's default names, audit entries and mail. |

Another value stops the API at start. Records saved earlier keep the language they were written in. Browsers keep `/stellarr-instance.js` for up to an hour, so a changed default may take that long to reach open tabs. See [Localization](../localization.md).

## Instance identity and desktop handshake

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_INSTANCE_ID` | UUID | generated | The instance's UUID in `/.well-known/stellarr/client`. Without it, one is generated and kept in `/data/stellarr/instance-id`. |
| `STELLARR_INSTANCE_NAME` | string | `Stellarr` | The name reported until the first server exists; after that, the server's name, which Cockpit edits. |
| `STELLARR_TIME_ZONE` | IANA time zone | the container's, `UTC` | **Seed** of the instance's time zone in Cockpit, such as `Europe/Warsaw`. |
| `STELLARR_DESKTOP_MINIMUM_VERSION` | semver | `0.1.0` | The oldest desktop client the client gate lets in. |
| `STELLARR_DESKTOP_RECOMMENDED_VERSION` | semver | `0.1.3` | The desktop client `/.well-known/stellarr/client` recommends. |

## Other settings

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_GIPHY_API_KEY` | string | empty | The public GIPHY web key for GIF search, sent to signed-in browsers (unless a person turns GIPHY off in their privacy settings). **Cockpit → Integrations** can set and check it too, and its value wins without a restart. |
| `STELLARR_EF_COMMAND_LOG_LEVEL` | .NET log level | `Warning` | The log level of `Microsoft.EntityFrameworkCore.Database.Command`; `Information` logs every SQL query. |

## Logs

The API writes its log to the container's output (`docker compose logs`) and to Cockpit's **Logs** page, which keeps the latest 10,000 lines in memory. By default it logs:

- warnings and errors from every component
- one line per API request (`POST /api/channels/…/messages 201 in 12 ms`, under `Stellarr.Api.Requests`), without the container's routine health checks
- operational events: start, media server resets, backups and maintenance
- the [setup code](quick-start.md#4-create-the-owner-account) of the owner account, at each start while the instance has no account, as a warning so that it shows even at the `Warning` level

ASP.NET Core's request diagnostics (`Microsoft.AspNetCore`), Entity Framework's SQL and outgoing HTTP calls log from `Warning` up. Any category takes a raw key in `.env`:

```bash
Logging__LogLevel__Microsoft.AspNetCore=Information
Logging__LogLevel__Stellarr.Api.Requests=Warning
```

## Container resource limits

Compose reads these; with `docker run`, the matching flags are `--memory`, `--cpus` and `--pids-limit`.

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_CONTAINER_MEMORY_LIMIT` | size | `4g` | The container's memory limit (`mem_limit`). |
| `STELLARR_CONTAINER_CPUS` | number | `4` | The container's CPU quota (`cpus`). |
| `STELLARR_CONTAINER_PIDS_LIMIT` | integer | `512` | The container's process limit (`pids_limit`). |

## Advanced

| Variable | Type | Default | Description |
| --- | --- | --- | --- |
| `STELLARR_TLS_RENEW_BEFORE_SECONDS` | seconds | `2592000` (30 days) | Renews the managed self-signed certificate when less validity remains, or when `STELLARR_DOMAIN` changes. |
| `STELLARR_TLS_RENEW_CHECK_SECONDS` | seconds | `43200` (12 hours) | How often the managed certificate is checked. |
| `STELLARR_MEDIA_ADVERTISE` | `ip:port` list | empty | `stellarr-media`'s host candidates, separated by commas, in place of the ones derived from `STELLARR_PUBLIC_IP` and the container's address. |
| `STELLARR_MEDIA_LOG` | `tracing` filter | `info` | `stellarr-media`'s log filter, such as `info,stellarr_media=debug`. |
| `STELLARR_MEDIA_CONTROL_SECRET` | string | generated | The bearer secret the API and `stellarr-media` share. Without it, one is generated in `/data/stellarr/secrets/media-control-secret`. |
| `STELLARR_TURN_SHARED_SECRET` | string | generated | The HMAC secret the API and `coturn` share. Without it, one is generated in `/data/stellarr/secrets/turn-shared-secret`. |

> [!IMPORTANT]
> The generated secrets are written to `/data/stellarr/secrets/` on the first start. Keep them in every backup of `/data/stellarr`.

The image keeps its data under `/data/stellarr`, the path the volume is mounted at. It is not a setting: the entrypoint, the TLS script and the CLI all read it from the image.

### Settings without a `STELLARR_*` name

A few settings exist only as raw .NET configuration keys, written `Section__Key`: the [additional OIDC providers](#additional-oidc-providers) (`Oidc__Providers__<id>__*`), [log levels](#logs) (`Logging__LogLevel__*`), the rate limits (`RateLimiting__*`) and link previews (`Stellarr__Embeds__*`). They go in `.env` like the others. A setting that has a `STELLARR_*` name takes only that name: its variable wins over the raw key.

## Development stack (`docker-compose.dev.yml`)

`docker-compose.dev.yml` runs each service from a bind mount with live reload. It reads no `env_file`. Its own variables below come from the shell or `.env`; of the instance's settings it forwards only `STELLARR_LANGUAGE`, `STELLARR_TIME_ZONE`, `STELLARR_INSTANCE_ID`, the two desktop versions, `STELLARR_GIPHY_API_KEY`, the mail sender and secrets and the Web Push settings, and sets the rest itself (instance name `Stellarr Dev`, mail on for Mailpit).

| Variable | Default | Description |
| --- | --- | --- |
| `STELLARR_DEV_WEB_PORT` | `3000` | Host port of Vite's dev server, which serves the web client and Cockpit (`/cockpit`). |
| `STELLARR_DEV_API_PORT` | `5000` | Host port of the API under `dotnet watch`. |
| `STELLARR_DEV_MEDIA_PORT` | `49100` | Host UDP port of `stellarr-media`, advertised on `127.0.0.1`. |
| `STELLARR_DEV_MEDIA_CONTROL_PORT` | `5200` | Host port of the `stellarr-media` control API, on `127.0.0.1`. |
| `STELLARR_DEV_MAILPIT_PORT` | `8025` | Host port of Mailpit's inbox, on `127.0.0.1`. |
| `STELLARR_DEV_MAILPIT_SMTP_PORT` | `1025` | Host port of Mailpit's SMTP server, on `127.0.0.1`. |
| `STELLARR_DEV_ALLOWED_HOSTS` | empty | Further host names Vite's dev server answers, comma-separated; `localhost` and IP addresses always work. |
| `STELLARR_DEV_POLLING` | `true` | Whether Vite polls for changes, where file events do not cross a bind mount. |

`npm run dev` in `web/` outside Docker reads the variables in [`web/.env.example`](../../web/.env.example), among them `STELLARR_API_ORIGIN`, the API it proxies to.

## Running without Docker Compose

`docker run` and platform templates such as [Unraid](unraid.md) use the same names as `.env`, and the image's defaults apply to everything they leave out. Beside the settings:

- **Volume**: mount a volume or a host folder at `/data/stellarr`.
- **Stop timeout**: pass `--stop-timeout=75`, so that `stellarr-init` has time to stop the services; Docker's 10 seconds cut the shutdown short.
- **Host ports**: a container cannot see its port mappings. When a host port differs from the container's, set its variable to the host port: `STELLARR_HTTPS_PORT` for `443/tcp` (the image assumes `443`), `STELLARR_MEDIA_PORT` for `49000/udp`, `STELLARR_TURN_PORT` for `3478` and `STELLARR_TURNS_PORT` for `5349`. The HTTP redirect on `80/tcp` needs no variable, and the TURN relay range `49160-49200/udp` maps 1:1.

```bash
docker run -d --name stellarr --restart unless-stopped --stop-timeout=75 \
  --memory=4g --cpus=4 --pids-limit=512 --security-opt=no-new-privileges:true \
  -e STELLARR_DOMAIN=chat.example.com -e STELLARR_PUBLIC_IP=auto \
  -e STELLARR_HTTPS_PORT=8443 \
  -p 8888:80 -p 8443:443 -p 3478:3478/tcp -p 3478:3478/udp \
  -p 5349:5349/tcp -p 5349:5349/udp -p 49000:49000/udp -p 49160-49200:49160-49200/udp \
  -v stellarr_data:/data/stellarr \
  ghcr.io/arkazzae/stellarr:latest
```
