# syntax=docker/dockerfile:1.8

# One image runs the whole instance: nginx, the API, optional coturn and the Stellarr
# Media server, supervised by Stellarr Init (init/), which runs as PID 1. nginx
# also serves the web client and the operator panel, Cockpit, under /cockpit,
# both built as static files. It builds for linux/amd64 and linux/arm64.
#
# The web build, whose output does not depend on the architecture, runs on the
# build machine's own platform, so a multi-platform build does not emulate it.
# The Rust stages (the API and the operator's tool, the media server and
# Stellarr Init) run there too and cross-compile for the target, because
# compiling Rust under emulation would not fit in the release job.

# The recipes of artifacts/recipes/ build some of the files the browser loads
# from source. Each has a stage, or a few, that fetch its pinned inputs and
# run its build file, and a FROM scratch stage, recipe-<name>, that holds only
# its outputs; web-build copies those into the recipe's out/ folder before the
# build checks them against artifacts/manifest.json. `docker buildx bake
# recipes` (docker-bake.hcl) builds the same stages into
# artifacts/recipes/<name>/out/ for development and for CI's recipes job,
# which hands them to the image job in place of these stages.
#
# A recipe's stages hold every pin of that recipe: a toolchain image or base
# image by the digest of its multi-platform index, each source by commit, and
# each other download by its sha256. Fetching is done in steps of its own;
# the compile runs with --network=none, so it can only use what was pinned.
# The manifest records one sha256 per output, so the bytes must not depend on
# the machine: by default the stages run on the build machine's platform,
# and RECIPE_PLATFORM moves them to another one to prove that, as
# `docker buildx bake recipes --set '*.args.RECIPE_PLATFORM=linux/amd64'`.
# A recipe whose toolchain is a different build on each host pins its stages
# to linux/amd64 instead, and says so beside them.
ARG RECIPE_PLATFORM=$BUILDPLATFORM
ARG VERSION=dev

# SpeexDSP's preprocessor, the Speex noise suppressor (artifacts/recipes/speex/).
FROM --platform=$RECIPE_PLATFORM ghcr.io/webassembly/wasi-sdk:wasi-sdk-33@sha256:f4d3519a6d4b36e8fd541019e13cb8d232980d9586e2b185f872ab97ea15bcfd AS recipe-speex-build
ADD https://github.com/xiph/speexdsp.git#7a158783df74efe7c2d1c6ee8363c1e695c71226 /src/speexdsp
COPY artifacts/recipes/speex/ /src/recipe/
RUN --network=none sh /src/recipe/build.sh /src/speexdsp /out

# On RECIPE_PLATFORM like the build, not on the target: web-build, which
# copies from it, then stays one stage for every platform of the image.
FROM --platform=$RECIPE_PLATFORM scratch AS recipe-speex
COPY --from=recipe-speex-build /out/ /

# The Bergamot translation engine, Mozilla's Marian for the web, with its
# Emscripten glue (artifacts/recipes/bergamot/). Pinned to linux/amd64, not
# RECIPE_PLATFORM: emsdk's Emscripten 3.1.8 for amd64 is clang 15.0.0 from
# llvm-project 80ec0ebf with wasm-opt version_106-10-g22d24fda9, for arm64
# llvm-project 3b983359 with version_106-26-ge6a62de43, and the two give
# different WASM from the same sources. The amd64 one gives Mozilla's bytes.
# The toolchain and the sources are not ADDed here: toolchain.sh and
# sources.sh name each download and commit and check them, the first a
# Debian snapshot and two archives by sha256, the second the commits of
# mozilla/translations and the four submodules the build compiles, which keep
# their .git because the build reads their revision.
FROM --platform=linux/amd64 debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc AS recipe-bergamot-build
COPY artifacts/recipes/bergamot/toolchain.sh /src/recipe/
RUN sh /src/recipe/toolchain.sh
COPY artifacts/recipes/bergamot/sources.sh /src/recipe/
RUN sh /src/recipe/sources.sh
COPY artifacts/recipes/bergamot/build.sh /src/recipe/
RUN --network=none sh /src/recipe/build.sh /out

FROM --platform=linux/amd64 scratch AS recipe-bergamot
COPY --from=recipe-bergamot-build /out/ /

