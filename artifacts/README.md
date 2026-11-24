# Browser runtime artifacts (`artifacts/`)

Third-party WebAssembly modules, AudioWorklet processors, and translation models that browsers fetch by URL at run time (`AudioWorklet.addModule`, `WebAssembly.instantiate`, `importScripts`, and `fetch`) and therefore cannot pass through Vite's module bundler.

## Layout

| Path | Purpose |
| --- | --- |
| `manifest.json` | Single inventory of every runtime artifact: `id`, served `path` under `web/public/`, `source` (`file` or `recipe`), `sha256`, byte `size`, `upstream` provenance/verification notes, and SPDX `license`. |
| `assemble.mjs` | Validates every `manifest.json` entry (path containment, byte length, SHA-256 digest) and atomically copies changed files into `web/public/`. Invoked automatically by `predev`, `prebuild`, and `pretest` in `web/`. |
| `files/` | Verified prebuilt artifacts committed directly in Git: `audio/gtcrn.wasm` and `audio/gtcrn-worklet.js` (GTCRN neural noise suppressor) and `bergamot-models/{plen,enpl}/*` (Mozilla Firefox Remote Settings Polish–English neural translation models). |
| [`recipes/`](recipes/README.md) | Reproducible Docker BuildKit source recipes (`speex`, `rnnoise`, and `bergamot`) compiled from pinned upstream commits into `artifacts/recipes/<name>/out/`. |
| `NOTICES` | Third-party copyright notices, licenses, and linked `libc`/`libm` archive attributions for every artifact in `manifest.json`. |

## Workflow

Build source recipes once in a fresh checkout or worktree (and whenever a recipe under `recipes/` or its `Dockerfile` stage changes), then assemble `web/public/`:

```bash
docker buildx bake recipes
node artifacts/assemble.mjs
```

`assemble.mjs` never runs Docker itself; if a recipe output is missing or does not match the SHA-256 recorded in `manifest.json`, `npm run dev`, `npm test`, and `npm run build` fail immediately rather than serving stale or unverified files.

See [`recipes/README.md`](recipes/README.md) for cross-platform determinism rules and instructions on converting a prebuilt file into a source recipe.
