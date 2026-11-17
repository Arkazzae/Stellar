# Platform templates

Each folder here packages the production container for a non-Compose platform. Platforms that install from Compose use [`docker-compose.yml`](../docker-compose.yml) directly rather than a copy of it.

| Platform | Template | Guide |
| --- | --- | --- |
| Unraid | [`unraid/stellarr.xml`](unraid/stellarr.xml) | [`docs/install/unraid.md`](../docs/install/unraid.md) |

## Rules every template keeps

`docker-compose.yml` is the reference. Every template must start the container with matching defaults and limits:

1. **Same image.** `ghcr.io/arkazzae/stellarr:latest` by default, allowing a version tag (such as `1.2`) to stay on one release line.
2. **Same data path.** One persistent folder or volume mounted at `/data/stellarr`.
3. **Same ports and default host ports.** Every port `docker-compose.yml` publishes, with matching protocol and default host port. Keep the TURN relay range (`49160-49200/udp`) mapped 1:1.
4. **Port variables matching host mappings.** The container cannot inspect host port mappings, so templates expose `PUBLIC_HTTPS_PORT`, `Rtc__MediaPort`, `Rtc__TurnPort`, and `Rtc__TurnsPort` and instruct operators to keep them equal to the host ports (see [Running without Docker Compose](../docs/install/configuration.md#running-without-docker-compose)).
5. **Defaults from `docker-compose.yml`.** Use container-side names (`Rtc__MediaPort`, not `STELLARR_MEDIA_PORT`) and set both `STELLARR_LANGUAGE` and `Stellarr__Language`.
6. **No frozen code defaults.** Omit settings whose defaults live in code (desktop client versions, OIDC callback path) so updates keep following new releases.
7. **Same limits.** Apply the memory (`4g`), CPU (`4`), PID (`512`), and `no-new-privileges` limits of `docker-compose.yml` where supported.
8. **Same stop grace period.** Pass `--stop-timeout=75` where extra Docker flags are supported so `stellarr-init` has time to stop services cleanly.
9. **Secrets masked.** Hide password and API key inputs using the platform's secret masking.

When a default in `docker-compose.yml` changes, update every template here to match.

## Adding a platform

1. Place the template in `deploy/<platform>/`, following the rules above.
2. Start the container from the template with a scratch volume and verify it reaches `healthy`, `http://localhost:8888/` redirects to HTTPS, and `docker exec <container> stellarr doctor` passes.
3. Add a platform guide at `docs/install/<platform>.md` covering only platform-specific steps, and link it from [`docs/README.md`](../docs/README.md) and the table above.

## Other folders

- [`cloudflare/backup-relay/`](cloudflare/backup-relay/README.md): optional Cloudflare Worker for offsite backups (see [Cloudflare R2 offsite backups](../docs/admin/cloudflare-r2.md) and [Automatic backups](../docs/admin/backups.md)).
