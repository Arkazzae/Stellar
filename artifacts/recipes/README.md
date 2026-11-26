# WebAssembly Build Recipes (`artifacts/recipes/`)

Reproducible Docker BuildKit recipes compiling Stellarr's browser-side audio DSP and offline translation engines from digest-pinned upstream sources (`ghcr.io/arkazzae/stellarr:browser-build-inputs-*`, tracked in `artifacts/npm/inputs.sha256` and `artifacts/npm/sources.json`).

| Recipe | Toolchain & upstream | Emitted files |
| --- | --- | --- |
| **Speex** (`speex/`) | `wasi-sdk` + SpeexDSP | `audio/speex.wasm` |
| **RNNoise** (`rnnoise/`) | `wasi-sdk` + Xiph RNNoise | `audio/rnnoise.wasm` |
| **GTCRN** (`gtcrn/`) | `onnx2c` + Emscripten `4.0.10` + GTCRN ONNX weights (verified against native inference) | `audio/gtcrn.wasm` + JS glue |
| **Bergamot** (`bergamot/`) | Emscripten `3.1.8` (`linux/amd64`, byte-identical to Firefox Remote Settings) + Bergamot `v0.6.0` | `codecs/bergamot/bergamot-translator.{wasm,js}` |

## Building locally

```bash
npm ci
docker buildx bake recipes   # writes outputs to artifacts/recipes/*/out/
```

`.github/workflows/npm-runtime.yml` pairs compiled outputs with `scripts/npm/fetch-input.sh` and `scripts/npm/pack-runtime.mjs` to bundle standalone AudioWorklets, generate `bergamot-models/registry.json`, and emit npm tarballs portable across `amd64` and `arm64` hosts. See [`artifacts/README.md`](../README.md) for the package release flow.

## Exporting or updating mirrored source inputs

```bash
# Export mirrored upstream tarballs and license notices to /tmp/stellarr-inputs/inputs/
docker buildx build -f artifacts/recipes/Dockerfile \
  --target browser-inputs --output type=local,dest=/tmp/stellarr-inputs .
```

To publish a new `browser-build-inputs-*` image after adding a model or source archive, verify `sha256sum -c SHA256SUMS` inside the inputs directory, build and push `artifacts/recipes/inputs.Dockerfile` for `linux/amd64,linux/arm64`, and update the pinned image digest and `artifacts/npm/inputs.sha256` together.
