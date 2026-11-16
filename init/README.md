# Stellarr Init

Stellarr Init (`stellarr-init`) is the process manager of the Stellarr container. It runs as PID 1, starts the services in the order their needs allow, checks that each is ready, restarts what exits, reaps every child and orphan, and stops everything in order on `docker stop`. A control socket reports the state of each service and takes restart and reload requests.

It depends on the standard library and `libc` only, and runs on Linux (`amd64` and `arm64`).

## Commands

```bash
stellarr-init run                  # the container's PID 1
stellarr-init status [--json]      # every service: state, PID, restarts, uptime
stellarr-init health               # exit 0 only when every critical service is ready
stellarr-init restart SERVICE      # stop a service and start it again at once
stellarr-init reload SERVICE       # send a service its reload signal
```

`--socket PATH` points any of them at another socket than `/run/stellarr/init.sock`. The client subcommands exit 0 on success, 1 on anything else, and 1 within three seconds when the manager does not answer, so `health` can serve as the container's `HEALTHCHECK`.

| Exit status of `run` | Meaning |
| --- | --- |
| 0 | A shutdown on SIGTERM or SIGINT completed, even if it had to kill |
| 1 | The manager itself failed: no signalfd, no socket, a failing `poll` |
| 2 | The service table cannot run: a cycle, an unknown need, a bad name |

## Services

The table is compiled in, in `src/services.rs`:

| Service | Runs as | Needs | Its check | Stop | Reload | Critical |
| --- | --- | --- | --- | --- | --- | --- |
| `media` | `nobody` | | `GET 127.0.0.1:5200/v1/health` answers 2xx | SIGTERM, 5 s | | yes |
| `api` | root | `media` | `GET 127.0.0.1:5000/health/ready` answers 2xx, which needs the database | SIGTERM, 35 s | | yes |
| `turn` | root | | a STUN binding request to `127.0.0.1:3478` succeeds | SIGTERM, 5 s | | yes |
| `nginx` | root | | `GET 127.0.0.1:80/index.html` with `X-Forwarded-Proto: https` answers 2xx | SIGTERM, 10 s | SIGHUP | yes |
| `tls-maintenance` | root | | still running after 1 s | SIGTERM, 5 s | | no |

Together the checks cover what the container's health check asked before this manager: the API and its database, nginx serving, the media server's control API, and a STUN answer from coturn. nginx is asked over plain HTTP, as a proxy in front would ask, because the check speaks no TLS; without the header the answer would be a redirect.

A shutdown stops them one at a time in the reverse of the start order, each with its own deadline, and kills whatever is left 60 s after it began. The container's stop grace period must be longer than that: `docker-compose.yml` sets `stop_grace_period: 75s`, and the Unraid template `--stop-timeout=75`.

## Behaviour

- Each service runs in a process group of its own, with the manager's stdout and stderr, `/dev/null` on stdin, no signal blocked and no descriptor of the manager's.
- A service starts only once every service it needs is ready, each time it starts. A need that stops being ready later does not stop the services that need it.
- At start, the check is asked every half second, each time given two seconds. A service that never becomes ready stays `starting`, and so does not satisfy `health`; it is not killed for it.
- Once ready, the check is asked again every 10 s, each time given 5 s. After three misses in a row, about 45 s of silence, the service is `failing`: `status` shows it and `health` fails. One check that passes makes it `ready` again. A service is never restarted for failing its check, only reported: on a machine that is only slow, a restart would throw away a process that was about to answer, and then the next one while it starts, where an unhealthy mark costs nothing and tells the operator. `stellarr-init restart` is there for a service that is really stuck.
- A service that exits, with any status, starts again after a wait that doubles with each short run, from 1 s up to 30 s. A run of a minute or more starts the count again. There is no limit on the number of tries.
- When a service's main process exits, what is left of its process group gets SIGKILL, so that the next run never finds the old one's port still held.
- The manager writes one line on stderr, prefixed `stellarr-init:`, for each state change. A thread of its own writes them, so a full pipe never holds up the manager; lines that do not fit are counted and the count is written later.

## Control socket

`/run/stellarr/init.sock`. The socket file is mode 0600 and the manager checks the peer's user ID too; the folder is created for root alone when it does not exist, but the entrypoint creates it first, for the file nginx serves from it. A client writes one line and reads until the manager closes the connection; every connection has two seconds. The wire format is in `src/protocol.rs`:

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

States are `waiting`, `starting`, `ready`, `failing`, `backoff`, `stopping` and `stopped`; times are milliseconds since the Unix epoch.

## Testing

The crate builds on Linux only. On another system, run everything in a privileged container, which the PID-namespace tests need:

```bash
docker run --rm --privileged -v "$PWD":/src -w /src rust:1-slim-bookworm \
  sh -c 'rustup component add rustfmt clippy && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test'
```

- `src/`: unit tests of the wire format, the account files, the HTTP status line, STUN packets and the compiled-in table.
- `tests/core.rs`: the decisions on a fake clock with no process: the backoff, the start order and its errors, every state transition, the checks of a ready service and when it counts as failing, the shutdown plan and its deadlines, and random event sequences checked against the rules that must always hold.
- `tests/pid1.rs`: 26 scenarios with real processes. Each starts the manager as PID 1 of a new PID namespace under `unshare`, with services played by `tests/support/misbehave.rs`: children that exit at once, crash, never become ready, stop answering while alive and answer again, ignore SIGTERM with their children, leave orphans, flood stdout and stderr, or read SIGHUP; TCP, HTTP and STUN readiness; clients that send nothing, send garbage, trickle a reply or are not root; SIGTERM during start-up, a backoff and a restart, and SIGINT; one scenario runs the manager as a subreaper instead of PID 1. Each checks the outcome, the manager's exit status and shutdown time, and that no zombie is left. It runs as root, refuses to run otherwise, and takes about two minutes; `cargo test --test pid1 -- NAME` runs some, and `PID1_VERBOSE=1` echoes the manager's log.

## Crate layout

- `src/core.rs`: the supervisor: every decision, on a clock it is given, through an `Io` trait it is given.
- `src/service.rs`: the description of a service and the checked table with its start order.
- `src/services.rs`: the container's services.
- `src/manager.rs`: the main loop: `poll` over a signalfd, the control socket and the readiness checks, `waitpid(-1)` on every round, and the `Io` that spawns and signals.
- `src/probe.rs`: nonblocking TCP, HTTP and STUN checks with deadlines.
- `src/control.rs`, `src/protocol.rs`, `src/client.rs`: the control socket, its wire format, and the subcommands' side.
- `src/log.rs`: the manager's log, written by its own thread.
- `src/accounts.rs`: users and groups from `/etc/passwd` and `/etc/group`.
- `src/sys.rs`: every `unsafe` block of the crate, each with the reason it is sound.
