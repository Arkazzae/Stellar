# Platforms

Stellarr runs as one container. The [Quick start](quick-start.md) starts it with Docker Compose and [`docker-compose.yml`](../../docker-compose.yml). A platform with its own app format starts the same container from a template in [`deploy/`](../../deploy/README.md).

| Platform | Starts from | Guide |
| --- | --- | --- |
| Docker Compose | [`docker-compose.yml`](../../docker-compose.yml) | [Quick start](quick-start.md) |
| Unraid | [`deploy/unraid/stellarr.xml`](../../deploy/unraid/stellarr.xml) | [Unraid](unraid.md) |

The rest of this page is what any installation gives the container. Compose does all of it already. A template does it too, and the platform guides say which of its fields to fill in.

## Image and data

- The image is `ghcr.io/arkazzae/stellarr:latest`, the newest stable release, or a version tag such as `1.2` to stay on one release line.
- One persistent folder or volume is mounted at `/data/stellarr`. It holds the database, uploaded files, the data-protection keys, the generated secrets and the TLS certificates. Back it up whole, with the container stopped: the secrets stored in the database can be read only with the keys next to it. See [Upgrading](upgrading.md).
- The container is given 75 seconds to stop. Its process manager stops the services one at a time, nginx first and the media server last, and kills whatever is left after 60 seconds; the API alone may take 35. It usually takes a second or two, but Docker's default of 10 seconds would cut a slow stop short. Compose sets `stop_grace_period: 75s`; with plain Docker, create the container with `--stop-timeout=75`, or stop it with `docker stop -t 75`.

## Services in the container

The container runs Nginx, the API, the media server, coturn and a certificate watcher. Its process manager, `stellarr-init`, runs as PID 1: it starts them in order, starts again any that exits, and keeps asking each one whether it answers. Docker's health check asks the manager in turn, so the container is `healthy` only while every service but the watcher is `ready`.

```bash
docker compose exec stellarr stellarr-init status          # each service: state, PID, restarts, uptime
docker compose exec stellarr stellarr-init status --json   # the same, for scripts
docker compose exec stellarr stellarr-init health          # exits 0 only while the container is healthy
docker compose exec stellarr stellarr-init restart api     # stops a service and starts it again
docker compose exec stellarr stellarr-init reload nginx    # reloads Nginx's configuration and certificate
```

The services are `media`, `api`, `turn`, `nginx` and `tls-maintenance`. Their states:

| State | Meaning |
| --- | --- |
| `waiting` | Not started yet: the API waits for the media server to be ready. |
| `starting` | Running, and not answering yet. |
| `ready` | Running and answering. |
| `failing` | Running, but it missed its last three checks, about 45 seconds of silence. The container is unhealthy until it answers again. It is not restarted for this, as a slow machine would only make a restart worse; restart it yourself if it stays stuck. |
| `backoff` | It exited, and starts again after a pause that grows from 1 to 30 seconds with each quick exit. `status` shows when. |
| `stopping`, `stopped` | On a restart or while the container stops. |

Each line the manager writes itself starts with `stellarr-init:` in `docker compose logs`, one for every change of state.

Every service writes to that log and to nothing inside the container. The API logs one line for each request it serves, and its warnings and errors (see [Logs](configuration.md#logs)); Nginx keeps no access log, as the requests it serves without the API are the apps' files, and logs only its errors. The manager's checks every ten seconds leave no line in either: a check that fails shows as the manager's own line when the service turns `failing`.

## Address

- `STELLARR_DOMAIN` is the name or IPv4 address people open Stellarr at, for example `chat.example.com`, or `192.168.1.10` on a home network. The self-signed certificate and TURN use it. When it is empty they use `localhost`, which only a browser on the same machine can reach.
- `PUBLIC_IP` is the IPv4 address browsers reach the host at: its LAN address when everybody is on your network, your public address for friends over the internet, or `auto` to resolve the domain. Without it, voice from any other device works only through TURN. See [Requirements](requirements.md#behind-nat).

## Ports and the variables that name them

The container cannot see which host ports were mapped to it, and it tells browsers which ports to use. Each of these variables names the host side of its port mapping, so a changed host port means a changed variable:

| Purpose | Container port | Default host port | Container variable | Compose variable |
| --- | --- | --- | --- | --- |
| HTTPS | `443/tcp` | `8443` | `PUBLIC_HTTPS_PORT` | `STELLARR_HTTPS_PORT` |
| Voice media | `49000/udp` | `49000` | `Rtc__MediaPort` | `STELLARR_MEDIA_PORT` |
| TURN | `3478/udp`, `3478/tcp` | `3478` | `Rtc__TurnPort` | `STELLARR_TURN_PORT` |
| TURNS | `5349/tcp`, `5349/udp` | `5349` | `Rtc__TurnsPort` | `STELLARR_TURNS_PORT` |

Compose sets both sides from one variable. The HTTP redirect (`80/tcp`, host `8888`) needs no variable. The TURN relay range `49160-49200/udp` must use the same numbers on the host as in the container. [Requirements](requirements.md#ports) describes what each port is for.

## Language

`STELLARR_LANGUAGE` is the web client's default language, and `Stellarr__Language` is the language of server messages, mail, and the roles and channels of a new server. Compose fills both from one value. Anywhere else, set both to the same value, `en` or `pl`.

## Other settings

Every other setting is optional. A template uses the name the container reads, which `docker-compose.yml` shows next to each Compose variable: `STELLARR_SMTP_PASSWORD` in `.env` reaches the container as `Mail__SmtpPassword`, for example. [Configuration](configuration.md) describes them.

Without `Operations__TimeZone`, the instance time zone starts from the container's `TZ`, or UTC when that is unset. It can be changed later in Cockpit.

Compose limits the container to 4 GB of memory, 4 CPUs and 512 processes, and sets `no-new-privileges`. See [Container resource limits](configuration.md#container-resource-limits).
