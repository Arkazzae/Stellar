# syntax=docker/dockerfile:1.8

# One image runs the whole instance: nginx, the API, coturn and the Stellarr
# Media server, supervised by Stellarr Init (init/), which runs as PID 1. nginx
# also serves the web client and the operator panel, Cockpit, under /cockpit,
# both built as static files. It builds for linux/amd64 and linux/arm64.
#
# Stages whose output is architecture-neutral (the web build and the portable
# API publish) run on the build machine's own platform, so a multi-platform
# build does not emulate them. The two Rust stages run there too and
# cross-compile for the target, because compiling Rust under emulation would
# not fit in the release job.

# web/ holds both apps, the web client and Cockpit (web/src/cockpit/). It is a
# workspace of one npm project with the shared packages/: the root
# package-lock.json covers all of them. The stage copies every workspace
# manifest, because npm ci checks the lockfile against all of them, but
# installs only web/, which links the shared packages it depends on. The
# manifests come before the sources, so that editing a source file keeps the
# install layer cached.
FROM --platform=$BUILDPLATFORM node:26-bookworm-slim AS web-build
WORKDIR /src
COPY package.json package-lock.json ./
COPY web/package.json web/
COPY packages/stellarr-icons/package.json packages/stellarr-icons/
COPY packages/stellarr-router/package.json packages/stellarr-router/
COPY packages/stellarr-ui/package.json packages/stellarr-ui/
RUN npm ci --workspace stellarr-web
COPY packages/ packages/
# The prebuild hook copies the browser's prebuilt binaries from artifacts/ into
# public/ and fails the build on any that differ from artifacts/manifest.json.
COPY artifacts/ artifacts/
COPY web/ web/
WORKDIR /src/web
# The build writes both apps to dist/. nginx sends the .gz file beside the
# original to a browser that accepts gzip, so the text formats and the WASM are
# compressed here once, at the highest level, instead of on every request. -n
# leaves the name and time out of each archive, so the same input gives the
# same file.
RUN npm run build \
    && find dist -type f \( -name '*.html' -o -name '*.js' -o -name '*.css' -o -name '*.json' \
         -o -name '*.svg' -o -name '*.wasm' \) -exec gzip -9 -k -n {} +

FROM --platform=$BUILDPLATFORM mcr.microsoft.com/dotnet/sdk:10.0-noble AS api-build
ARG VERSION=2.0.0
WORKDIR /src
COPY global.json Directory.Build.props Stellarr.slnx ./
COPY server/Stellarr.Api/Stellarr.Api.csproj server/Stellarr.Api/packages.lock.json server/Stellarr.Api/
COPY server/Stellarr.Cli/Stellarr.Cli.csproj server/Stellarr.Cli/packages.lock.json server/Stellarr.Cli/
RUN dotnet restore server/Stellarr.Cli/Stellarr.Cli.csproj --locked-mode \
    && dotnet restore server/Stellarr.Api/Stellarr.Api.csproj --locked-mode
COPY server/Stellarr.Api/ server/Stellarr.Api/
COPY server/Stellarr.Cli/ server/Stellarr.Cli/
RUN dotnet publish server/Stellarr.Api/Stellarr.Api.csproj \
    -c Release -o /out/api --no-restore -p:UseAppHost=false -p:Version=${VERSION}
RUN dotnet publish server/Stellarr.Cli/Stellarr.Cli.csproj \
    -c Release -o /out/cli --no-restore -p:UseAppHost=false -p:Version=${VERSION}
# The CLI references the API project, so its publish repeats the API's: the
# assemblies, appsettings.json, the native libraries. Only Stellarr.Cli.dll and
# its .pdb, .deps.json and .runtimeconfig.json are its own. The image ships one
# folder, the API's, with the CLI's own files added to it. Every file both
# publishes hold must be byte for byte the same, or the build stops here: the
# CLI must never run against an assembly it was not built with.
RUN <<'EOF'
set -eu
cd /out/cli
find . -type f | sort > /tmp/cli-files
differing=0
while IFS= read -r file; do
  if [ -e "/out/api/$file" ] && ! cmp -s "$file" "/out/api/$file"; then
    echo "The CLI and the API publish different copies of ${file#./}." >&2
    differing=1
  fi
