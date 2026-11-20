# Quick start

This guide starts a local Stellarr instance on a single machine, creates the owner account, invites a second user, and tests voice chat between two browser profiles.

## Prerequisites

- Docker Engine with the Compose plugin (`docker compose version`). See [Requirements](requirements.md).
- Free host ports `8443`, `8888`, `3478`, `5349`, `49000`, and `49160-49200`. If a port is in use, override the corresponding variable in `.env` (see [Requirements](requirements.md#ports)).
- A second browser profile or private window to test a two-user call.

## One-click install

One command checks the machine, writes the installation's files, starts the container and prints the address and the setup code:

```bash
curl -fsSL https://stellarr.online/install.sh | bash
```

In a terminal it asks up to three things: the domain or IP address people will open, the IPv4 address voice calls use when that is a domain, and the interface language, English or Polish. On Linux it offers to install Docker Engine when Docker is missing. With `--yes`, or without a terminal, it asks nothing and takes its options and defaults instead:

```bash
curl -fsSL https://stellarr.online/install.sh | bash -s -- --yes --domain chat.example.com --public-ip auto
```

It installs into `~/stellarr`, unless `--dir` names another directory, the current directory already holds Stellarr's `docker-compose.yml`, or Docker already runs Stellarr from another directory. When a port it needs is in use, it offers the next free one in a terminal and otherwise stops before it changes anything; it also stops when an installation it did not make is in the way.

Running it again updates Stellarr to the latest release. It replaces `docker-compose.yml` with the release's own, keeping the previous one beside it, sets `STELLARR_VERSION` in `.env`, and leaves the rest of `.env` as it is; local changes belong in `.env` or in `docker-compose.override.yml`. In a checkout, `./install.sh` uses the checkout's files, and `./install.sh --build` builds the image from it. `--help` lists every option.

Until the first release is published there is no image to pull, and the installer says so; build the image from a checkout instead, as in [Build from source](#build-from-source).

The rest of this guide describes the same steps by hand.

## 1. Download Compose and environment files

```bash
mkdir stellarr && cd stellarr
curl -fsSLO https://raw.githubusercontent.com/Arkazzae/Stellarr/main/docker-compose.yml
curl -fsSL https://raw.githubusercontent.com/Arkazzae/Stellarr/main/.env.example -o .env
```

All variables in `.env` have defaults suitable for local testing on `localhost`.

## 2. Start the container

```bash
docker compose up -d
```

Docker pulls `ghcr.io/arkazzae/stellarr`, creates the `stellarr_stellarr_data` volume, and starts the `stellarr` container. Verify that it is healthy:

```bash
docker compose ps
```

The container reports `healthy` once Nginx, the API and database, `stellarr-media`, and `coturn` all pass their readiness checks. If startup fails, inspect service state and logs:

```bash
docker compose exec stellarr stellarr-init status
docker compose logs
```

## 3. Open the web client

Open `https://localhost:8443` in your browser.

> [!NOTE]
> On first start, the container generates a 365-day self-signed TLS certificate for `localhost`. Accept the browser's certificate warning to continue. For remote deployments, configure a trusted certificate as described in [Reverse proxy and TLS](reverse-proxy-and-tls.md).

## 4. Create the owner account

Until the instance has an account, the web client opens its setup page, `https://localhost:8443/setup`. Creating the owner account there takes a setup code that the container writes to its log, so nobody who reaches the address before you can take the instance over. Find it with:

```bash
docker compose logs stellarr | grep "Setup code"
```

`docker logs stellarr` shows the same log. The line reads:

```text
Setup code: K7QM-2XWP-9HDA. Open /setup at the address of this instance to create the owner account.
```

With `STELLARR_DOMAIN` or `STELLARR_PUBLIC_URL` set, it gives the full address instead, such as `https://chat.example.com/setup`. Every start of the container draws a new code, so after a restart use the last line. Case, spaces and dashes do not matter when you type it.

1. Enter the setup code and the owner account: a display name, username (3–32 characters: letters, digits, `.`, `-`, `_`), email, and password (at least 8 characters). Select **Next: first server**.
2. Continue to the server step. Choose a server name, an optional description, and the first text channel. Keep the voice channel enabled if you want to test voice next; you can rename or remove it later.
3. Select **Create account and server**.

The first account becomes the instance owner and is created together with its server, in the last step, so a page closed halfway leaves nothing behind. After that the setup code stops working. Subsequent accounts require an invite link and skip server setup.

## 5. Invite a second user

The last setup step shows an invite link with **Copy**. To make another one later:

1. In the channel list header, click **Invite to server** next to the server name.
2. Click **New invite link** to generate an invite URL:

   ```text
   https://localhost/invite/22b115a390a20370
   ```

3. If you are testing on `localhost:8443` without `STELLARR_DOMAIN` or `STELLARR_PUBLIC_URL` set, add `:8443` before opening the link (for example, `https://localhost:8443/invite/22b115a390a20370`).

> [!WARNING]
> When `STELLARR_DOMAIN` and `STELLARR_PUBLIC_URL` are unset, invite URLs fall back to the request `Host` header without non-443 ports. Once `STELLARR_DOMAIN` (with `STELLARR_PUBLIC_PORT`) or `STELLARR_PUBLIC_URL` is configured, invite links include the public origin automatically.

## 6. Join and test voice

1. Open the invite link in a second browser profile or private window, accept the certificate warning, select **Create an account and join**, and register a second account.
2. In both browser windows, join the voice channel you configured during setup and grant microphone access.
3. Verify two-way audio and check the connection indicator (`Connected`) in the bottom-left status bar.

> [!IMPORTANT]
> Local testing on one machine works without extra network setup because both browsers run on the Docker host. For calls from any other device (including another machine on your LAN), set `PUBLIC_IP` in `.env` and forward UDP port `49000` (`STELLARR_MEDIA_PORT`) on your router. See [Requirements](requirements.md#behind-nat).

If voice stays on connecting or fails across networks, follow the checklist in [RTC and ports](../rtc-and-ports.md#diagnostics).

## Stop or remove

```bash
docker compose stop      # stop the container
docker compose down      # remove container and network, preserve data volume
```

Both commands stop services gracefully (Compose allows up to `stop_grace_period: 75s`, though shutdown normally finishes in a couple of seconds). See [Running without Docker Compose](configuration.md#running-without-docker-compose) if using plain `docker run`.

> [!CAUTION]
> `docker compose down -v` permanently deletes the `stellarr_stellarr_data` volume, including all accounts, messages, uploads, and secrets.

## Next steps

- **Operator panel**: Sign in with the owner account and open `https://localhost:8443/cockpit`.
- [Requirements](requirements.md): `PUBLIC_IP` and router port forwarding for voice from external devices.
- [Reverse proxy and TLS](reverse-proxy-and-tls.md): trusted certificates and reverse proxy setup.
- [Configuration](configuration.md): environment variable reference.

## Build from source

To build the container image from a local checkout instead of pulling from GHCR:

```bash
git clone https://github.com/Arkazzae/Stellarr.git
cd Stellarr
docker compose up -d --build
```

The multi-stage `Dockerfile` builds the web client, Cockpit, .NET API, Rust media server, and `stellarr-init` inside Docker. No local toolchains are required on the host.
