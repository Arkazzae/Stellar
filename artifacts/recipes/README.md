# Artifact recipes

A recipe builds one or more browser artifacts declared in [`../manifest.json`](../manifest.json) from pinned source instead of committing prebuilt binaries under `../files/` (see [`artifacts/README.md`](../README.md)). Remaining third-party artifacts without a recipe are committed under `../files/`.

| Recipe | Outputs | Upstream source & toolchain |
| --- | --- | --- |
| [`speex/`](speex/) | `speex.wasm`, served at `/audio/speex.wasm` | SpeexDSP preprocessor compiled with `wasi-sdk` |
| [`bergamot/`](bergamot/) | `bergamot-translator.wasm` and `bergamot-translator.js`, served under `/codecs/bergamot/` | Mozilla Bergamot translation engine `v0.6.0` compiled with Emscripten `3.1.8` on `linux/amd64` |
| [`rnnoise/`](rnnoise/) | `rnnoise.wasm`, served at `/audio/rnnoise.wasm` | Xiph RNNoise with embedded neural weights and WebAssembly SIMD matrix kernels, compiled with `wasi-sdk` |

## Building recipes

From the repository root (requires Docker with the Buildx plugin):

```sh
docker buildx bake recipes
```

This builds every recipe listed in [`../../docker-bake.hcl`](../../docker-bake.hcl) into `artifacts/recipes/<name>/out/` using the same `Dockerfile` stages as the production image build.

Run `docker buildx bake recipes` once in a fresh checkout or worktree, and whenever a recipe changes, before running `npm run dev`, `npm test`, or `npm run build` in `web/`. Those commands invoke [`../assemble.mjs`](../assemble.mjs) first, which copies outputs from `out/` into `web/public/`, verifies every SHA-256 digest against `artifacts/manifest.json`, and fails if any output is missing or mismatched (`assemble.mjs` never invokes Docker itself). The `web` service in `docker-compose.dev.yml` reads the assembled files from the host bind mount.

