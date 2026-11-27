# Stellarr Init (`stellarr-init`)

`stellarr-init` is the PID 1 process supervisor inside the production Stellarr container. It starts container services in dependency order, runs active readiness and liveness probes, restarts exited processes with exponential backoff, reaps zombie and orphaned children, and performs ordered shutdown on `SIGTERM` / `SIGINT` (`docker stop`). It also exposes a local Unix domain control socket for status queries, restarts, and signal reloads.

The crate is written in Rust, depends only on the Rust standard library and `libc`, and targets Linux (`amd64` and `arm64`).

## CLI commands

```bash
stellarr-init run                  # run as container PID 1 supervisor
stellarr-init status [--json]      # report state, PID, restart count, and uptime for all services
stellarr-init health               # exit 0 only when every critical service is ready
stellarr-init restart SERVICE      # stop a service and restart it immediately
stellarr-init reload SERVICE       # send a service its configured reload signal
```

Pass `--socket PATH` to use a control socket path other than `/run/stellarr/init.sock`. Client subcommands exit `0` on success, `1` on failure or unhealthy state, and time out with exit code `1` after 3 seconds if the supervisor does not respond, making `stellarr-init health` suitable for the container `HEALTHCHECK`.

| `stellarr-init run` exit code | Meaning |
| --- | --- |
| `0` | Clean shutdown after `SIGTERM` or `SIGINT` completed (including any processes that required `SIGKILL` at deadline) |
| `1` | Fatal supervisor error (`signalfd`, control socket bind, or `poll` failure) |
| `2` | Invalid compiled service table (dependency cycle, unknown dependency, or invalid service name) |

## Supervised services

The service table is compiled into `src/services.rs`. The `turn` service runs and contributes to container health only when `STELLARR_TURN_ENABLED=true`.

| Service | User | Dependencies | Readiness / liveness probe | Stop signal & timeout | Reload signal | Critical |
| --- | --- | --- | --- | --- | --- | --- |
| `media` | `nobody` | — | `GET http://127.0.0.1:5200/v1/health` returns `2xx` | `SIGTERM`, `5 s` | — | yes |
| `api` | `root` | `media` | `GET http://127.0.0.1:5000/health/ready` returns `2xx` (verifies SQLite readiness) | `SIGTERM`, `35 s` | — | yes |
| `turn` | `root` | — | STUN Binding Request to `127.0.0.1:3478` returns a valid STUN response | `SIGTERM`, `5 s` | — | yes |
| `nginx` | `root` | — | `GET http://127.0.0.1:80/index.html` with `X-Forwarded-Proto: https` returns `2xx` | `SIGTERM`, `10 s` | `SIGHUP` | yes |
| `tls-maintenance` | `root` | — | Process remains alive `1 s` after spawn | `SIGTERM`, `5 s` | — | no |

The `nginx` probe queries port `80` with `X-Forwarded-Proto: https` to verify HTTP serving without requiring a TLS client inside `stellarr-init` (without the header, port `80` returns an `https://` redirect).

On container shutdown, services stop one at a time in reverse startup order, each bounded by its per-service stop timeout, with an overall 60-second deadline before any remaining processes receive `SIGKILL`. The container's stop grace period must exceed 60 seconds (`docker-compose.yml` sets `stop_grace_period: 75s`; the Unraid template sets `--stop-timeout=75`).

## Supervision behavior

