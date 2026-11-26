# syntax=docker/dockerfile:1.8
# Compiler stages for the browser package workflow. The pinned input archives
# contain the sources and weights mirrored under the Stellarr repository.
ARG RECIPE_PLATFORM=$BUILDPLATFORM

# SpeexDSP's preprocessor, the Speex noise suppressor (artifacts/recipes/speex/).
FROM --platform=$RECIPE_PLATFORM ghcr.io/webassembly/wasi-sdk:wasi-sdk-33@sha256:f4d3519a6d4b36e8fd541019e13cb8d232980d9586e2b185f872ab97ea15bcfd AS recipe-speex-build
ADD --checksum=sha256:ea864507b6fb3e12a7810861d278fe2f64ff9b2a1abc9d6969f9ff3721414b41 https://github.com/Arkazzae/Stellarr/releases/download/npm-sources-1/speexdsp-sources.tar.gz /src/source.tar.gz
RUN tar -xzf /src/source.tar.gz -C /src
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
# The toolchain and the sources are not ADDed here: toolchain.sh and
# sources.sh name each download and commit and check them, the first a
# Debian snapshot and two archives by sha256, the second the commits of
# mozilla/translations and the four submodules the build compiles, which keep
# their .git because the build reads their revision.
FROM --platform=linux/amd64 debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc AS recipe-bergamot-build
COPY artifacts/recipes/bergamot/toolchain.sh /src/recipe/
RUN sh /src/recipe/toolchain.sh
ADD --checksum=sha256:1cbb917909a1830bce3c3ee4b65c0dd4e17bf913d07c3428b57d0023e110f2b8 https://github.com/Arkazzae/Stellarr/releases/download/npm-sources-1/bergamot-sources.tar.gz /src/source.tar.gz
# Marian watches the reflog timestamp and reads the revision from Git HEAD.
# The source snapshot omits workstation reflogs.
RUN mkdir -p /builds/worker/checkouts \
    && tar --no-same-owner --warning=no-unknown-keyword -xzf /src/source.tar.gz --strip-components=1 -C /builds/worker/checkouts \
    && mkdir -p /builds/worker/checkouts/.git/logs \
    && touch /builds/worker/checkouts/.git/logs/HEAD
COPY artifacts/recipes/bergamot/build.sh /src/recipe/
RUN --network=none sh /src/recipe/build.sh /out

FROM --platform=linux/amd64 scratch AS recipe-bergamot
COPY --from=recipe-bergamot-build /out/ /

# Xiph's RNNoise with its model, the RNNoise noise suppressor
# (artifacts/recipes/rnnoise/). The model is not in the repository: the
# commit's model_version names the archive by its sha256, and its
# download_model.sh fetches it from media.xiph.org, which this pins instead.
FROM --platform=$RECIPE_PLATFORM ghcr.io/webassembly/wasi-sdk:wasi-sdk-33@sha256:f4d3519a6d4b36e8fd541019e13cb8d232980d9586e2b185f872ab97ea15bcfd AS recipe-rnnoise-build
ADD --checksum=sha256:ca20ae659b39a45fa886da23a7b01d1ad1d90db80903c1423c24c875eaeb7aa5 https://github.com/Arkazzae/Stellarr/releases/download/npm-sources-1/rnnoise-sources.tar.gz /src/source.tar.gz
RUN tar -xzf /src/source.tar.gz -C /src
ADD --checksum=sha256:0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37 \
    https://github.com/Arkazzae/Stellarr/releases/download/npm-sources-1/rnnoise-data.tar.gz \
    /src/rnnoise_data.tar.gz
COPY artifacts/recipes/rnnoise/ /src/recipe/
RUN --network=none sh /src/recipe/build.sh /src/rnnoise /src/rnnoise_data.tar.gz /out

FROM --platform=$RECIPE_PLATFORM scratch AS recipe-rnnoise
COPY --from=recipe-rnnoise-build /out/ /
