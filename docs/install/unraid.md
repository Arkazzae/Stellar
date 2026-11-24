# Unraid

Stellarr provides an Unraid template at [`deploy/unraid/stellarr.xml`](../../deploy/unraid/stellarr.xml) with the same image and resource limits as [`docker-compose.yml`](../../docker-compose.yml). Template variables use the `STELLARR_*` names listed in [Configuration](configuration.md), and any blank field falls back to the container image's built-in default. See [Running without Docker Compose](configuration.md#running-without-docker-compose) for host port variable rules and [Requirements](requirements.md) for host resources and NAT forwarding.

## 1. Install the user template

Stellarr is not published in Community Applications. Download the user template from the Unraid web terminal or SSH session:

```bash
wget -O /boot/config/plugins/dockerMan/templates-user/my-Stellarr.xml \
  https://raw.githubusercontent.com/Arkazzae/Stellarr/main/deploy/unraid/stellarr.xml
```

Open **Docker → Add Container**, select **Stellarr** under **Template**, and review the settings below before clicking **Apply**.

## 2. Configure required settings

1. **Domain and Public IP**: Set **Domain** (`STELLARR_DOMAIN`) to the address users open. If it is already the IPv4 address that browsers can reach for media, direct calls can use it. Otherwise set **Public IP** (`STELLARR_PUBLIC_IP`) to the Unraid LAN IPv4 for LAN-only calls, or a reachable public IPv4 / `auto` for internet access.
2. **Ports**: Unraid uses ports `80` and `443` for its own web UI, so the template defaults to `8443/tcp` for HTTPS. Whenever you change a host port mapping in Unraid, update its matching environment variable to the same port number:

   | Port mapping | Matching variable |
   | --- | --- |
   | **HTTPS port** (`8443`) | **HTTPS host port** (`STELLARR_HTTPS_PORT`) |
   | **Voice media port (UDP and TCP)** (`49000`) | **Voice media host port** (`STELLARR_MEDIA_PORT`) |

   Keep the UDP and TCP media mappings on the same host port. The template uses direct media without TURN.

3. **Router port forwarding**: For calls from outside your LAN, forward `49000/udp` and `49000/tcp` on your router to the Unraid server on the same external port, plus either the HTTPS port or an external [reverse proxy](reverse-proxy-and-tls.md).
4. **Language and time zone**: Set **Language** (`STELLARR_LANGUAGE`) to `en` or `pl` to configure both the default web client locale and the server language. Unraid passes its host time zone to the container automatically.

## 3. First start

Start the container and click **WebUI** from its menu on the Docker tab. Accept the self-signed certificate warning, then complete the three-step `/setup` wizard as described in the [Quick start](quick-start.md#4-create-the-owner-account-and-first-server). Retrieve the required setup code by clicking **Logs** in the container's menu on the Docker tab and copying the latest `Setup code:` line.

For a browser-trusted TLS certificate or reverse proxy configuration, see [Reverse proxy and TLS](reverse-proxy-and-tls.md).

## Stopping, backups, and CLI

- **Stop timeout**: The template's **Extra Parameters** include `--stop-timeout=75` so `stellarr-init` can complete its 60-second ordered service shutdown before Docker sends `SIGKILL`. Preserve that flag if you edit Extra Parameters.
- **Data path**: `/mnt/user/appdata/stellarr` by default. Keep `appdata` on an SSD cache/pool drive and back up the entire directory while the container is stopped (see [Upgrading](upgrading.md)).
- **CLI and service checks**: Because the Unraid container is named `Stellarr` (capital `S`), run [`stellarr`](../admin/cli.md) and [`stellarr-init`](../architecture.md#container-services-stellarr-init) against `Stellarr`:

  ```bash
  docker exec -it Stellarr stellarr doctor
  docker exec -it Stellarr stellarr-init status
  ```

## Updating

The template tracks `ghcr.io/arkazzae/stellarr:latest`. Clicking **Apply update** on the Unraid Docker tab pulls the newest image while preserving your template settings and `/mnt/user/appdata/stellarr` volume. To pin a specific release line, set **Repository** to a version tag such as `ghcr.io/arkazzae/stellarr:1.2`. Review [Upgrading](upgrading.md) before major upgrades.
