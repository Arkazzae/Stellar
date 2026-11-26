# syntax=docker/dockerfile:1.8
# Compiler stages for the browser package workflow. The pinned input archives
# contain the sources and weights mirrored in Stellarr's container registry.
ARG RECIPE_PLATFORM=$BUILDPLATFORM

FROM ghcr.io/arkazzae/stellarr:browser-build-inputs-1@sha256:f123fe57e0dcfc6fd7fd3f6f1a326bb04f3c138f882ceae63188bbd49e262f72 AS browser-inputs

# Export one archive for the GTCRN compiler or the model packer on the host.
FROM scratch AS recipe-input
ARG INPUT_ARCHIVE
COPY --from=browser-inputs /inputs/${INPUT_ARCHIVE} /

# SpeexDSP's preprocessor, the Speex noise suppressor (artifacts/recipes/speex/).
FROM --platform=$RECIPE_PLATFORM ghcr.io/webassembly/wasi-sdk:wasi-sdk-33@sha256:f4d3519a6d4b36e8fd541019e13cb8d232980d9586e2b185f872ab97ea15bcfd AS recipe-speex-build
COPY --from=browser-inputs /inputs/speexdsp-sources.tar.gz /src/
RUN --mount=type=bind,source=artifacts/npm/inputs.sha256,target=/checksums \
    cd /src && sha256sum --check --ignore-missing /checksums && tar -xzf speexdsp-sources.tar.gz
COPY artifacts/recipes/speex/ /src/recipe/
RUN --network=none sh /src/recipe/build.sh /src/speexdsp /out

FROM --platform=$RECIPE_PLATFORM scratch AS recipe-speex
COPY --from=recipe-speex-build /out/ /

# The Bergamot translation engine, Mozilla's Marian for the web, with its
# Emscripten glue (artifacts/recipes/bergamot/). Pinned to linux/amd64, not
# RECIPE_PLATFORM: emsdk's Emscripten 3.1.8 for amd64 is clang 15.0.0 from
# llvm-project 80ec0ebf with wasm-opt version_106-10-g22d24fda9, for arm64
# llvm-project 3b983359 with version_106-26-ge6a62de43, and the two give
# different WASM from the same sources. The amd64 one gives Mozilla's bytes.
# The source snapshot retains the Git revisions the engine reads at build time.
FROM --platform=linux/amd64 debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc AS recipe-bergamot-build
COPY artifacts/recipes/bergamot/toolchain.sh /src/recipe/
RUN sh /src/recipe/toolchain.sh
COPY --from=browser-inputs /inputs/bergamot-sources.tar.gz /src/
# Marian watches the reflog timestamp and reads the revision from Git HEAD.
# The source snapshot omits workstation reflogs.
RUN --mount=type=bind,source=artifacts/npm/inputs.sha256,target=/checksums \
    cd /src && sha256sum --check --ignore-missing /checksums \
    && mkdir -p /builds/worker/checkouts \
    && tar --no-same-owner --warning=no-unknown-keyword -xzf bergamot-sources.tar.gz --strip-components=1 -C /builds/worker/checkouts \
    && mkdir -p /builds/worker/checkouts/.git/logs \
    && touch /builds/worker/checkouts/.git/logs/HEAD
COPY artifacts/recipes/bergamot/build.sh /src/recipe/
RUN --network=none sh /src/recipe/build.sh /out

FROM --platform=linux/amd64 scratch AS recipe-bergamot
COPY --from=recipe-bergamot-build /out/ /

# Xiph's RNNoise with its model, the RNNoise noise suppressor
# (artifacts/recipes/rnnoise/). The model is not in the repository: the
# commit's model_version names the archive by its sha256, and its
# download_model.sh fetches it from media.xiph.org.
FROM --platform=$RECIPE_PLATFORM ghcr.io/webassembly/wasi-sdk:wasi-sdk-33@sha256:f4d3519a6d4b36e8fd541019e13cb8d232980d9586e2b185f872ab97ea15bcfd AS recipe-rnnoise-build
COPY --from=browser-inputs /inputs/rnnoise-sources.tar.gz /inputs/rnnoise-data.tar.gz /src/
RUN --mount=type=bind,source=artifacts/npm/inputs.sha256,target=/checksums \
    cd /src && sha256sum --check --ignore-missing /checksums && tar -xzf rnnoise-sources.tar.gz
COPY artifacts/recipes/rnnoise/ /src/recipe/
RUN --network=none sh /src/recipe/build.sh /src/rnnoise /src/rnnoise-data.tar.gz /out

FROM --platform=$RECIPE_PLATFORM scratch AS recipe-rnnoise
COPY --from=recipe-rnnoise-build /out/ /
