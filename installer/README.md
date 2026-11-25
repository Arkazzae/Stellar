# Stellarr installer

`stellarr-installer` installs Stellarr with Docker Compose on a Linux or macOS host, and updates an installation it made. People never run it by name: the root [`install.sh`](../install.sh), served at `https://stellarr.online/install.sh`, is a short POSIX script that downloads the installer for the machine from the latest GitHub release, checks it against the release's `stellarr-installer.sha256`, and runs it with the same options.

```bash
curl -fsSL https://stellarr.online/install.sh | bash -s -- --help
```

What it does, in order, with nothing written until every check has passed:

1. Checks the machine (memory, disk), Docker and Compose 2.24 or newer; reaches the daemon directly or through `sudo` (`sudo -n` without a terminal), and offers to install Docker Engine on Linux.
2. Finds an existing installation from the `stellarr` container's Compose labels, `--dir`, the checkout, the current directory or `~/stellarr`, and refuses directories that hold another project's Compose files.
3. Picks the release (the latest from GitHub's `releases/latest` redirect, a pinned or moving tag in `.env`, or `--version`) and never goes back to an older one.
4. Downloads `docker-compose.yml`, `.env.example` and, with TURN, `docker-compose.turn.yml` for that release; settles the settings from options, `.env`, questions and defaults; checks that the ports are free.
5. Writes the files atomically, backs up what it replaces, and changes only the `.env` keys it is given, keeping every other line byte for byte.
6. Pulls (or with `--build`, builds) the image, starts the container, waits for `stellarr-init health` and prints the setup code.

## Layout

| File | What it holds |
| --- | --- |
| `src/main.rs` | Entry, exit statuses, the fault report |
| `src/cli.rs` | Options, their environment variables and validation |
| `src/flow.rs` | One run, step by step |
| `src/ui.rs` | Terminal and plain output, questions and the waiting line |
| `src/envfile.rs` | Reading and rewriting `.env` as Compose reads it |
| `src/checks.rs` | Ports, addresses, tags, refs and release order |
| `src/http.rs` | HTTPS only, with rustls and its own roots |
| `src/ports.rs` | Listening ports from `ss`, `lsof` or `netstat` |
| `src/sys.rs` | Commands, user, clock, memory, disk and signals |

Docker, Compose, `sudo`, `systemctl` and the port listers stay commands it runs, as a person would; everything else is native.

## Building and testing

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --features test-seams
bash ../infra/tests/install.test.sh
```

The crate builds on Unix only. [`infra/tests/install.test.sh`](../infra/tests/install.test.sh) builds it with the `test-seams` feature, which answers HTTPS from files and lets a case choose the OS, the user id and a clock that does not wait, then runs it and `install.sh` against stub commands. A release build never has that feature.

The release workflow builds static programs for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-apple-darwin` and `aarch64-apple-darwin` and attaches them, with their sums, when it creates the GitHub release.
