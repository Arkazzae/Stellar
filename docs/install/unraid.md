# Unraid

Stellarr includes a ready-to-use Unraid template at [`deploy/unraid/stellarr.xml`](../../deploy/unraid/stellarr.xml) matching the image, capabilities, and resource caps of [`docker-compose.yml`](../../docker-compose.yml). Template fields map 1:1 to `STELLARR_*` variables in [Configuration](configuration.md), and blank optional fields inherit built-in image defaults.

## 1. Install the user template

Run from the Unraid web terminal or SSH:

```bash
wget -O /boot/config/plugins/dockerMan/templates-user/my-Stellarr.xml \
  https://raw.githubusercontent.com/Arkazzae/Stellarr/main/deploy/unraid/stellarr.xml
```

Open **Docker → Add Container**, select **Stellarr** under **Template**, and review the settings below before clicking **Apply**.

*(To test a local checkout build instead of GHCR, run `docker buildx build --load --tag stellarr-local:dev .` and set **Repository** to `stellarr-local:dev`.)*

## 2. Configure host settings and port pairs

1. **Domain & media IP:** Set **Domain** (`STELLARR_DOMAIN`) to the hostname or IP users open. Set **Public IP** (`STELLARR_PUBLIC_IP`) to your Unraid LAN IPv4 for LAN-only use, or to your WAN IPv4 / `auto` for internet access. Set **LAN IP for voice** (`STELLARR_MEDIA_LAN_IP`) to the Unraid LAN IPv4 if local callers cannot hairpin through your router's WAN address.
2. **Matching port pairs:** Unraid reserves `80` and `443` for its own UI, so the template maps HTTPS to `8443/tcp`. Because containers cannot inspect Unraid's host port mappings ([see why](configuration.md#running-without-docker-compose)), keep each port mapping and its matching variable identical:

   | Unraid port mapping | Matching template variable | Default |
   | --- | --- | --- |
   | **HTTPS port** | **HTTPS host port** (`STELLARR_HTTPS_PORT`) | `8443` |
   | **Voice media port (UDP and TCP)** | **Voice media host port** (`STELLARR_MEDIA_PORT`) | `49000` |

3. **Router forwarding:** For internet access, forward `49000/udp` and `49000/tcp` on the same external port to Unraid, plus either `8443/tcp` or `443/tcp` to your reverse proxy.
4. **Language:** Set **Language** (`STELLARR_LANGUAGE`) to `en` or `pl`. Unraid injects the host time zone automatically.

## 3. Complete initial setup

Start the container, open **Logs** from its Docker menu to copy the `Setup code:` line, then click **WebUI**, accept the self-signed certificate warning, and complete the three-step `/setup` wizard ([Quick start: Step 4](quick-start.md#4-create-the-owner-account-and-first-server)).

---

## Nginx Proxy Manager on the same Unraid host

Attach both containers to a shared user-defined Docker bridge with a fixed IP for Nginx Proxy Manager (NPM) so `STELLARR_TRUSTED_PROXIES` survives container updates:

```bash
docker network create --subnet 172.29.35.0/24 stellarr-proxy
```

1. **NPM container:** Set **Network Type** to `stellarr-proxy` and pin its IP (e.g. `--ip=172.29.35.10` in **Extra Parameters**). Create a **Proxy Host** forwarding `http://Stellarr:80` with **Websockets Support** enabled, a TLS certificate on the **SSL** tab, and `client_max_body_size 128m;` under **Advanced** (see [Proxy requirements](reverse-proxy-and-tls.md#proxy-requirements)).
2. **Stellarr container:** Set **Network Type** to `stellarr-proxy`, **Domain** to your public hostname, **Public HTTPS port** (`STELLARR_PUBLIC_PORT`) to `443`, and **Trusted proxies** (`STELLARR_TRUSTED_PROXIES`) to `172.29.35.10`.
3. **Router:** Point external `443/tcp` to NPM and forward `49000/udp` + `49000/tcp` directly to the Unraid host.

---

## Operations, backups, and updates

| Task | Unraid specifics |
| --- | --- |
| **Graceful stop** | Preserve `--stop-timeout=75` in **Extra Parameters** so [`stellarr-init`](../architecture.md#container-services-stellarr-init) completes its 60 s ordered shutdown before `SIGKILL`. |
| **Data directory** | `/mnt/user/appdata/stellarr` — keep `appdata` on an SSD pool and archive it while stopped before major upgrades ([Upgrading](upgrading.md)). |
| **Operator CLI** | Container name uses a capital `S`: run `docker exec -it Stellarr stellarr doctor` and `docker exec -it Stellarr stellarr-init status` ([Operator CLI](../admin/cli.md)). |
| **Updating** | Click **Apply update** on the Docker tab (`ghcr.io/arkazzae/stellarr:latest` default, or pin a series tag like `ghcr.io/arkazzae/stellarr:1.2`). |