# Xiph's RNNoise with its model, the RNNoise noise suppressor
# (artifacts/recipes/rnnoise/). The model is not in the repository: the
# commit's model_version names the archive by its sha256, and its
# download_model.sh fetches it from media.xiph.org, which this pins instead.
FROM --platform=$RECIPE_PLATFORM ghcr.io/webassembly/wasi-sdk:wasi-sdk-33@sha256:f4d3519a6d4b36e8fd541019e13cb8d232980d9586e2b185f872ab97ea15bcfd AS recipe-rnnoise-build
ADD https://github.com/xiph/rnnoise.git#70f1d256acd4b34a572f999a05c87bf00b67730d /src/rnnoise
ADD --checksum=sha256:0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37 \
    https://media.xiph.org/rnnoise/models/rnnoise_data-0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37.tar.gz \
    /src/rnnoise_data.tar.gz
COPY artifacts/recipes/rnnoise/ /src/recipe/
RUN --network=none sh /src/recipe/build.sh /src/rnnoise /src/rnnoise_data.tar.gz /out

FROM --platform=$RECIPE_PLATFORM scratch AS recipe-rnnoise
COPY --from=recipe-rnnoise-build /out/ /

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
# The prebuild hook copies the browser's binaries from artifacts/ into public/
# and fails the build on any that differ from artifacts/manifest.json: the
# committed ones, and those the recipes build, each into its out/ folder.
COPY artifacts/ artifacts/
COPY --from=recipe-speex / artifacts/recipes/speex/out/
COPY --from=recipe-bergamot / artifacts/recipes/bergamot/out/
COPY --from=recipe-rnnoise / artifacts/recipes/rnnoise/out/
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

# The API's workspace with every Rust file replaced by an empty stub, like the
# media crate's above. A change to one crate's source then leaves the
# third-party dependencies cached.
FROM --platform=$BUILDPLATFORM rust:1-slim-bookworm AS api-skeleton
WORKDIR /src/api
COPY api/ ./
RUN <<'EOF'
set -eu
mkdir -p /skeleton
cp Cargo.toml Cargo.lock /skeleton/
if [ -d .cargo ]; then cp -R .cargo /skeleton/; fi
find . -name Cargo.toml -type f | while read -r file; do
  mkdir -p "/skeleton/$(dirname "$file")"
  cp "$file" "/skeleton/$file"
done
find . -name '*.rs' -type f | while read -r file; do
  mkdir -p "/skeleton/$(dirname "$file")"
  echo 'pub fn main() {}' > "/skeleton/$file"
done
EOF

