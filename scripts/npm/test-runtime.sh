#!/usr/bin/env bash
# Install a release tarball into an empty consumer and exercise it in Chrome.
set -euo pipefail
name=${1:?package name}
archive=$(realpath "${2:?tarball}")
root=$(cd "$(dirname "$0")/../.." && pwd)
consumer=$(mktemp -d)
trap 'rm -rf -- "$consumer"' EXIT
printf '%s\n' '{"private":true}' > "$consumer/package.json"
npm install --prefix "$consumer" --offline --ignore-scripts --no-audit --no-fund "$archive"
dir="$consumer/node_modules/@stellarr/$name/dist"
if [[ $name == translation ]]; then
  STELLARR_TRANSLATION_ROOT="$dir" node "$root/web/scripts/verify-bergamot-runtime.mjs"
elif [[ $name == audio-filters ]]; then
  previous=$(cd "$root/web" && node -p 'require("node:path").dirname(require.resolve("@stellarr/audio-filters/package.json"))')
  for engine in speex rnnoise gtcrn; do
    STELLARR_AUDIO_ROOT="$dir/audio" node "$root/web/scripts/benchmark-voice-suppressor.mjs" --engine "$engine" \
      --before-worklet "$previous/dist/audio/$engine-worklet.js" --before-wasm "$previous/dist/audio/$engine.wasm"
  done
else
  exit 1
fi