Cached builds finish in a few seconds. Cold builds download the pinned toolchains first (`wasi-sdk` is ~776 MB compressed; Bergamot's Emscripten archive is ~318 MB and runs under `linux/amd64` emulation on Apple silicon, taking 13–28 minutes on a cold cache).

## Recipe structure

Each recipe lives in `artifacts/recipes/<name>/` (lowercase letters, digits, and hyphens) and consists of:

1. **A build script** (such as `build.sh`) plus any local C/C++ glue or headers needed to compile the pinned upstream sources. It runs exclusively inside its `Dockerfile` stage, enforces the allowed WebAssembly target features (see [WebAssembly build guidelines](#webassembly-build-guidelines)), and verifies that the output imports only symbols the worklet provides.
2. **Generated outputs** written flat into `artifacts/recipes/<name>/out/` (gitignored and excluded via `.dockerignore`).

All upstream pins live in the root [`Dockerfile`](../../Dockerfile) stages (`recipe-<name>-build` and `recipe-<name>`):

- **Base and toolchain images**: pinned by tag and multi-platform manifest list digest so `amd64` and `arm64` hosts resolve the exact same image index.
- **Git repositories**: pinned by full commit hash with `ADD <repository>.git#<commit>` (GitHub-generated tarballs are not guaranteed to be byte-stable across time and are not used).
- **Tarballs and archives**: pinned by URL and SHA-256 digest via `ADD --checksum=sha256:<hash> <url>` or verified with `sha256sum` inside a fetch step. Debian packages are pinned via `snapshot.debian.org` timestamped repositories.

Fetching and compilation are separated into distinct `Dockerfile` steps. Network-dependent steps fetch and verify inputs first; compilation runs under `RUN --network=none` so the build cannot touch unpinned external resources. Each step cleans up temporary build trees in the same layer to keep BuildKit cache layers small.

## Reproducibility across platforms

Because `artifacts/manifest.json` records a single SHA-256 digest per output, a recipe must produce byte-identical binaries across runs and across host architectures. By default, recipe stages run on `$RECIPE_PLATFORM` (the host's native platform: `linux/arm64` on Apple silicon, `linux/amd64` on CI runners).

Verify determinism for any new or modified recipe before committing:

```sh
docker buildx build --no-cache --target recipe-<name> --output type=local,dest=/tmp/a .
docker buildx build --no-cache --target recipe-<name> --output type=local,dest=/tmp/b .
docker buildx bake recipes --set '*.args.RECIPE_PLATFORM=linux/amd64'
```

All three builds must match the SHA-256 digest in `artifacts/manifest.json`.

If `linux/arm64` and `linux/amd64` produce different bytes after host-path and timestamp leaks are removed, check whether the upstream toolchain ships different compiler revisions per host architecture (`clang --version`, `emcc -v`, `wasm-opt --version`, and bundled static libraries):

- **`wasi-sdk`** ships the exact same LLVM revision (`4434dabb`) and byte-identical target libraries on `arm64` and `amd64`, so `speex` and `rnnoise` run natively on `$RECIPE_PLATFORM`.
- **Emscripten (`emsdk`)** ships different host archives per architecture with different LLVM and `wasm-opt` builds. Recipes that use Emscripten (such as `bergamot`) pin `--platform=linux/amd64` on both `recipe-<name>-build` and `recipe-<name>` stages. On `amd64` CI runners this runs natively; on `arm64` development hosts it runs once under emulation and is cached by BuildKit.

Common sources of non-determinism and how recipes prevent them:

- **Build paths (`__FILE__`)**: Override fatal-error/assert hooks so source filenames are omitted, compile with `-ffile-prefix-map`, and fail the build if `strings` on the output binary contains a build path.
- **Timestamps and parallel object ordering**: Compile translation units in a fixed deterministic order and disable timestamp embedding.
- **Host `libm` constant folding**: Cross-check `arm64` and `amd64` builds to ensure compile-time floating-point folding does not vary by host.

## Converting an artifact to a recipe

1. Create `artifacts/recipes/<name>/` and add two stages to the root `Dockerfile`:
   - `FROM --platform=$RECIPE_PLATFORM <image>@<digest> AS recipe-<name>-build` (or `--platform=linux/amd64` if the toolchain differs by host) with fetch steps followed by a `RUN --network=none` compile step. Copy only `artifacts/recipes/<name>/` into the stage after toolchain and upstream source steps so edits to `build.sh` do not invalidate upstream downloads.
   - `FROM --platform=$RECIPE_PLATFORM scratch AS recipe-<name>` copying only the final output files. Keeping `--platform` on the `scratch` stage ensures multi-platform release builds do not duplicate `web-build` per target architecture.
2. In the `web-build` stage of `Dockerfile`, add `COPY --from=recipe-<name> / artifacts/recipes/<name>/out/` before `npm run build`.
3. Add `<name>` to `RECIPES` in `docker-bake.hcl`.
4. In `artifacts/manifest.json`, replace the artifact's `source` with `{ "recipe": "<name>", "output": "<filename>" }` and update `sha256`, `size`, `upstream` (repository, commit hash, and `verified` notes), and `license`. Keep `id` and `path` unchanged so client code needs no edits.
5. Document all compiled sources and linked `libc`/`libm` archive members (inspected with `-Wl,--why-extract=<file>`) in [`../NOTICES`](../NOTICES).
6. Remove the prebuilt binary from `../files/` and clean up any obsolete lines in `.gitignore` and `.dockerignore`.

`web/src/stellarr/lib/artifacts.test.ts` verifies that every recipe referenced in `artifacts/manifest.json` has a matching `Dockerfile` stage, `COPY --from=recipe-<name>` in `web-build`, and entry in `docker-bake.hcl`.

## CI caching and workflow

The `recipes` job in `.github/workflows/ci.yml` builds all recipes once per workflow run using a per-recipe GitHub Actions cache (`scope=recipe-<name>`, `mode=min`) and uploads the compiled binaries as the `recipe-outputs` workflow artifact:

- The `web` job downloads `recipe-outputs` into `artifacts/recipes/*/out/` before running `assemble.mjs`, tests, and Vite builds.
- The `image` job passes each `artifacts/recipes/<name>/out/` directory as a Buildx `--build-context recipe-<name>=...` override, replacing the `recipe-<name>` stage so the container image build never recompiles recipes.
- Using `mode=min` caches only the final `scratch` output layer (a few kilobytes for `speex`, ~5 MB for `bergamot`) rather than multi-hundred-megabyte intermediate toolchain layers, keeping usage well within GitHub's 10 GB repository cache limit.

## WebAssembly build guidelines

- **Standalone `wasm32-wasip1` modules without JS glue (where possible)**: For audio worklets (`speex`, `rnnoise`), compile with `wasi-sdk` `clang --target=wasm32-wasip1 -nostartfiles -Wl,--no-entry` and explicitly export only the functions called by the worklet plus `malloc` and `free`. Static `libm` and `dlmalloc` from `wasi-libc` link without WASI host imports; the build script checks that the final `.wasm` binary has zero imports so the worklet can instantiate it with `{}`. (If C++ static constructors are present, export and call `__wasm_call_ctors` once on initialization.)
- **Explicit WebAssembly feature pinning**: Compiler defaults advance over time. Pass `-mcpu=mvp` plus explicit `-m<feature>` flags to `clang`, and pass `-Xlinker --features=<comma-separated-list>` to `wasm-ld` (use `-Xlinker` because `-Wl,` splits on commas). `wasm-ld` rejects the link if any object (including `wasi-libc`, which in `wasi-sdk 33` targets `lime1` and requires `extended-const`) uses a feature outside the allowed list. Enable `simd128` only when the recipe's own kernels use WebAssembly SIMD (such as `rnnoise`).
- **Minimal manual configuration**: Rather than running Autotools inside the build stage, compile only the required source files with explicit `-D` defines matching upstream's `configure.ac`. Stub out unused symbols referenced by shared compilation units with `__builtin_trap()`.
- **Bit-exact or characterized audio verification**: Compare new recipe builds against previous binaries using `npm run bench:voice-suppressor -w stellarr-web -- --engine <engine> --before <revision>` (see [`web/bench/voice/README.md`](../../web/bench/voice/README.md)).
- **Worklet lifecycle**: `speex-worklet.js` compiles its small binary inside the worklet constructor; `rnnoise-worklet.js` receives a `WebAssembly.Module` compiled on the main thread (`noiseSuppression.ts`) and instantiates it in the constructor. Both process audio from the first render quantum, attach `this.port.onmessage` (so `"destroy"` messages start the MessagePort and tear down state), perform zero per-quantum heap allocations, and refresh typed-array views whenever linear memory grows.
- **Buildx local output scope**: `docker buildx bake` only writes to paths inside the repository unless `--allow=fs.write=<path>` is passed.
