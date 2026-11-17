# Unraid

Stellarr ships an Unraid template at [`deploy/unraid/stellarr.xml`](../../deploy/unraid/stellarr.xml) with the same image, defaults, and resource limits as [`docker-compose.yml`](../../docker-compose.yml). See [Configuration](configuration.md#running-without-docker-compose) for container variable mapping and [Requirements](requirements.md) for host resources and NAT forwarding.

## 1. Add the template

Stellarr is not in Community Applications. Download the user template from the Unraid terminal:

```bash
wget -O /boot/config/plugins/dockerMan/templates-user/my-Stellarr.xml \
  https://raw.githubusercontent.com/Arkazzae/Stellarr/main/deploy/unraid/stellarr.xml
```

Open **Docker → Add Container**, select **Stellarr** under **Template**, and review the settings below before clicking **Apply**.

## 2. Configure required settings

1. **Domain and Public IP**: Fill in **Domain** (`STELLARR_DOMAIN`) and **Public IP** (`PUBLIC_IP`) so browsers outside the Unraid host can connect and join voice calls.
2. **Ports**: Unraid uses ports `80` and `443` for its own web UI, so the template defaults avoid them (`8443` HTTPS, `8888` HTTP). Whenever you change a host port mapping, update its matching variable to the same number:

   | Port mapping | Matching variable |
   | --- | --- |
   | **HTTPS port** (`8443`) | **HTTPS host port** (`PUBLIC_HTTPS_PORT`) |
   | **Voice media port** (`49000`) | **Voice media host port** (`Rtc__MediaPort`) |
   | **TURN port** (`3478`, advanced) | **TURN host port** (`Rtc__TurnPort`) |
   | **TURNS port** (`5349`, advanced) | **TURNS host port** (`Rtc__TurnsPort`) |

   Keep **TURN relay ports** mapped 1:1 (`49160-49200` on both host and container).

3. **Router forwarding**: For calls from outside your LAN, forward UDP `49000`, `3478`, and `49160-49200` to your Unraid server on the same external ports, plus either the HTTPS port or a [reverse proxy](reverse-proxy-and-tls.md).
4. **Language and time zone**: Set **Interface language** (`STELLARR_LANGUAGE`) and **Server language** (`Stellarr__Language`) to the same value (`en` or `pl`). Unraid passes its own `TZ` to the container as the initial time zone.

## 3. First start

Start the container and click **WebUI** from its menu on the Docker tab. Accept the self-signed certificate warning, then create the owner account as described in the [Quick start](quick-start.md#4-create-the-owner-account).

For a browser-trusted certificate or reverse proxy setup, see [Reverse proxy and TLS](reverse-proxy-and-tls.md).

## Stopping, backups, and CLI

- **Stop timeout**: The template's **Extra Parameters** include `--stop-timeout=75` so services stop cleanly. Keep that flag if you edit Extra Parameters.
- **Data path**: `/mnt/user/appdata/stellarr` by default. Keep `appdata` on a cache/pool drive and back up the whole folder while the container is stopped (see [Upgrading](upgrading.md)).
- **CLI and service checks**: Because the Unraid container is named `Stellarr`, run [`stellarr`](../admin/cli.md) and [`stellarr-init`](../architecture.md#container-services-stellarr-init) against `Stellarr`:

  ```bash
  docker exec -it Stellarr stellarr doctor
  docker exec -it Stellarr stellarr-init status
  ```

## Updating

The template tracks `ghcr.io/arkazzae/stellarr:latest`. Clicking **Apply update** on the Docker tab pulls the newest image while keeping your template settings and `/mnt/user/appdata/stellarr` data. To stay on a specific release line, set **Repository** to a tag such as `ghcr.io/arkazzae/stellarr:1.2`. Review [Upgrading](upgrading.md) before major upgrades.
