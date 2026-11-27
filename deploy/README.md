# Platform Templates (`deploy/`)

The default [Compose deployment](../docker-compose.yml) runs one container. This directory contains the deployment with separate service images and platform integrations.

| Target | Artifact | Documentation |
| --- | --- | --- |
| **Compose services** | [`services/compose.yml`](services/compose.yml) | [Setup and operations](services/README.md) |
| **Unraid** | [`unraid/stellarr.xml`](unraid/stellarr.xml) | [`docs/install/unraid.md`](../docs/install/unraid.md) |
| **Cloudflare R2 Relay Worker** | [`cloudflare/backup-relay/`](cloudflare/backup-relay/README.md) | [`docs/admin/cloudflare-r2.md`](../docs/admin/cloudflare-r2.md) |

## Template contract

[`docker-compose.yml`](../docker-compose.yml) defines the canonical ports, capabilities, and resource limits; the image provides runtime defaults. Templates for the single-container image follow these rules:

1. **Image tag:** Default to `ghcr.io/arkazzae/stellarr:latest` while allowing operators to pin a series tag (e.g. `1.2`).
2. **Single volume:** Mount one persistent host path or volume at `/data/stellarr`.
3. **Ports:** Publish HTTPS plus **both** UDP and TCP on the media port (`49000` default), bind any HTTP proxy upstream to loopback, and publish TURN ports (`49160–49200/udp` mapped 1:1) only when TURN is opted in.
4. **Matching host-port variables:** Expose `STELLARR_HTTPS_PORT` and `STELLARR_MEDIA_PORT` (plus TURN port variables when applicable) initialized to the template's host ports ([see why](../docs/install/configuration.md#running-without-docker-compose)).
5. **Canonical variable names:** Use `STELLARR_*` names from [Configuration](../docs/install/configuration.md), leaving optional non-port fields blank so built-in image defaults apply.
6. **Limits & stop grace:** Match `4g` memory, `4` CPUs, `512` PIDs, `no-new-privileges`, and `--stop-timeout=75` (`stellarr-init` requires `> 60 s` for ordered shutdown).
7. **Masked secrets:** Mark password, token, and API key inputs as secret/masked fields in the host UI.

## Adding a new platform template

1. Create `deploy/<platform>/` satisfying all rules above.
2. Boot against an empty `/data/stellarr` directory and verify `healthy` status, `/setup` reachability over HTTPS, and `docker exec <container> stellarr doctor`.
3. Add `docs/install/<platform>.md` and link it from [`docs/README.md`](../docs/README.md) and the table above.
