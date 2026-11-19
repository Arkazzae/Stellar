#!/bin/sh
# Builds the Bergamot translation engine, Mozilla's fork of Marian compiled to
# WebAssembly, from the sources sources.sh fetched, with the toolchain
# toolchain.sh installed, into the folder it is given:
#
#   bergamot-translator.js    the Emscripten glue, which defines loadBergamot()
#   bergamot-translator.wasm  the engine
#
# It is the last step of the recipe's stage in the root Dockerfile and runs
# without the network. Upstream builds with inference/scripts/build-wasm.py,
# which updates every submodule of the repository and installs Emscripten
# through emsdk; this takes the same steps on what the two steps before it
# pinned, and gives byte for byte the WASM Mozilla publishes.
#
#   sh build.sh <output folder>
set -eu

out=$1
checkout=/builds/worker/checkouts
emsdk=/opt/emsdk
build="$checkout/inference/build-wasm"

export EM_CONFIG="$emsdk/.emscripten"
export PATH="$emsdk/upstream/emscripten:$emsdk/node/bin:$PATH"
# Emscripten's system libraries and wasm-opt take their parallelism from
# these, not from make.
export EMCC_CORES=4 BINARYEN_CORES=4
export LC_ALL=C.UTF-8 TZ=UTC PYTHONHASHSEED=0
# Marian's CMakeLists runs `git submodule update --init` for every submodule of
# the repository. Refusing remote transports makes that fail at once instead of
# reaching for what the build never reads.
export GIT_ALLOW_PROTOCOL=file
umask 022

rm -rf "$build"
mkdir "$build"
cd "$build"
emcmake cmake -DCOMPILE_WASM=on -DWORMHOLE=off -DGIT_SUBMODULE=OFF "$checkout/inference"
emmake make -j4

# The two steps build-wasm.py takes after make, from the pinned tree: import
# the matrix multiplication from the host when it offers it, then wrap the
# glue in loadBergamot(Module).
bash "$checkout/inference/wasm/patch-artifacts-import-gemm-module.sh" "$build"
# The patch reports success even when its pattern matched nothing.
grep -q '"wasm_gemm": createWasmGemm(),' bergamot-translator.js || {
  echo "build.sh: the glue was not patched to import the matrix multiplication" >&2
  exit 1
}
python3 -I - "$checkout/inference/scripts" <<'EOF'
import importlib.util, sys
scripts = sys.argv[1]
sys.path.insert(0, scripts)
spec = importlib.util.spec_from_file_location("build_wasm", f"{scripts}/build-wasm.py")
build_wasm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build_wasm)
build_wasm.prepare_js_artifact()
EOF

# The WebAssembly features the engine is built with, by upstream's flags
# (-msimd128 among them) and Emscripten 3.1.8's libraries: the MVP with
# sign-extension operators, SIMD128, bulk memory and the atomic operations of
# the threads proposal, on unshared memory, since the engine starts no threads.
# They need Chrome 91, Firefox 89 and Safari 16.4, the releases that brought
# SIMD; the other three came earlier in each. The flags stay upstream's, which
# give Mozilla's bytes, so the set is checked here instead: wasm-opt refuses to
# validate a module that uses a feature outside it.
"$emsdk/upstream/bin/wasm-opt" bergamot-translator.wasm --mvp-features \
  --enable-sign-ext --enable-simd --enable-bulk-memory --enable-threads || {
  echo "build.sh: the WASM uses a WebAssembly feature outside its set" >&2
  exit 1
}

# Source paths reach the WASM by design, all below the checkout, which
# sources.sh puts where Mozilla's build had it; the toolchain's and temporary
# ones must not.
if grep -a -q -e "$emsdk" -e /tmp/ bergamot-translator.wasm bergamot-translator.js; then
  echo "build.sh: a path of the build machine reached the output" >&2
  exit 1
fi

mkdir -p "$out"
cp bergamot-translator.js bergamot-translator.wasm "$out/"
cd /
# Only the outputs belong in this step's layer, which a cache may copy.
rm -rf "$build"
sha256sum "$out"/*