done < /tmp/cli-files
if [ "$differing" -ne 0 ]; then
  exit 1
fi
while IFS= read -r file; do
  if [ ! -e "/out/api/$file" ]; then
    echo "Adding the CLI's ${file#./} to the API's folder."
    mkdir -p "/out/api/$(dirname "$file")"
    cp -p "$file" "/out/api/$file"
  fi
done < /tmp/cli-files
EOF

# The media crate with every Rust file replaced by an empty stub. It changes
# only when the manifest, the lockfile or the set of files changes, so the
# dependencies compiled from it stay cached across source edits. Stubbing every
# file, not only src/main.rs, keeps targets that Cargo.toml names explicitly
# (a [[test]] with harness = false, say) from failing the manifest check.
FROM --platform=$BUILDPLATFORM rust:1-slim-bookworm AS media-skeleton
WORKDIR /src/media
COPY media/ ./
RUN <<'EOF'
set -eu
mkdir -p /skeleton
cp Cargo.toml Cargo.lock /skeleton/
if [ -d .cargo ]; then cp -R .cargo /skeleton/; fi
find . -name '*.rs' -type f | while read -r file; do
  mkdir -p "/skeleton/$(dirname "$file")"
  echo 'pub fn main() {}' > "/skeleton/$file"
done
EOF

# The Rust toolchain on the build machine with what a cross build for the
# target needs: the Rust target, and the target's C compiler and C library,
# which link both binaries and which aws-lc-rs, the crypto backend of str0m,
# compiles with. Bookworm's glibc 2.36 is older than the 2.39 of the final
# image, so every symbol the binaries link against is there at run time.
FROM --platform=$BUILDPLATFORM rust:1-slim-bookworm AS rust-cross
ARG BUILDARCH
ARG TARGETARCH
# RUSTUP_TOOLCHAIN pins the image's own toolchain even if the crate later adds
# a rust-toolchain.toml: the cross target below is installed for this one only.
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION} \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar \
    AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
RUN <<'EOF'
set -eu
case "$TARGETARCH" in
  amd64) arch=x86_64 cross_gcc=gcc-x86-64-linux-gnu ;;
  arm64) arch=aarch64 cross_gcc=gcc-aarch64-linux-gnu ;;
  *) echo "Stellarr's Rust builds for amd64 and arm64, not $TARGETARCH." >&2; exit 1 ;;
