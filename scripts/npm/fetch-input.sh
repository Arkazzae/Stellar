#!/usr/bin/env bash
# Fetch an immutable Stellarr build input and verify it before extracting it.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
name=${1:?archive name}
output=${2:?destination directory}
expected=$(awk -v name="$name" '$2 == name {print $1}' "$root/artifacts/npm/inputs.sha256")
[[ $expected =~ ^[a-f0-9]{64}$ ]]
mkdir -p "$output"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
docker buildx build --file "$root/artifacts/recipes/Dockerfile" --target recipe-input \
  --build-arg "INPUT_ARCHIVE=$name" --output "type=local,dest=$work" "$root"
(cd "$work" && printf '%s  %s\n' "$expected" "$name" | shasum -a 256 --check)
tar -xzf "$work/$name" -C "$output"
