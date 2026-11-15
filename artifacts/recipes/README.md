# Recipes

A recipe builds one or more of the files in [`../manifest.json`](../manifest.json) from source, in place of a file committed under `../files/`. There are none yet: every artifact is still a committed copy of a third-party build.

## What a recipe provides

A recipe is a folder `artifacts/recipes/<name>/`, where `<name>` is lower case letters, digits and dashes. It holds:

1. **Pinned inputs.** Every source the build reads, at a fixed revision: a repository URL with a full commit hash, or an archive URL with its sha256, and the toolchain by an exact version or image digest. Nothing is taken from a branch or a "latest" tag.
2. **A build file** that turns those inputs into the output files and nothing else. A `build.sh` run inside a pinned container image is enough.
3. **A reproducible result.** The same inputs give the same bytes on every machine and every run, because the manifest records one sha256 for each output and the assemble step refuses anything else. Timestamps, paths and parallelism must not leak into the output.
4. **The output file names the manifest expects.** The build writes them, flat, into `artifacts/recipes/<name>/out/`. That folder is ignored by git.

## Switching an artifact to a recipe

1. Add two stages to the root `Dockerfile`: one, on `$BUILDPLATFORM`, that copies the recipe folder in and runs its build file, and a `FROM scratch` stage named `recipe-<name>` that holds only the outputs. In the `web-build` stage, `COPY --from=recipe-<name> / artifacts/recipes/<name>/out/` before `npm run build`, which runs the assemble step.
2. Change the artifact's `source` in the manifest to `{ "recipe": "<name>", "output": "<file name>" }`, and update its `sha256`, `size`, `upstream` and `license`. Its `id` and `path` stay, so the client does not change. Add the new upstream and license text to `../NOTICES` when they differ from the committed copy.
3. Delete the committed copy under `../files/`.
4. In development, build the same outputs with `docker build --target recipe-<name> --output type=local,dest=artifacts/recipes/<name>/out .` before `npm run dev`.
