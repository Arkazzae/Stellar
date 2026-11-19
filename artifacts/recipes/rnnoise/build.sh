#!/bin/sh
# Builds rnnoise.wasm: Xiph's RNNoise with its model compiled in, as a
# WebAssembly module that needs no JavaScript of its own. It imports nothing
# and exports its memory, malloc and free, and the three rnnoise_* functions
# the worklet calls (web/src/stellarr/lib/voice/capture/rnnoise/).
#
#   build.sh <rnnoise checkout> <model archive> <output directory>
#
# The root Dockerfile runs it in the recipe's stage, which pins the toolchain
# image, the RNNoise commit and the model archive. The toolchain is wasi-sdk:
# clang, wasm-ld and wasi-libc, whose malloc and libm are linked in
# statically.
#
# The model is not in RNNoise's repository. The commit names it by the
# sha256 in model_version, and download_model.sh fetches
# rnnoise_data-<that hash>.tar.gz from media.xiph.org: the weights as C
# sources, a full model and a smaller "little" one. This build takes the
# full model's src/rnnoise_data.c and refuses an archive whose sha256 is not
# the one the commit names.
#
# The configuration is the one RNNoise's configure picks by default:
# DISABLE_DEBUG_FLOAT, so the weights are the int8 ones and the float copies
# kept for debugging are left out, no assertions, no x86 run-time CPU
# detection. Only the library's own files are compiled, not its examples or
# training tools. nnet_simd.c, ours, stands in for src/nnet_default.c, so
# that the matrix products run on WebAssembly SIMD; it gives the same bits
# as the plain C it replaces.
set -eu

source=$(cd "$1" && pwd)
archive=$2
out=$3
recipe=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)

expected=$(cat "$source/model_version")
actual=$(sha256sum "$archive" | cut -d' ' -f1)
if [ "$expected" != "$actual" ]; then
  echo "The model archive is $actual; the RNNoise commit names $expected." >&2
  exit 1
fi
mkdir -p "$work/model"
tar -xzf "$archive" -C "$work/model" src/rnnoise_data.c src/rnnoise_data.h
model=$work/model/src

# The WebAssembly features the build may use, named one by one on top of the
# MVP, as Speex's build file names them: so that a newer clang, whose
# defaults grow, cannot raise what a browser must support without this list
# changing. They are the nine Speex is built with (clang 22's default CPU,
# and extended-const, which wasi-libc is built with) and simd128, the
# fixed-width SIMD that nnet_simd.c is written for and the compiler may also
# use elsewhere. Fixed-width SIMD runs from Chrome 91, Firefox 89 and Safari
# 16.4, which the list's own floor, extended-const in Chrome 114, Firefox 112
# and Safari 17, already lies above. No relaxed SIMD and no threads.
#
# The link takes the same list as --features, and wasm-ld refuses it when an
# object, ours or one of wasi-libc's, was built with a feature outside it.
features="bulk-memory bulk-memory-opt call-indirect-overlong extended-const multivalue mutable-globals nontrapping-fptoint reference-types sign-ext simd128"
cpu="-mcpu=mvp"
for feature in $features; do cpu="$cpu -m$feature"; done

compile() {
  # Relative names and the prefix maps keep the build's directories out of
  # anything the compiler records. $cpu is a list of flags, split on purpose.
  # -Wno-#warnings: nnet.c warns that it is built without vectorization,
  # which is true of vec.h's kernels but not of the ones nnet_simd.c puts in
  # their place.
  /opt/wasi-sdk/bin/clang --target=wasm32-wasip1 -O2 $cpu -DDISABLE_DEBUG_FLOAT -Wno-#warnings \
    -I"$model" -I"$source/include" -I"$source/src" \
    -ffile-prefix-map="$source"=rnnoise -ffile-prefix-map="$recipe"=recipe -ffile-prefix-map="$work"=work \
    -c "$1" -o "$work/$(basename "$1" .c).o"
}

for file in denoise rnn pitch kiss_fft celt_lpc nnet parse_lpcnet_weights rnnoise_tables; do
  compile "$source/src/$file.c"
done
compile "$model/rnnoise_data.c"
compile "$recipe/nnet_simd.c"

# The stack comes first in memory, so that a call deeper than it traps on
# the module's first address instead of writing over the weights after it.
# Processing a frame takes 72 KB of it at most (the scratch arrays of the
# FFT and of the network's layers), and the default stack is 64 KB; 128 KB
# leaves room.
mkdir -p "$out"
/opt/wasi-sdk/bin/clang --target=wasm32-wasip1 -O2 $cpu -nostartfiles \
  -Xlinker --features="$(printf '%s' "$features" | tr ' ' ',')" \
  -Wl,--no-entry -Wl,--strip-all -Wl,--stack-first -Wl,-z,stack-size=131072 \
  -Wl,--export=malloc -Wl,--export=free \
  -Wl,--export=rnnoise_create \
  -Wl,--export=rnnoise_destroy \
  -Wl,--export=rnnoise_process_frame \
  "$work"/*.o \
  -o "$out/rnnoise.wasm"
rm -rf "$work"

# The worklet instantiates the module with an empty import object, so a
# single import (a WASI call pulled in through stdio, say) would fail every
# browser at run time; fail here instead.
headers=$(/opt/wasi-sdk/bin/llvm-objdump --section-headers "$out/rnnoise.wasm")
if printf '%s\n' "$headers" | grep -q ' IMPORT '; then
  echo "rnnoise.wasm imports something:" >&2
  /opt/wasi-sdk/bin/llvm-objdump --all-headers "$out/rnnoise.wasm" >&2
  exit 1
fi
if /opt/wasi-sdk/bin/llvm-strings "$out/rnnoise.wasm" | grep -q -e "$source" -e "$recipe" -e "$work" -e '/opt/'; then
  echo "rnnoise.wasm names a path of the build machine." >&2
  exit 1
fi
