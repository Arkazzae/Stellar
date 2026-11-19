#!/bin/sh
# Installs what build.sh compiles the Bergamot engine with, as the first step
# of the recipe's stage in the root Dockerfile: Debian's build tools, and
# Emscripten 3.1.8 with the Node.js emsdk pairs with it in /opt/emsdk. It runs
# as root, with the network, and checks every download against the sha256
# below.
#
# The stage runs as linux/amd64 on every build machine. emsdk publishes each
# Emscripten release once per host, and for 3.1.8 the two are different
# builds of LLVM and Binaryen that give different WASM from the same sources:
# the amd64 archive holds clang 15.0.0 from llvm-project 80ec0ebf and wasm-opt
# version_106-10-g22d24fda9, the arm64 one llvm-project 3b983359 and
# version_106-26-ge6a62de43. The amd64 one is what Mozilla builds with, and
# the manifest records its bytes.
set -eu

# The Debian archive as of the snapshot the base image was built from, the one
# its /etc/apt/sources.list.d/debian.sources names.
debian_snapshot=20261104T212339Z

# Emscripten 3.1.8, the version mozilla/translations' build-wasm.py installs
# (its inference/3rd_party/emsdk is at 2346baa7, emsdk's 3.1.8 tag): emsdk's
# emscripten-releases build of it for amd64, which carries the npm packages and
# prebuilt system libraries that `emsdk install` would add, and Node.js 14.18.2,
# which emsdk pairs with it.
emscripten_url=https://storage.googleapis.com/webassembly/emscripten-releases-builds/linux/8c9e0a76ebed2c5e88a718d43e8b62452def3771/wasm-binaries.tbz2
emscripten_sha256=6b170777eb523e62972ad458e533b1853cd0c4e02f6f2cf4cd68e109499ccd9b
node_url=https://nodejs.org/dist/v14.18.2/node-v14.18.2-linux-x64.tar.gz
node_sha256=83fa18a0e3642235446b66653eb27c169224ae9c1a15a32d6c3d9ddefb154ed4

emsdk=/opt/emsdk

if [ "$(uname -m)" != x86_64 ]; then
  echo "toolchain.sh: runs as linux/amd64 only, emulated on other hosts" >&2
  exit 1
fi

download() { # url sha256 file
  curl --fail --silent --show-error --location --retry 5 --retry-all-errors --output "$3" "$1"
  echo "$2  $3" | sha256sum --check --quiet -
}

cat > /etc/apt/sources.list.d/debian.sources <<EOF
Types: deb
URIs: http://snapshot.debian.org/archive/debian/$debian_snapshot
Suites: bookworm bookworm-updates
Components: main
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg

Types: deb
URIs: http://snapshot.debian.org/archive/debian-security/$debian_snapshot
Suites: bookworm-security
Components: main
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg
EOF
# A snapshot's Release files have expired; their signatures are still checked.
apt-get -o Acquire::Check-Valid-Until=false -o Acquire::Retries=5 update
DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
  bzip2 ca-certificates cmake curl git make python3
rm -rf /var/lib/apt/lists/*

downloads=$(mktemp -d)
download "$emscripten_url" "$emscripten_sha256" "$downloads/emscripten.tbz2"
download "$node_url" "$node_sha256" "$downloads/node.tar.gz"
mkdir -p "$emsdk/upstream" "$emsdk/node"
tar -xjf "$downloads/emscripten.tbz2" -C "$emsdk/upstream" --strip-components=1
tar -xzf "$downloads/node.tar.gz" -C "$emsdk/node" --strip-components=1
rm -rf "$downloads"

# What `emsdk activate` writes.
cat > "$emsdk/.emscripten" <<EOF
NODE_JS = '$emsdk/node/bin/node'
LLVM_ROOT = '$emsdk/upstream/bin'
BINARYEN_ROOT = '$emsdk/upstream'
EOF