- **Process isolation**: Each service spawns in its own process group (`setpgid`) with stdin attached to `/dev/null`, stdout/stderr inherited from the supervisor, an empty signal mask, and all internal supervisor file descriptors closed.
- **Dependency ordering**: A service starts only after every service in its `Dependencies` list reaches `ready`. If a dependency later transitions out of `ready`, already-running dependent services are not stopped.
- **Startup readiness probing**: While a service is in `starting`, its probe runs every `500 ms` with a `2 s` timeout. A service that never passes its readiness probe remains in `starting` (causing `stellarr-init health` to return `1`), but is not killed automatically.
- **Steady-state liveness probing**: Once `ready`, a service's probe runs every `10 s` with a `5 s` timeout. After three consecutive probe failures (~45 seconds), the service state changes to `failing` (`status` reports `failing` and `health` exits `1`). A single passing probe restores the service to `ready`. Probe failures deliberately do not trigger automatic process restarts so transient host CPU or disk stalls do not cause cascading restart loops; operators can run `stellarr-init restart <service>` to force a restart. The one exception is `media`, whose probe is answered by the task that carries every call: instead of being left `failing`, it is stopped as on `restart` (`SIGTERM`, then `SIGKILL` to its process group after `5 s`) and started again after the crash backoff, so `status` shows `stopping`, `backoff` and `starting`, `restarts` counts it, and two such restarts are always at least the ~45 seconds of missed probes apart.
- **Crash backoff**: Whenever a service process exits, `stellarr-init` kills any leftover processes in its process group with `SIGKILL` (preventing orphaned children from holding ports) and restarts the service with exponential backoff (`1 s`, `2 s`, `4 s`, `8 s`, `16 s`, capped at `30 s`). A run lasting at least `60 s` resets the backoff counter.
- **Non-blocking logging**: State transitions are logged to stderr with a `stellarr-init:` prefix from a dedicated writer thread so a blocked stderr pipe never stalls the PID 1 event loop. If the bounded log queue fills, dropped lines are counted and reported once the pipe drains.

## Control socket protocol

The Unix domain socket at `/run/stellarr/init.sock` is created with mode `0600` and verifies that the connecting peer's effective UID is `0` via `SO_PEERCRED`. Each request writes a single line and reads response lines until EOF, subject to a 2-second connection deadline (`src/protocol.rs`):

```text
> status
< ok
< manager phase=running pid=1
< service media state=ready pid=41 restarts=0 up=1763300000123 next=- critical=yes
< service api state=backoff pid=- restarts=3 up=- next=1763300012000 critical=yes

> restart api
< ok restarting api

> reload media
< error media cannot reload; restart it instead
```

Service states are `waiting`, `starting`, `ready`, `failing`, `backoff`, `stopping`, and `stopped`. Timestamps (`up`, `next`) are milliseconds since the Unix epoch.

## Testing

Because `stellarr-init` uses Linux-specific system calls (`signalfd`, `unshare`, `prctl`, `SO_PEERCRED`), build and test it on Linux or inside a privileged Linux container (required for PID namespace tests):

```bash
docker run --rm --privileged -v "$PWD":/src -w /src rust:1-slim-bookworm \
  sh -c 'rustup component add rustfmt clippy && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test'
```

- `src/`: Unit tests for the control socket wire protocol, `/etc/passwd` and `/etc/group` parsers, HTTP status parsing, STUN packet building/parsing, and the compiled service table.
- `tests/core.rs`: Deterministic sans-IO supervisor state machine tests on a simulated clock covering dependency ordering, cycle/validation errors, state transitions, backoff curves, probe failure thresholds, shutdown deadlines, and randomized event-sequence invariants.
- `tests/pid1.rs`: Integration scenarios spawning `stellarr-init` as PID 1 inside isolated PID namespaces (`unshare`) against adversarial child processes (`tests/support/misbehave.rs`): immediate exits, crashes, hung readiness, transient stalls, stalls that get a service restarted, `SIGTERM`-ignoring process trees, orphan processes, stdout/stderr floods, `SIGHUP` reloads, TCP/HTTP/STUN probes, malformed/unprivileged/slow socket clients, `SIGTERM` during startup/backoff/restart, `SIGINT`, and running as a `PR_SET_CHILD_SUBREAPER` process. Run a single scenario with `cargo test --test pid1 -- <name>` (`PID1_VERBOSE=1` prints supervisor logs).

## Crate layout

- `src/core.rs`: Deterministic sans-IO supervisor state machine driven by an injected clock and `Io` trait.
- `src/service.rs`: Service definition types and topological dependency validation.
- `src/services.rs`: Compiled service table for the Stellarr container.
- `src/manager.rs`: Linux event loop (`poll` over `signalfd`, control socket, and non-blocking readiness probes, plus `waitpid(-1)` reaping and process spawning).
- `src/probe.rs`: Non-blocking TCP, HTTP, and STUN readiness probes with deadlines.
- `src/control.rs`, `src/protocol.rs`, `src/client.rs`: Control socket server, wire protocol parser/formatter, and CLI client subcommands.
- `src/log.rs`: Dedicated non-blocking stderr logging thread.
- `src/accounts.rs`: Minimal `/etc/passwd` and `/etc/group` resolver for dropping privileges.
- `src/sys.rs`: Thin wrappers around `libc` calls, with safety invariants documented on each `unsafe` block.
