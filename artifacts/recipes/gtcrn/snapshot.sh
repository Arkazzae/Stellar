#!/usr/bin/env bash
# Recreate the source archive stored with the Stellarr npm build inputs.
set -euo pipefail
dest=$(cd "${1:?output directory}" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
cd "$tmp"

snapshot() {
  local repo=$1 rev=$2 dir=$3
  shift 3
  mkdir -p "$dir"
  git init -q source
  git -C source fetch -q --depth 1 --filter=blob:none "https://github.com/$repo.git" "$rev"
  git -C source archive "$rev" "$@" | tar -x -C "$dir"
  rm -rf source
}

snapshot sapphi-red/gtcrn-wasm 24662ff13f184b96edf91f1568e186341a8ac988 gtcrn build src LICENSE
base=gtcrn/build
snapshot Xiaobin-Rong/gtcrn 3862c44808dca492ea5a8a145d2dc2a1028d08c8 "$base/gtcrn" stream/onnx_models/gtcrn_simple.onnx LICENSE
snapshot marton78/pffft a4b03590cc2a4bea56f9721996e3057835799179 "$base/pffft"
converter="$base/onnx2c"
snapshot kraiskil/onnx2c bfd0753ac0ad32ce6ccd324add61ff5505a68c02 "$converter"
snapshot onnx/onnx 57b9c6a4f6eebb09ae1b34cfb632078518f72832 "$converter/onnx" onnx/onnx.proto LICENSE
snapshot badaix/aixlog 50fe38d7765ccef58483a3aba61fd5c435075d94 "$converter/aixlog"
snapshot Taywee/args 2ff29ca3af2f33c196ca4c7416dd51ff80e6b9bd "$converter/args"
snapshot kraiskil/cmake_timestamp cd58ac42dee3ec856df88462f1e0f29be9e6a6d6 "$converter/cmake_timestamp"
snapshot google/benchmark 77d1e74d29d4ee309b1c08eaea40ad789f08d125 "$converter/benchmark"
snapshot sapphi-red/web-noise-suppressor 1e168964f8e99835c43dd8fb81fde9f273e5699b gtcrn/worklet src LICENSE

# Git archives only contain tracked source files. Normalize host metadata.
python3 - "$dest/gtcrn-sources.tar.gz" <<'PY'
import gzip, os, pathlib, sys, tarfile
with open(sys.argv[1], "wb") as raw, gzip.GzipFile(fileobj=raw, filename="", mode="wb", mtime=0) as gz, tarfile.open(fileobj=gz, mode="w") as tar:
    for path in sorted(pathlib.Path("gtcrn").rglob("*")):
        info = tar.gettarinfo(str(path))
        info.uid = info.gid = info.mtime = 0
        info.uname = info.gname = ""
        info.mode = 0o755 if path.is_dir() or os.access(path, os.X_OK) else 0o644
        if info.isreg():
            with path.open("rb") as source:
                tar.addfile(info, source)
        else:
            tar.addfile(info)
PY
