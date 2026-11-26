# Stellarr Installer (`init/installer/`)

`stellarr-installer` deploys and updates Docker Compose installations of Stellarr on Linux and macOS hosts. The root [`install.sh`](../../install.sh) (`https://stellarr.online/install.sh`) downloads the matching platform binary from `stellarr.online/server-downloads/`, verifies its SHA-256 against `stellarr-installer.sha256`, and forwards all arguments.

```bash
# Inspect CLI options via the hosted bootstrap script
curl -fsSL https://stellarr.online/install.sh | bash -s -- --help

# Build and run directly from local Rust sources + local checkout image
cargo run --locked --manifest-path init/installer/Cargo.toml -- --dir . --build
```

Use `STELLARR_INSTALLER_VERSION` to pin a specific installer binary version, `STELLARR_INSTALLER_BIN` to use a pre-existing local binary, and `--version TAG` to choose the target Stellarr server image tag.

## Execution sequence

1. **Host & Docker checks:** Verifies OS/arch and Docker Compose `2.24+` (using `sudo` on Linux when needed and offering Docker Engine installation).
2. **Locking & recovery:** Acquires file locks on the install directory and Docker daemon, recovering any interrupted prior file publication before reading `.env`.
3. **Configuration & ports:** Resolves target release (or `--build`), prepares staged Compose/env files, and checks host port availability while preserving ports already bound by an existing Stellarr container.
4. **Validation & pull:** Validates the staged Compose configuration and pulls/builds images (`--no-start` stops after file validation).
5. **Atomic publication:** Backs up replaced files, writes a recovery journal, swaps `docker-compose.yml` and `.env` atomically, and re-verifies resolved Compose config at the final paths.
6. **Activation & readiness:** Runs `docker compose up -d`, polls `stellarr-init health` up to `STELLARR_START_TIMEOUT` (`300 s` default), and prints the HTTPS URL and setup code.

## Source layout

| File | Responsibility |
| --- | --- |
| `src/main.rs`, `src/cli.rs` | Entrypoint, exit codes, fault reports, CLI flags, and `STELLARR_*` env mapping |
| `src/flow.rs`, `src/flow/deploy.rs` | End-to-end install/update orchestration, Compose staging, image pulls, and activation |
| `src/install_plan.rs`, `src/envfile.rs` | Directory/daemon locks, backup/journal recovery, and Compose-compatible `.env` rewriting |
| `src/checks.rs`, `src/ports.rs` | Domain/IP/port/tag validators and active listener detection (`ss`, `lsof`, `netstat`) |
| `src/http.rs`, `src/ui.rs`, `src/sys.rs` | `rustls` HTTPS client, interactive/TTY output, and Unix process/clock/disk primitives |

## Building, testing, and releases

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --features test-seams
bash ../../infra/tests/install.test.sh
```

- `test-seams` (never enabled in release builds) stubs HTTPS responses, OS identity, UID, and wall clocks for deterministic testing in `infra/tests/install.test.sh`.
- Pushing a `server-vX.Y.Z` tag builds static binaries for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`, publishing them with SHA-256 manifests as a versioned OCI bundle on GHCR served via `stellarr.online/server-downloads/`.
