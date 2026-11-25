# Platform templates

Each subdirectory packages the production Stellarr container for a non-Compose host platform. Platforms that support Docker Compose directly use the root [`docker-compose.yml`](../docker-compose.yml).

| Platform | Template | Guide |
| --- | --- | --- |
| Unraid | [`unraid/stellarr.xml`](unraid/stellarr.xml) | [`docs/install/unraid.md`](../docs/install/unraid.md) |

## Template requirements

[`docker-compose.yml`](../docker-compose.yml) is the single source of truth for published ports and container resource limits, while the container image provides all runtime defaults. Every platform template must adhere to these rules:

1. **Image reference**: Default to `ghcr.io/arkazzae/stellarr:latest` while allowing operators to pin a release line tag (such as `1.2`).
2. **Data mount**: Mount a single persistent host directory or volume at `/data/stellarr`.
3. **Port mappings**: Publish HTTPS and both UDP and TCP media mappings. Use the Compose defaults where the host leaves them available. Bind any published HTTP reverse-proxy upstream to loopback. Publish TURN ports only for an opt-in relay, keeping its UDP relay range mapped 1:1.
4. **Advertised port variables**: Because a container cannot inspect host port mappings, expose `STELLARR_HTTPS_PORT` and `STELLARR_MEDIA_PORT`, plus the two TURN port variables when the template offers TURN, initialized to the chosen host ports and instruct operators to keep them equal to the published host ports (see [Running without Docker Compose](../docs/install/configuration.md#running-without-docker-compose)).
5. **Variable names**: Use the canonical `STELLARR_*` names from [Configuration](../docs/install/configuration.md). Only use raw `Section__Key` environment variables when no `STELLARR_*` name exists.
6. **Unset optional defaults**: Leave all non-port variables empty so the container image's built-in defaults apply across upgrades.
7. **Resource and security limits**: Apply the memory (`4g`), CPU (`4`), PID (`512`), `no-new-privileges`, and capability allowlist from `docker-compose.yml` wherever the platform supports them.
8. **Stop timeout**: Pass `--stop-timeout=75` so `stellarr-init` has time to complete its 60-second ordered service shutdown before Docker sends `SIGKILL`.
9. **Secret masking**: Mark password, token, and API key fields as masked/secret inputs in the platform UI.

Whenever a port, limit, or image reference changes in `docker-compose.yml`, update all templates in this directory to match.

The release workflow verifies that the versioned image can be pulled without signing in before it publishes the GitHub release. It also checks the stable aliases after promotion. After the first image push, the package owner must [make the GHCR package public](https://docs.github.com/en/packages/learn-github-packages/configuring-a-packages-access-control-and-visibility) and rerun the failed verification job if the package starts out private.

## Adding a platform template

1. Add the template under `deploy/<platform>/` following the requirements above.
2. Start a container from the template against an empty data directory and verify that it reaches `healthy`, the template’s HTTPS address serves the setup screen, and `docker exec <container> stellarr doctor` passes.
3. Add a platform installation guide at `docs/install/<platform>.md` covering platform-specific steps, and link it from [`docs/README.md`](../docs/README.md) and the table above.

## Additional deployment utilities

- [`cloudflare/backup-relay/`](cloudflare/backup-relay/README.md): Optional Cloudflare Worker that writes offsite backups to an R2 bucket via a Worker binding (see [Cloudflare R2 offsite backups](../docs/admin/cloudflare-r2.md) and [Automatic backups](../docs/admin/backups.md)).