esac
# A native build already has gcc and libc6-dev in this image.
if [ "$BUILDARCH" != "$TARGETARCH" ]; then
  apt-get update
  apt-get install -y --no-install-recommends "$cross_gcc" "libc6-dev-$TARGETARCH-cross"
  rm -rf /var/lib/apt/lists/*
fi
rustup target add "$arch-unknown-linux-gnu"
echo "$arch-unknown-linux-gnu" > /etc/rust-target
EOF

FROM rust-cross AS media-build
WORKDIR /src/media
COPY --from=media-skeleton /skeleton/ ./
# The stub's own artifacts are removed in the same layer. The real sources keep
# the modification times of the checkout, older than these artifacts, so cargo
# would otherwise take the stub binary for up to date and ship it.
RUN cargo build --release --locked --target "$(cat /etc/rust-target)" \
    && cargo clean --release --locked --target "$(cat /etc/rust-target)" --package stellarr-media
COPY media/ ./
RUN cargo build --release --locked --target "$(cat /etc/rust-target)" \
    && install -D "target/$(cat /etc/rust-target)/release/stellarr-media" /out/stellarr-media

# Stellarr Init, the process manager that runs as PID 1. It depends on libc
# alone, which compiles in seconds, so it needs no stage of stubs. The whole
# crate is copied, tests included: Cargo.toml names the test targets, and a
# missing one fails the manifest check even when only the binary is built.
FROM rust-cross AS init-build
WORKDIR /src/init
COPY init/ ./
RUN cargo build --release --locked --target "$(cat /etc/rust-target)" --bin stellarr-init \
    && install -D "target/$(cat /etc/rust-target)/release/stellarr-init" /out/stellarr-init

# Target of docker-compose.dev.yml: the SDK with the source bind-mounted.
FROM mcr.microsoft.com/dotnet/sdk:10.0-noble AS api-dev
WORKDIR /src/server/Stellarr.Api

# Target of docker-compose.dev.yml: the Rust toolchain and a file watcher that
# rebuilds and restarts the bind-mounted media crate. The dev stack mounts its
# own volume over the registry, so the copy left by the install is dropped.
FROM rust:1-slim-bookworm AS media-dev
RUN cargo install --locked watchexec-cli@2.8.0 \
    && rm -rf "$CARGO_HOME/registry"
WORKDIR /app

FROM mcr.microsoft.com/dotnet/aspnet:10.0-noble AS final

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates coturn nginx openssl \
    && rm -rf /var/lib/apt/lists/*

COPY infra/nginx.conf /etc/nginx/nginx.conf.template
COPY infra/turnserver.conf.template /etc/turnserver.conf.template
COPY --from=init-build /out/stellarr-init /usr/local/bin/stellarr-init
COPY --chmod=0755 infra/entrypoint.sh /usr/local/bin/stellarr-entrypoint
COPY --chmod=0755 infra/tls-maintenance.sh /usr/local/bin/stellarr-tls-maintenance
COPY --chmod=0755 infra/stellarr-cli.sh /usr/local/bin/stellarr

# The API, with the operator CLI's own files added in the api-build stage.
COPY --from=api-build /out/api /opt/stellarr/api
# The two apps, which nginx serves: the web client's document at the top,
# Cockpit's in cockpit/, the build's own files in static/, and the folders of
# web/public/ beside them.
COPY --from=web-build /src/web/dist /opt/stellarr/web
COPY --from=media-build /out/stellarr-media /opt/stellarr/media/stellarr-media
# The origin, hash and licence of every prebuilt browser file travel with the
# image, next to the notices they refer to.
COPY artifacts/NOTICES artifacts/manifest.json /usr/share/doc/stellarr/artifacts/

ENV STELLARR_DATA_ROOT=/data/stellarr
# 49000/udp is the single UDP port of Stellarr Media and carries every call.
# The media server's control API (5200/tcp) listens on loopback and is not
# exposed.
EXPOSE 80 443 3478/tcp 3478/udp 5349/tcp 5349/udp 49000/udp 49160-49200/udp
VOLUME ["/data/stellarr"]

# Healthy means every critical service is ready, as stellarr-init reports it.
# The manager asks each one again every ten seconds and counts it as failing
# after three misses in a row, about 45 seconds: the API with its database on
# /health/ready, nginx for a page it serves, the media server's control API on
# /v1/health, and coturn for a STUN binding answer. A service that exits, or
# cannot start, fails at once. The manager already waits out a slow answer, so
# Docker needs fewer misses of its own than when it asked the services itself.
# The client answers within three seconds, or exits 1 when the manager does
# not.
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 \
  CMD ["/usr/local/bin/stellarr-init", "health"]

ARG VERSION=dev
ARG REVISION=unknown
LABEL org.opencontainers.image.title="Stellarr" \
      org.opencontainers.image.description="Self-hosted text and voice chat for small groups" \
      org.opencontainers.image.source="https://github.com/Arkazzae/stellarr" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"

ENTRYPOINT ["/usr/local/bin/stellarr-entrypoint"]
