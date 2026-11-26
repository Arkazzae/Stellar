# Building the browser packages

Stellarr mirrors the source snapshots, model weights, checksums and license notices in `ghcr.io/arkazzae/stellarr:browser-build-inputs-1`. The recipe Dockerfile pins the image by digest. `artifacts/npm/inputs.sha256` identifies the individual archives. `artifacts/npm/sources.json` records the upstream provenance, expected model and WASM bytes, and licenses. The generated npm manifest also lists the worklets and translation registry built alongside them.

| Recipe | Build | Output |
| --- | --- | --- |
| Speex | SpeexDSP with wasi-sdk | `audio/speex.wasm` |
| RNNoise | Xiph RNNoise and its weights with wasi-sdk | `audio/rnnoise.wasm` |
| GTCRN | GTCRN source and ONNX weights, onnx2c and Emscripten 4.0.10 | `audio/gtcrn.wasm` and JS glue |
| Bergamot | Bergamot v0.6.0 with Emscripten 3.1.8 on linux/amd64 | `codecs/bergamot/bergamot-translator.{wasm,js}` |

The [browser package workflow](../../.github/workflows/npm-runtime.yml) builds and packs release candidates. From the root, run `npm ci` and `docker buildx bake recipes` to build the Speex, RNNoise and Bergamot recipes locally. The bake file uses `artifacts/recipes/Dockerfile` and writes each recipe's outputs under its ignored `out/` directory. The GTCRN compiler runs the pinned source tree's native inference comparison before emitting the JS glue and SIMD WASM. `gtcrn/snapshot.sh` can reproduce its source archive.

After building the recipe outputs, use the workflow's `scripts/npm/fetch-input.sh` and `pack-runtime.mjs` steps to produce the tarballs. The packer links each audio worklet separately and emits a language registry from the translation model inventory. To add a direction, retain its source files in a versioned input archive and describe the model, lexicon and vocabulary in the inventory, then release a new translation package.

To export every original archive and its notices, run `docker buildx build -f artifacts/recipes/Dockerfile --target browser-inputs --output type=local,dest=/tmp/stellarr-inputs .` from the repository root. The files appear in `/tmp/stellarr-inputs/inputs/`. To publish revised inputs, place the archives, `SHA256SUMS` and `NOTICES` in a build directory. Verify them with `sha256sum -c SHA256SUMS` in that directory, build `artifacts/recipes/inputs.Dockerfile` with it as the context for `linux/amd64,linux/arm64`, and push a new numbered `browser-build-inputs-*` tag. Update the recipe's digest and `artifacts/npm/inputs.sha256` together.

Published files are portable across the image's amd64 and arm64 targets. The application build consumes the two locked npm tarballs. See the [application update procedure](../README.md).
