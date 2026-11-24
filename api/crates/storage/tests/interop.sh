#!/bin/sh
# The folder store and the C# server it replaces must read each other's trees.
# This writes a tree with the C# folder store, writes the same objects with the
# Rust one, compares the two trees (names, kinds, sizes, permissions, bytes),
# has the Rust store read the C# tree, and has the C# store read the Rust tree.
#
#   CARGO=/path/to/cargo ./interop.sh PUBLISH_DIR
#
# PUBLISH_DIR is a published build of the C# API (the folder with
# Stellarr.Api.dll). It runs in a throwaway container of the image
# stellarr-api-dev:local (override with INTEROP_IMAGE), named
# stellarr-port-interop, which holds the .NET SDK. Nothing is built into an
# image and everything it makes is removed on exit.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
api=$(cd "$here/../../.." && pwd)
publish=${1:?the folder of the published C# API}
cargo=${CARGO:-cargo}
image=${INTEROP_IMAGE:-stellarr-api-dev:local}
name=stellarr-port-interop

work=$(mktemp -d "${TMPDIR:-/tmp}/stellarr-interop.XXXXXX")
trap 'docker rm -f "$name" >/dev/null 2>&1 || true; rm -rf "$work"' EXIT INT TERM
mkdir -p "$work/cs" "$work/rs" "$work/project"
cp "$here/interop/Interop.csproj" "$here/interop/Program.cs" "$work/project/"

csharp() { # csharp write|check DIR-under-/trees
  docker run --rm --name "$name" \
    -v "$publish":/app:ro -v "$work/project":/project -v "$work":/trees \
    -e DOTNET_CLI_TELEMETRY_OPTOUT=1 -e DOTNET_NOLOGO=1 \
    "$image" sh -c "cd /project && dotnet run --no-launch-profile -- $1 /trees/$2"
}

echo "interop: the C# store writes its tree"
csharp write cs/files
echo "interop: the Rust store writes its tree and reads the C# one"
cd "$api"
STELLARR_TEST_CS_TREE="$work/cs/files" STELLARR_TEST_RS_TREE="$work/rs/files" \
  "$cargo" test -p stellarr-storage --test interop -- --ignored --nocapture
echo "interop: the C# store reads the Rust tree"
csharp check rs/files
echo "interop: both directions agree"
