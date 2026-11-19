#!/bin/sh
# Builds speex.wasm: SpeexDSP's preprocessor, and only that, as a WebAssembly
# module that needs no JavaScript of its own. It imports nothing and exports
# its memory, malloc and free, and the four speex_preprocess_* functions the
# worklet calls (web/src/stellarr/lib/voice/capture/speex/).
#
#   build.sh <speexdsp checkout> <output directory>
#
# The root Dockerfile runs it in the recipe's stage, which pins the toolchain
# image and the SpeexDSP commit. The toolchain is wasi-sdk: clang, wasm-ld and
# wasi-libc, whose malloc and libm are linked in statically.
#
# The configuration is the one SpeexDSP's configure picks by default, which is
# what the binary this replaces was built with: floating point, the smallft
# FFT, C99 variable-length arrays for scratch space, -O2 and nothing that
# relaxes float arithmetic. Only the files the preprocessor uses are compiled,
# instead of running autotools over the whole library.
set -eu

source=$(cd "$1" && pwd)
out=$2
recipe=$(cd "$(dirname "$0")" && pwd)
objects=$(mktemp -d)

compile() {
  # Relative names and the prefix maps keep the build's directories out of
  # anything the compiler records.
  /opt/wasi-sdk/bin/clang --target=wasm32-wasip1 -O2 \
    -DFLOATING_POINT -DUSE_SMALLFT -DVAR_ARRAYS -DEXPORT= -DOS_SUPPORT_CUSTOM \
    -I"$recipe/include" -I"$source/include" \
    -ffile-prefix-map="$source"=speexdsp -ffile-prefix-map="$recipe"=recipe \
    -c "$1" -o "$objects/$(basename "$1" .c).o"
}

compile "$source/libspeexdsp/preprocess.c"
compile "$source/libspeexdsp/filterbank.c"
compile "$source/libspeexdsp/fftwrap.c"
compile "$source/libspeexdsp/smallft.c"
compile "$recipe/no_echo.c"

mkdir -p "$out"
/opt/wasi-sdk/bin/clang --target=wasm32-wasip1 -O2 -nostartfiles \
  -Wl,--no-entry -Wl,--strip-all \
  -Wl,--export=malloc -Wl,--export=free \
  -Wl,--export=speex_preprocess_state_init \
  -Wl,--export=speex_preprocess_state_destroy \
  -Wl,--export=speex_preprocess_run \
  -Wl,--export=speex_preprocess_ctl \
  "$objects/preprocess.o" "$objects/filterbank.o" "$objects/fftwrap.o" \
  "$objects/smallft.o" "$objects/no_echo.o" \
  -o "$out/speex.wasm"
rm -rf "$objects"

# The worklet instantiates the module with an empty import object, so a
# single import (a WASI call pulled in through stdio, say) would fail every
# browser at run time; fail here instead.
headers=$(/opt/wasi-sdk/bin/llvm-objdump --section-headers "$out/speex.wasm")
if printf '%s\n' "$headers" | grep -q ' IMPORT '; then
  echo "speex.wasm imports something:" >&2
  /opt/wasi-sdk/bin/llvm-objdump --all-headers "$out/speex.wasm" >&2
  exit 1
fi
if /opt/wasi-sdk/bin/llvm-strings "$out/speex.wasm" | grep -q -e "$source" -e "$recipe" -e '/opt/'; then
  echo "speex.wasm names a path of the build machine." >&2
  exit 1
fi
