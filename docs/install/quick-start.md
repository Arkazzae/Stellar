# Quick start

Deploy a local Stellarr instance, create the owner account and initial server, invite a second user, and verify two-way voice audio.

## Prerequisites

- **Git** to check out the release.
- **Docker Engine** with the Compose plugin `2.24.0` or newer (`docker compose version`). See [Requirements](requirements.md).
- **Available ports:** `443/tcp` (HTTPS), `49000/udp` and `49000/tcp` (WebRTC media), and `127.0.0.1:8888/tcp` (local HTTP upstream). Occupied ports can be remapped in `.env`; see [Requirements: Ports](requirements.md#ports).
- **Second browser session:** an incognito window or separate browser profile to test a two-user call.

## 1. Select a release

From a directory for new installations, check out a release and pin its image in `.env`.

```bash
git clone --depth 1 --branch server-v0.3.2 https://github.com/Arkazzae/Stellarr.git &&
  cd Stellarr &&
  cp .env.example .env &&
  echo 'STELLARR_VERSION=0.3.2' >> .env
```

Review `docker-compose.yml` for the image, ports, and volume. For access from another device, set the [LAN or internet address](requirements.md#behind-nat) in `.env` before starting.

## 2. Start the container

```bash
docker compose up -d --wait
docker compose ps
```

Docker pulls `ghcr.io/arkazzae/stellarr`, creates the `stellarr_stellarr_data` volume, and marks the container `healthy` once `stellarr-media`, `stellarr-api` (with SQLite), and `nginx` pass readiness probes (`coturn` is checked as well when TURN is enabled).

If the container stays `starting` or `unhealthy`, inspect the supervisor and logs:

```bash
docker compose exec stellarr stellarr-init status
docker compose logs
```

## 3. Open the web client

Open your configured HTTPS address (default `https://localhost`).

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
3. **Invite:** The wizard commits the owner account and primary server in a single SQLite transaction, invalidates the setup code, and displays an invitation. Copy it and click **Open server**. On a loopback address such as `localhost`, `127.0.0.1` or `[::1]`, the wizard shows only the invite code, because a link to that address would open on the other person's own device. Releases up to 0.2.1 show the full link. Either one can be pasted under **I have an invite link** on the instance's sign-in page.

## 5. Invite a second user

To create additional invite links inside the app:

1. Click **Invite to server** next to the server name in the channel rail header.
2. Click **New invite link**. On a loopback address the dialog shows the invite code. To test on this computer, open the instance in a private window or another browser profile and paste the code under **I have an invite link**.
3. For other devices, set the [public hostname and media address](requirements.md#behind-nat), apply them with `docker compose up -d`, and reopen the invite dialog. Links then use the configured public origin, for existing invites as well as new ones.

Without `STELLARR_DOMAIN` or `STELLARR_PUBLIC_URL`, a link carries the address and port your browser used to open the instance.

## 6. Join and test voice

1. In a private window or second browser profile, open the invite link, or the sign-in page with the invite code, accept the local certificate prompt, and register a second user.
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

## Optional guided installer

To run the wizard from a source checkout, use Cargo. The wizard checks ports, prepares Compose files and shows the setup code. Choose an installation directory with `--dir`.

```bash
cargo run --locked --manifest-path init/installer/Cargo.toml -- --dir "$PWD"
```

For a published installer, use the current [bootstrap script and verification instructions](https://github.com/Arkazzae/Stellarr/tree/main/init/installer). For unattended setup, pass `--yes --domain chat.example.com --public-ip auto`.

| Mode / Option | Behavior |
| --- | --- |
| **Interactive** (default) | Prompts for `--domain` (`localhost` default), `--public-ip` (IPv4 or `auto` when a domain is set), and `--language` (`en`, `pl` or `de`). Offers to install Docker on Linux (`--install-docker`) and picks free fallback ports on conflict. |
| **Non-interactive** (`--yes`) | Skips prompts and applies flags (`--domain`, `--public-ip`, `--language`, `--https-port`, `--public-port`, `--http-port`, `--media-port`, `--enable-turn`, `--disable-turn`, `--turn-port`, `--turns-port`, `--version`, `--ref`, `--no-start`, `--skip-port-check`, `--force`) or matching `STELLARR_*` env vars (`STELLARR_START_TIMEOUT=300` s default). |
| **Target directory** (`--dir`) | Defaults in order to: (1) git checkout containing `./install.sh`, (2) working directory of an existing Stellarr Compose container, (3) `$PWD` if `docker-compose.yml` exists there, or (4) `~/stellarr` (`STELLARR_INSTALL_DIR`). |
| **Updating** | Re-running `install.sh` updates `docker-compose.yml` and `STELLARR_VERSION` while keeping all other `.env` values intact. Put custom Compose overrides in `docker-compose.override.yml`. |
| **Git checkout build** | Run `cargo run --locked --manifest-path init/installer/Cargo.toml -- --dir . --build` to build `stellarr-local:dev` locally. |
| **Binary pinning** | `install.sh` verifies the platform binary (`linux`/`darwin`, `amd64`/`arm64`) against the SHA-256 manifest from `stellarr.online/server-downloads/` and the GitHub attestation for the release workflow and tag. This path requires an authenticated GitHub CLI with `gh attestation verify --source-ref` support. Override with `STELLARR_INSTALLER_VERSION` or `STELLARR_INSTALLER_BIN`. |

## Next steps

- Open `https://localhost/cockpit` with your owner account to explore [Stellarr Cockpit](../admin/cockpit.md).
- Configure LAN/WAN media addresses and firewall rules in [Requirements](requirements.md).
- Add a browser-trusted certificate in [Reverse proxy and TLS](reverse-proxy-and-tls.md).
- Browse all environment variables in [Configuration](configuration.md).