# The API, stellarr-api, and the operator's tool, stellarr, built from one
# workspace. The stub build compiles the dependencies; the crates of the
# workspace are then cleaned, as the media crate is, so that the real sources
# are built and no stub is shipped. The server reports the version it was
# built as, which it reads from STELLARR_VERSION at compile time.
FROM rust-cross AS api-build
ARG VERSION
ENV STELLARR_VERSION=${VERSION}
WORKDIR /src/api
COPY --from=api-skeleton /skeleton/ ./
RUN <<'EOF'
set -eu
target="$(cat /etc/rust-target)"
cargo build --release --locked --target "$target" -p stellarr-api -p stellarr-cli
for manifest in crates/*/Cargo.toml; do
  name="$(sed -n 's/^name = "\(.*\)"$/\1/p' "$manifest" | head -n 1)"
  cargo clean --release --locked --target "$target" --package "$name"
done
EOF
COPY api/ ./
RUN cargo build --release --locked --target "$(cat /etc/rust-target)" -p stellarr-api -p stellarr-cli \
    && install -D "target/$(cat /etc/rust-target)/release/stellarr-api" /out/api/stellarr-api \
    && install -D "target/$(cat /etc/rust-target)/release/stellarr" /out/cli/stellarr

# Stellarr Init, the process manager that runs as PID 1. It depends on libc
# alone, which compiles in seconds, so it needs no stage of stubs. The whole
# crate is copied, tests included: Cargo.toml names the test targets, and a
# missing one fails the manifest check even when only the binary is built.
FROM rust-cross AS init-build
WORKDIR /src/init
COPY init/ ./
RUN cargo build --release --locked --target "$(cat /etc/rust-target)" --bin stellarr-init \
    && install -D "target/$(cat /etc/rust-target)/release/stellarr-init" /out/stellarr-init

# The Rust toolchain and a file watcher, for the development stack's two Rust
# services. Each is a target of infra/compose/dev.yml that rebuilds and
# restarts its bind-mounted crate. The stack mounts its own volume over the
# registry, so the copy left by the install is dropped.
FROM rust:1-slim-bookworm AS rust-watch
RUN cargo install --locked watchexec-cli@2.8.0 \
    && rm -rf "$CARGO_HOME/registry"
WORKDIR /app

# Target of infra/compose/dev.yml: the media server.
FROM rust-watch AS media-dev

# Target of infra/compose/dev.yml: the API.
FROM rust-watch AS api-dev

# The same distribution as before, pinned by the digest of its multi-platform
# index, so nginx, coturn and openssl are the same packages. What the Rust
# programs need at run time is the C library, which the base has, the CA bundle
# (ca-certificates) for the API's calls to other hosts, and the zone database
# (tzdata) the server reads time zones from in /usr/share/zoneinfo. The
# operator's tool calls df and stty, which are coreutils'.
FROM ubuntu:26.04@sha256:d66b366dac61b575bee2aef07e9c26e8070096fbe84a3d4d3eb0da069b5d76db AS final

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
         ca-certificates coturn nginx openssl tzdata \
    && rm -rf /var/lib/apt/lists/*

COPY infra/nginx.conf /etc/nginx/nginx.conf.template
COPY infra/turnserver.conf.template /etc/turnserver.conf.template
COPY --from=init-build /out/stellarr-init /usr/local/bin/stellarr-init
# How the settings are read, which the entrypoint and the CLI's wrapper source.
COPY --chmod=0644 infra/settings.sh /usr/local/lib/stellarr/settings.sh
COPY --chmod=0755 infra/entrypoint.sh /usr/local/bin/stellarr-entrypoint
COPY --chmod=0755 infra/tls-maintenance.sh /usr/local/bin/stellarr-tls-maintenance
COPY --chmod=0755 infra/stellarr-cli.sh /usr/local/bin/stellarr

# The API and the operator's tool, one executable each.
COPY --from=api-build /out/api/stellarr-api /opt/stellarr/api/stellarr-api
COPY --from=api-build /out/cli/stellarr /opt/stellarr/cli/stellarr
# The two apps, which nginx serves: the web client's document at the top,
# Cockpit's in cockpit/, the build's own files in static/, and the folders of
# web/public/ beside them.
COPY --from=web-build /src/web/dist /opt/stellarr/web
COPY --from=media-build /out/stellarr-media /opt/stellarr/media/stellarr-media
# The origin, hash and licence of every prebuilt browser file travel with the
# image, next to the notices they refer to.
COPY artifacts/NOTICES artifacts/manifest.json /usr/share/doc/stellarr/artifacts/

ENV STELLARR_DATA_ROOT=/data/stellarr
# 49000 carries media over UDP and TCP on the same public port.
# The media server's control API (5200/tcp) listens on loopback and is not
# exposed.
EXPOSE 443/tcp 49000/udp 49000/tcp
VOLUME ["/data/stellarr"]

# Healthy means every critical service is ready, as stellarr-init reports it.
# The manager asks each one again every ten seconds and counts it as failing
# after three misses in a row, about 45 seconds: the API with its database on
# /health/ready, nginx for a page it serves, the media server's control API on
# /v1/health, and optional coturn for a STUN binding answer. A service that
# exits or cannot start fails at once. The manager already waits out a slow answer, so
# Docker needs fewer misses of its own than when it asked the services itself.
# The client answers within three seconds, or exits 1 when the manager does
# not.
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 \
  CMD ["/usr/local/bin/stellarr-init", "health"]

ARG VERSION
ARG REVISION=unknown
LABEL org.opencontainers.image.title="Stellarr" \
      org.opencontainers.image.description="Self-hosted text and voice chat for small groups" \
      org.opencontainers.image.source="https://github.com/Arkazzae/stellarr" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"

ENTRYPOINT ["/usr/local/bin/stellarr-entrypoint"]
