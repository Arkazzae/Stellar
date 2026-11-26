# Building the browser packages

Stellarr retains the pinned source snapshots and model weights in its [build inputs release](https://github.com/Arkazzae/Stellarr/releases/tag/npm-sources-1). `artifacts/npm/inputs.sha256` identifies those archives. `artifacts/npm/sources.json` records the upstream provenance, expected model and WASM bytes, and licenses. The generated npm manifest also lists the worklets and translation registry built alongside them.

| Recipe | Build | Output |
| --- | --- | --- |
| Speex | SpeexDSP with wasi-sdk | `audio/speex.wasm` |
| RNNoise | Xiph RNNoise and its weights with wasi-sdk | `audio/rnnoise.wasm` |
| GTCRN | GTCRN source and ONNX weights, onnx2c and Emscripten 4.0.10 | `audio/gtcrn.wasm` and JS glue |
| Bergamot | Bergamot v0.6.0 with Emscripten 3.1.8 on linux/amd64 | `codecs/bergamot/bergamot-translator.{wasm,js}` |

The [browser package workflow](../../.github/workflows/npm-runtime.yml) builds and packs release candidates. From the root, run `npm ci` and `docker buildx bake recipes` to build the Speex, RNNoise and Bergamot recipes locally. The bake file uses `artifacts/recipes/Dockerfile` and writes each recipe's outputs under its ignored `out/` directory. The GTCRN compiler runs the pinned source tree's native inference comparison before emitting the JS glue and SIMD WASM. `gtcrn/snapshot.sh` can reproduce its source archive.

After building the recipe outputs, use the workflow's `scripts/npm/fetch-input.sh` and `pack-runtime.mjs` steps to produce the tarballs. The packer links each audio worklet separately and emits a language registry from the translation model inventory. To add a direction, retain its source files in a versioned input archive and describe the model, lexicon and vocabulary in the inventory, then release a new translation package.

Published files are portable across the image's amd64 and arm64 targets. The application build consumes the two locked npm tarballs. See the [application update procedure](../README.md).
