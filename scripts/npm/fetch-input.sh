#!/usr/bin/env bash
# Fetch an immutable Stellarr build input and verify it before extracting it.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
name=${1:?archive name}
output=${2:?destination directory}
expected=$(awk -v name="$name" '$2 == name {print $1}' "$root/artifacts/npm/inputs.sha256")
[[ $expected =~ ^[a-f0-9]{64}$ ]]
mkdir -p "$output"
file=$(mktemp)
trap 'rm -f -- "$file"' EXIT
curl --retry 3 --fail --location --silent --show-error "https://github.com/Arkazzae/Stellarr/releases/download/npm-sources-1/$name" -o "$file"
printf '%s  %s\n' "$expected" "$file" | shasum -a 256 --check
tar -xzf "$file" -C "$output"
