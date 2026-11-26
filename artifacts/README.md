# Browser runtime files

Stellarr installs `@stellarr/audio-filters` and `@stellarr/translation` at the exact versions in `web/package.json` and the root lockfile. Run `npm ci` at the repository root for development. The web workspace's dev, build and test hooks run `assemble.mjs` to check the installed bytes against `manifest.json` and copy them to the paths the instance serves.

The package source manifests and metadata live in `npm/`. Each published archive contains its own SHA-256 inventory, attribution file and static files under `dist/`. The public URL paths are shared by the development server and the production image. The browser loads assets from its Stellarr instance.

## Updating a package

1. Change the package's source or pinned inputs and bump its own version under `artifacts/npm/`. See the [recipes](recipes/README.md).
2. Test the packed tarball, then tag the release commit with `npm-audio-filters-vX.Y.Z` or `npm-translation-vX.Y.Z`. The browser package workflow publishes the selected archive under the npm tag `next`.
3. Install the exact published version in the web workspace, for example `npm install --save-exact -w stellarr-web @stellarr/translation@X.Y.Z`. Run `node scripts/npm/sync-manifest.mjs` and include the manifest and lockfile in the application change.
4. Build and test the web client and image. The final image contains the inventory and notices under `/usr/share/doc/stellarr/artifacts/`.
