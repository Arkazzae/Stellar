#!/bin/sh
# Fetches the sources build.sh compiles into /builds/worker/checkouts, as the
# second step of the recipe's stage in the root Dockerfile. It needs the
# network, and takes each repository at the full hash of a commit, which names
# its content, rather than at a branch or a tag, which can move. The sources
# keep their .git: the build reads the commit of each to name its version.
#
# The path is the one Mozilla's build used. It reaches the WASM, through
# __FILE__ in Marian's error messages, and with it the bytes match Mozilla's.
set -eu

checkout=${1:-/builds/worker/checkouts}

# mozilla/translations at v0.6.0 (inference/BERGAMOT_VERSION), the revision of
# the bergamot-translator 3.0 that Firefox downloads from Remote Settings; built
# from it, the recipe gives that WASM byte for byte. The commits after it that
# change inference/ (up to eea6e5a8, 2026-01-26) fix bounds in the reader of
# text shortlists, which the browser never uses since it loads a binary one,
# and add a header comment to the glue: nothing the engine runs here changes,
# and the WASM would no longer be Mozilla's, differing in its embedded revision
# and in line numbers of error messages.
translations_url=https://github.com/mozilla/translations.git
translations_commit=1de4a085d3a7afb625c51a60aabb5ad298e4059f

# The submodules the WASM build compiles, at the commits their parent records:
# path, repository, commit. Eigen is a submodule of onnxjs. The others, the
# training tools, emsdk and the CPU-only libraries, are left out.
submodules="
inference/marian-fork/src/3rd_party/sentencepiece https://github.com/browsermt/sentencepiece.git ae41b7740d7006596bb9257e83340b2620db9d00
inference/marian-fork/src/3rd_party/intgemm https://github.com/kpu/intgemm.git f7401513da71758dacce52fed1c7855549abee59
inference/marian-fork/src/3rd_party/onnxjs https://github.com/browsermt/onnxjs.git 924924b08e9596b41aeebada4a172f026be95f5a
inference/marian-fork/src/3rd_party/onnxjs/deps/eigen https://github.com/abhi-agg/eigen-git-mirror.git fff37f4ca0397af9ed7e04f3bd6b893a1ea2b08e
"

mkdir -p "$checkout"
git init --quiet "$checkout"
git -C "$checkout" remote add origin "$translations_url"
# Only inference/ is built; the training pipeline and its data stay behind.
git -C "$checkout" fetch --quiet --depth 1 --filter=blob:none origin "$translations_commit"
git -C "$checkout" sparse-checkout set inference
git -C "$checkout" checkout --quiet --detach "$translations_commit"
# The version strings use short hashes, whose length otherwise depends on how
# many objects the clone holds. Nine is the length Mozilla's build shows.
git -C "$checkout" config core.abbrev 9

echo "$submodules" | while read -r path url commit; do
  [ -n "$path" ] || continue
  rmdir "$checkout/$path"
  git init --quiet "$checkout/$path"
  git -C "$checkout/$path" fetch --quiet --depth 1 "$url" "$commit"
  git -C "$checkout/$path" checkout --quiet --detach "$commit"
done
