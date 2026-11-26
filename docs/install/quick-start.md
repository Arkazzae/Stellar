# Quick start

Deploy a local Stellarr instance, create the owner account and initial server, invite a second user, and verify two-way voice audio.

## Prerequisites

- **Docker Engine** with the Compose plugin `2.24.0` or newer (`docker compose version`). See [Requirements](requirements.md).
- **Available ports:** `443/tcp` (HTTPS), `49000/udp` and `49000/tcp` (WebRTC media), and `127.0.0.1:8888/tcp` (local HTTP upstream). Occupied ports can be remapped in `.env`; see [Requirements: Ports](requirements.md#ports).
- **Second browser session:** an incognito window or separate browser profile to test a two-user call.

## Automated installer (`install.sh`)

The installer checks host prerequisites, writes `docker-compose.yml` and `.env`, starts the container, waits for health checks, and prints your instance URL and one-time setup code:

```bash
curl -fsSL https://stellarr.online/install.sh | bash
```

For unattended deployments, pass `--yes` (`-y`) alongside your target domain and public IP policy:

```bash
curl -fsSL https://stellarr.online/install.sh | bash -s -- \
  --yes --domain chat.example.com --public-ip auto
```

| Mode / Option | Behavior |
| --- | --- |
| **Interactive** (default) | Prompts for `--domain` (`localhost` default), `--public-ip` (IPv4 or `auto` when a domain is set), and `--language` (`en` or `pl`). Offers to install Docker on Linux (`--install-docker`) and picks free fallback ports on conflict. |
| **Non-interactive** (`--yes`) | Skips prompts and applies flags (`--domain`, `--public-ip`, `--language`, `--https-port`, `--public-port`, `--http-port`, `--media-port`, `--enable-turn`, `--disable-turn`, `--turn-port`, `--turns-port`, `--version`, `--ref`, `--no-start`, `--skip-port-check`, `--force`) or matching `STELLARR_*` env vars (`STELLARR_START_TIMEOUT=300` s default). |
| **Target directory** (`--dir`) | Defaults in order to: (1) git checkout containing `./install.sh`, (2) working directory of an existing Stellarr Compose container, (3) `$PWD` if `docker-compose.yml` exists there, or (4) `~/stellarr` (`STELLARR_INSTALL_DIR`). |
| **Updating** | Re-running `install.sh` updates `docker-compose.yml` and `STELLARR_VERSION` while keeping all other `.env` values intact. Put custom Compose overrides in `docker-compose.override.yml`. |
| **Git checkout build** | Run `./install.sh --build` (or `cargo run --locked --manifest-path init/installer/Cargo.toml -- --dir . --build` before binary releases) to build `stellarr-local:dev` locally. |
| **Binary pinning** | `install.sh` verifies the platform binary (`linux`/`darwin`, `amd64`/`arm64`) against SHA-256 manifests from `stellarr.online/server-downloads/`. Override with `STELLARR_INSTALLER_VERSION` or `STELLARR_INSTALLER_BIN`. |

If you used `install.sh`, skip directly to [Step 3: Open the web client](#3-open-the-web-client). The steps below cover manual setup with Docker Compose.

---

## Manual setup with Docker Compose

## 1. Download Compose and environment files

```bash
mkdir stellarr && cd stellarr
curl -fsSLO https://raw.githubusercontent.com/Arkazzae/Stellarr/main/docker-compose.yml
curl -fsSL https://raw.githubusercontent.com/Arkazzae/Stellarr/main/.env.example -o .env
```

Every line in `.env.example` is commented out so built-in defaults apply out of the box on `localhost`.

## 2. Start the container

```bash
docker compose up -d
docker compose ps
```

Docker pulls `ghcr.io/arkazzae/stellarr`, creates the `stellarr_stellarr_data` volume, and marks the container `healthy` once `stellarr-media`, `stellarr-api` (with SQLite), and `nginx` pass readiness probes (`coturn` is checked as well when TURN is enabled).

If the container stays `starting` or `unhealthy`, inspect the supervisor and logs:

```bash
docker compose exec stellarr stellarr-init status
docker compose logs
```

## 3. Open the web client

Open the HTTPS URL printed by the installer (default `https://localhost`).

> [!NOTE]
> On first boot, Stellarr generates a 365-day self-signed TLS certificate for `localhost`. Accept the browser's certificate prompt for local testing. Before exposing the server remotely, configure a trusted certificate via [Reverse proxy and TLS](reverse-proxy-and-tls.md).

## 4. Create the owner account and first server

Until an owner account exists, every client route redirects to `/setup`. Claiming the instance requires a one-time setup code written to container stdout:

```bash
docker compose logs stellarr | grep "Setup code"
```

```text
Setup code: K7QM-2XWP-9HDA. Open /setup at the address of this instance to create the owner account.
```

*(A new setup code is generated on each restart until setup finishes; case, spaces, and hyphens are ignored.)*

Complete the three-step `/setup` wizard:

1. **Owner account:** Enter the setup code, display name, username (`3–32` characters: letters, digits, `.`, `-`, `_`), email address, and password (`8+` characters).
2. **First server:** Set the server name (`1–80` characters), optional description, first text channel (default `general`), and voice channel toggle (default `lounge`). Click **Create account and server**.
3. **Invite link:** The wizard commits the owner account and primary server in a single SQLite transaction, invalidates the setup code, and displays an invite URL. Copy the link and click **Open server**.

## 5. Invite a second user

To create additional invite links inside the app:

1. Click **Invite to server** next to the server name in the channel rail header.
2. Click **New invite link** to generate an invite URL (for example, `https://localhost/invite/22b115a390a20370`).
3. If you mapped HTTPS to a custom host port on `localhost`, include that port in the URL.

> [!WARNING]
> When both `STELLARR_DOMAIN` and `STELLARR_PUBLIC_URL` are unset, generated invite URLs fall back to the request `Host` header without custom port numbers. Set `STELLARR_DOMAIN` (with `STELLARR_PUBLIC_PORT`) or `STELLARR_PUBLIC_URL` so links always carry the exact public origin.

## 6. Join and test voice

1. Open the invite URL in a private window or second browser profile, accept the local certificate prompt, and register a second user.
2. Join the `lounge` voice channel in both browser windows and grant microphone access.
3. Verify two-way audio and confirm the call dock status in the bottom-left corner reads **Connected**.

> [!IMPORTANT]
> Default `localhost` settings advertise media on `127.0.0.1` for browsers on the same machine. Before calling from LAN devices or across the internet, set `STELLARR_PUBLIC_IP` and forward `49000/udp` and `49000/tcp` on your router; see [Requirements: Behind NAT](requirements.md#behind-nat) and [RTC diagnostics](../rtc-and-ports.md#diagnostics).

---

## Stop, remove, or build from source

### Stopping the stack

```bash
docker compose stop      # stop services in reverse dependency order (75 s grace period)
docker compose down      # remove container and network; keep stellarr_stellarr_data volume
```

See [Running without Docker Compose](configuration.md#running-without-docker-compose) if you manage containers with `docker run`.

> [!CAUTION]
> Running `docker compose down -v` permanently deletes the `stellarr_stellarr_data` volume, destroying all accounts, messages, files, encryption keys, and secrets.

### Building from source

To compile the entire production image locally (WASM recipes, web client, Cockpit, Rust API, CLI, `stellarr-media`, and `stellarr-init`) using Docker BuildKit alone:

```bash
git clone https://github.com/Arkazzae/Stellarr.git
cd Stellarr
docker compose -f docker-compose.yml -f infra/compose/build.yml up -d --build
```

## Next steps

- Open `https://localhost/cockpit` with your owner account to explore [Stellarr Cockpit](../admin/cockpit.md).
- Configure LAN/WAN media addresses and firewall rules in [Requirements](requirements.md).
- Add a browser-trusted certificate in [Reverse proxy and TLS](reverse-proxy-and-tls.md).
- Browse all environment variables in [Configuration](configuration.md).
