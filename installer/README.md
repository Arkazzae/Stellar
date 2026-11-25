# Stellarr installer

`stellarr-installer` installs Stellarr with Docker Compose on a Linux or macOS host, and updates an installation it made. The root [`install.sh`](../install.sh), served at `https://stellarr.online/install.sh`, is a short POSIX script that downloads the installer for the machine from a single GitHub release, checks it against that release's `stellarr-installer.sha256`, and runs it with the same options.

```bash
curl -fsSL https://stellarr.online/install.sh | bash -s -- --help
```

The script resolves the latest stable release once. Set `STELLARR_INSTALLER_VERSION` to use another release's installer, or `STELLARR_INSTALLER_BIN` to use an existing executable. The installer option `--version TAG` selects the Stellarr container release.

To run the installer from source before release binaries are available, run this from the repository root with Rust and Cargo installed:

```bash
cargo run --locked --manifest-path installer/Cargo.toml -- --dir . --build
```

The Cargo command builds the installer. The installer flag `--build` builds the container image from the checkout.

Installation sequence

1. Checks the machine, Docker and Compose 2.24 or newer. On Linux it can use `sudo` to reach the daemon and offers to install Docker Engine if needed.
2. Finds the installation directory and locks it for the run. A run that starts a container also locks the Docker daemon against other installer runs. It recovers any previous interrupted publication before reading the settings.
3. Selects a release or the local build, prepares the Compose and environment files, and checks host ports. An existing Stellarr container can keep the bindings it already publishes.
4. Validates a staged Compose project and prepares its images for startup. With `--no-start`, it validates the files without pulling, building or starting images.
5. Backs up the files it replaces and publishes each file atomically. A journal lets the installer recover an interrupted publication. It checks that Compose resolves the same configuration at the final paths before committing the files.
6. Starts the container with the prepared images, waits for `stellarr-init health` within `STELLARR_START_TIMEOUT` (300 seconds by default), and displays the instance URL and setup code. If activation or startup fails after publication, the new files and backups are available for diagnosis and retry.

## Layout

| File | What it holds |
| --- | --- |
| `src/main.rs` | Entry, exit statuses, the fault report |
| `src/cli.rs` | Options, their environment variables and validation |
| `src/flow.rs` | Installer sequence, host checks and release selection |
| `src/flow/deploy.rs` | Compose staging and validation, image preparation, publication and activation |
| `src/ui.rs` | Terminal and plain output, questions and the waiting line |
| `src/install_plan.rs` | Installer locks, staged files, backups and publication recovery |
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

The release workflow builds installer binaries for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-apple-darwin` and `aarch64-apple-darwin` and attaches them, with their sums, when it creates the GitHub release.
