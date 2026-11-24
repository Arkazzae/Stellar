# Quick start

Start a local Stellarr instance on a single machine, create the owner account and initial server, invite a second user, and verify a two-user voice call.

## Prerequisites

- Docker Engine with the Compose plugin `2.24.0` or newer (`docker compose version`). See [Requirements](requirements.md).
- Free host ports `8443`, `8888`, `3478`, `5349`, `49000`, and `49160-49200`. If a port is in use, override the corresponding variable in `.env` (see [Requirements](requirements.md#ports)).
- A second browser profile or private window to test a two-user call.

## One-line installer

Run the installer script to verify host prerequisites, write `docker-compose.yml` and `.env`, start the container, and print the instance URL and setup code:

```bash
curl -fsSL https://stellarr.online/install.sh | bash
```

- **Interactive mode**: Prompts for up to three settings: the domain or IPv4 address users will open (`--domain` / `STELLARR_DOMAIN`, default `localhost`), the external IPv4 address for voice calls when a domain is entered (`--public-ip` / `STELLARR_PUBLIC_IP`, an IPv4 address or `auto`), and the interface language (`--language` / `STELLARR_LANGUAGE`, `en` or `pl`, defaulting to the shell locale). On Linux, if Docker is missing, the installer offers to install Docker Engine via `https://get.docker.com` (or pass `--install-docker`). If a required port is occupied, it offers the next free port.
- **Non-interactive mode**: Pass `-y` / `--yes` (or run without a TTY) to skip prompts and apply CLI flags (`--domain`, `--public-ip`, `--language`, `--https-port`, `--public-port`, `--http-port`, `--media-port`, `--turn-port`, `--turns-port`, `--version`, `--ref`, `--no-start`, `--skip-port-check`, `--force`) or their `STELLARR_*` environment variables (`STELLARR_START_TIMEOUT` defaults to `300` s):

  ```bash
  curl -fsSL https://stellarr.online/install.sh | bash -s -- --yes --domain chat.example.com --public-ip auto
  ```

- **Install directory (`--dir PATH` or `STELLARR_INSTALL_DIR`)**: When omitted, resolves in order to: (1) the git checkout containing `./install.sh`, (2) the working directory of an existing Stellarr Compose container, (3) the current directory if it already holds Stellarr's `docker-compose.yml`, or (4) `~/stellarr`.
- **Updating**: Re-running the installer updates an existing installation to the latest pinned release tag (or `--version TAG`), backs up the previous `docker-compose.yml`, updates `STELLARR_VERSION` in `.env`, and preserves all other `.env` settings. Local Compose customizations belong in `docker-compose.override.yml`.
- **From a git checkout**: Run `./install.sh` to use the checkout's files, or `./install.sh --build` to build the container image locally. Run `./install.sh --help` for the full option list.

> [!NOTE]
> If no container image has been published to GHCR for the requested release yet, build from a local checkout instead (`./install.sh --build` or see [Build from source](#build-from-source)).

The steps below walk through performing the same setup manually with Docker Compose.

## 1. Download Compose and environment files

```bash
mkdir stellarr && cd stellarr
curl -fsSLO https://raw.githubusercontent.com/Arkazzae/Stellarr/main/docker-compose.yml
curl -fsSL https://raw.githubusercontent.com/Arkazzae/Stellarr/main/.env.example -o .env
```

All entries in `.env.example` are commented out so the container's built-in defaults apply out of the box for local testing on `localhost`.

## 2. Start the container

```bash
docker compose up -d
```

Docker pulls `ghcr.io/arkazzae/stellarr`, creates the `stellarr_stellarr_data` volume, and starts the `stellarr` container. Check that the container reaches `healthy`:

```bash
docker compose ps
```

The container reports `healthy` once `stellarr-media`, the API and its SQLite database, `coturn`, and Nginx all pass their readiness checks. If startup fails, inspect service state and logs:

```bash
docker compose exec stellarr stellarr-init status
docker compose logs
```

## 3. Open the web client

Open `https://localhost:8443` in your browser.

> [!NOTE]
> On first start, the container generates a 365-day self-signed TLS certificate for `localhost`. Accept the browser's certificate warning to proceed. For remote deployments, configure a trusted certificate as described in [Reverse proxy and TLS](reverse-proxy-and-tls.md).

## 4. Create the owner account and first server

While the instance has no accounts, every web client route redirects to `https://localhost:8443/setup`. Completing setup requires a one-time setup code printed to the container logs so that an unauthenticated visitor cannot claim a fresh instance before the operator:

```bash
docker compose logs stellarr | grep "Setup code"
```

The log entry looks like:

```text
Setup code: K7QM-2XWP-9HDA. Open /setup at the address of this instance to create the owner account.
```

*(When `STELLARR_DOMAIN` or `STELLARR_PUBLIC_URL` is configured, the log prints the full URL such as `https://chat.example.com/setup`. A fresh code is generated on each container restart until the owner account is created; case, spaces, and hyphens are ignored.)*

Complete the three-step `/setup` wizard:

1. **Step 1 (Owner account)**: Enter the setup code, display name, username (`3–32` characters: letters, digits, `.`, `-`, `_`), email address, and password (at least `8` characters). Click **Next: first server**.
2. **Step 2 (First server)**: Enter a server name (`1–80` characters), an optional description, and the first text channel name (default `general`). Leave the voice channel toggle enabled (default `lounge`) to test voice calls, then click **Create account and server**.
3. **Step 3 (Invite friends)**: The wizard creates the owner account and initial server atomically in one transaction (invalidating the setup code), generates an invite link, and displays a **Copy link** button. Copy the invite link, then click **Open server**.

## 5. Invite a second user

If you need to generate another invite link later from inside the app:

1. In the channel rail header, click **Invite to server** next to the server name.
2. Click **New invite link** to create an invite URL:

   ```text
   https://localhost/invite/22b115a390a20370
   ```

3. When testing locally on `localhost:8443` without `STELLARR_DOMAIN` or `STELLARR_PUBLIC_URL` set, include `:8443` in the URL (`https://localhost:8443/invite/22b115a390a20370`).

> [!WARNING]
> When `STELLARR_DOMAIN` and `STELLARR_PUBLIC_URL` are both unset, invite links fall back to the request `Host` header without non-443 ports. Configuring `STELLARR_DOMAIN` (with `STELLARR_PUBLIC_PORT`) or `STELLARR_PUBLIC_URL` ensures generated invite links always include the full public origin.

## 6. Join and test voice

1. Open the invite link in a second browser profile or private window, accept the certificate warning, click **Create an account and join**, and register a second user.
2. In both browser windows, click the voice channel in the channel rail and allow microphone access.
3. Confirm two-way audio and verify that the connection status in the bottom-left call dock shows `Connected`.

> [!IMPORTANT]
> Local testing on a single machine works without additional network configuration because both browsers run on the Docker host. For voice and video calls from any other device (including another machine on your LAN), set `STELLARR_PUBLIC_IP` in `.env` and forward UDP port `49000` (`STELLARR_MEDIA_PORT`) on your router. See [Requirements](requirements.md#behind-nat).

If voice stays on `Connecting` or fails across networks, run through the checklist in [RTC and ports](../rtc-and-ports.md#diagnostics).

## Stop or remove

```bash
docker compose stop      # stop the container
docker compose down      # remove container and network, preserve data volume
```

Both commands stop internal services in reverse dependency order (`stop_grace_period: 75s`, typically finishing in a few seconds). See [Running without Docker Compose](configuration.md#running-without-docker-compose) if using `docker run`.

> [!CAUTION]
> `docker compose down -v` permanently deletes the `stellarr_stellarr_data` volume, destroying all accounts, messages, uploads, encryption keys, and secrets.

## Next steps

- **Operator panel**: Sign in with the owner account and visit `https://localhost:8443/cockpit`.
- [Requirements](requirements.md): `STELLARR_PUBLIC_IP` and router port forwarding for external devices.
- [Reverse proxy and TLS](reverse-proxy-and-tls.md): browser-trusted TLS certificates and reverse proxy configuration.
- [Configuration](configuration.md): full environment variable reference.

## Build from source

To build the production container image directly from a local git checkout:

```bash
git clone https://github.com/Arkazzae/Stellarr.git
cd Stellarr
docker compose up -d --build
```

The multi-stage `Dockerfile` compiles the WebAssembly artifact recipes (`speex`, `rnnoise`, `bergamot`), web client, Cockpit, Rust API (`stellarr-api`), operator CLI (`stellarr`), `stellarr-media`, and `stellarr-init` inside Docker. No host toolchains beyond Docker and BuildKit are required.
