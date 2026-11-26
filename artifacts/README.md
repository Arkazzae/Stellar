# Browser Runtime Artifacts (`artifacts/`)

Manages the pinned same-origin WebAssembly modules, AudioWorklets, and offline translation models (`@stellarr/audio-filters` and `@stellarr/translation`) served by every Stellarr instance.

## How assembly works

1. `npm ci` at the repository root installs the exact locked versions of `@stellarr/audio-filters` and `@stellarr/translation` declared in `web/package.json`.
2. `web`'s `predev`, `prebuild`, and `pretest` hooks run `artifacts/assemble.mjs`, verifying every file's byte length and SHA-256 hash against `artifacts/manifest.json` before copying assets into `web/public/` (`/audio/*`, `/codecs/bergamot/*`, `/bergamot-models/*`).
3. Production container builds copy `artifacts/manifest.json` and `artifacts/NOTICES` to `/usr/share/doc/stellarr/artifacts/`.

## Releasing an updated runtime package

1. Update pinned inputs or source code and bump the package version under `artifacts/npm/{audio-filters,translation}/` (see [`recipes/README.md`](recipes/README.md)).
2. Verify the packed tarball locally, then push tag `npm-audio-filters-vX.Y.Z` or `npm-translation-vX.Y.Z` to trigger `.github/workflows/npm-runtime.yml` (publishes under npm dist-tag `next`).
3. Pin the exact version in `web/`:

   ```bash
   npm install --save-exact -w stellarr-web @stellarr/translation@X.Y.Z
   node scripts/npm/sync-manifest.mjs
   ```

4. Commit the updated `web/package.json`, `package-lock.json`, `artifacts/manifest.json`, and `artifacts/NOTICES`, then verify `npm run build:web` and `npm run test:translation-runtime -w stellarr-web`.
