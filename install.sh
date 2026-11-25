#!/bin/sh
# Installs Stellarr with Docker Compose, and updates an installation it made.
#
#   curl -fsSL https://stellarr.online/install.sh | bash
#   curl -fsSL https://stellarr.online/install.sh | bash -s -- --yes --domain chat.example.com
#   ./install.sh               in a checkout
#   ./install.sh --build       in a checkout, building the image from it
#
# The installer itself is a program, built from installer/ in the repository
# and attached to every release. This script only fetches the one for this
# machine, checks its SHA-256 sum and runs it with the same arguments; --help
# lists them. In a checkout with Rust's cargo it builds the installer from the
# checkout instead, and STELLARR_INSTALLER_BIN runs a program already here.
# STELLARR_INSTALLER_VERSION takes the installer of another release, such as
# 1.5.0, in place of the latest one.
#
# It is POSIX sh, for any sh as for bash.

# Everything is in functions, run by the last line: a download cut off before
# it runs nothing.

repo_url=https://github.com/Arkazzae/Stellarr

fail() {
  printf '[error] %s\n' "$1" >&2
  shift
  if [ "$#" -gt 0 ]; then
    printf '\n' >&2
    printf '%s\n' "$@" >&2
  fi
  exit 1
}

have() { command -v "$1" >/dev/null 2>&1; }

# The target the installer is built for on this machine.
installer_target() {
  os=$(uname -s 2>/dev/null) || os=unknown
  arch=$(uname -m 2>/dev/null) || arch=unknown
  case "$os" in
    Linux) system=unknown-linux-musl ;;
    Darwin) system=apple-darwin ;;
    MINGW* | MSYS* | CYGWIN*) fail "On Windows, run the installer inside WSL 2, with Docker Desktop" ;;
    *) fail "The installer supports Linux and macOS, and this machine runs $os" ;;
  esac
  case "$arch" in
    x86_64 | amd64) cpu=x86_64 ;;
    aarch64 | arm64) cpu=aarch64 ;;
    *)
      fail "There is no Stellarr installer or image for $arch, only for amd64 and arm64" \
        "In a checkout with Rust's cargo, ./install.sh --build builds both from it."
      ;;
  esac
  target=$cpu-$system
}

# Downloads $1 to $2 over HTTPS only, with curl or wget.
download() {
  if have curl; then
    curl --proto '=https' --tlsv1.2 -fsSL --connect-timeout 15 --max-time 300 \
      --retry 3 --retry-delay 2 -o "$2" "$1"
  elif have wget; then
    # wget follows a redirect to plain HTTP, and BusyBox's knows no option
    # against it, so only an https address is accepted at all.
    case "$1" in https://*) ;; *) return 1 ;; esac
    wget -q -T 20 -O "$2" "$1"
  else
    fail "Neither curl nor wget is installed" "Install one of them and run the installer again."
  fi
}

sha256() {
  if have sha256sum; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif have shasum; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  elif have openssl; then
    openssl dgst -sha256 -r "$1" | cut -d ' ' -f 1
  else
    return 1
  fi
}

# Fetches the installer for this machine into $work and checks it against
# the sums published with it.
fetch_installer() {
  installer_target
  if [ -n "${STELLARR_INSTALLER_VERSION:-}" ]; then
    base="$repo_url/releases/download/v${STELLARR_INSTALLER_VERSION#v}"
  else
    base="$repo_url/releases/latest/download"
  fi
  name="stellarr-installer-$target"
  if ! download "$base/$name" "$work/$name" || ! download "$base/stellarr-installer.sha256" "$work/sums"; then
    if [ -n "$checkout" ]; then
      fail "Could not download the Stellarr installer for $target" "$base/$name" '' \
        "Check that this machine reaches github.com. If Stellarr has no release with" \
        "an installer yet, install Rust's cargo (https://rustup.rs) and run" \
        "./install.sh again: it then builds the installer from this checkout."
    fi
    fail "Could not download the Stellarr installer for $target" "$base/$name" '' \
      "Check that this machine reaches github.com. If Stellarr has no release with" \
      "an installer yet, build it from the source instead, with Rust's cargo" \
      "(https://rustup.rs) installed:" \
      "  git clone $repo_url.git" "  cd Stellarr && ./install.sh --build"
  fi
  expected=$(awk -v name="$name" '$2 == name || $2 == "*" name { print $1; exit }' "$work/sums")
  actual=$(sha256 "$work/$name") ||
    fail "Neither sha256sum, shasum nor openssl is installed to check the installer" \
      "Install one of them and run the installer again."
  if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    fail "The downloaded installer does not match its published SHA-256 sum" \
      "Nothing was run. Try again; if it persists, report it at $repo_url/issues"
  fi
  chmod 755 "$work/$name"
  installer="$work/$name"
}

# Builds the installer from the checkout at $1 with cargo.
build_installer() {
  printf 'Building the installer from the checkout with cargo.\n' >&2
  cargo build --quiet --release --locked --manifest-path "$1/installer/Cargo.toml" \
    --target-dir "$1/installer/target" ||
    fail "cargo could not build the installer from the checkout" "Its output is above."
  installer="$1/installer/target/release/stellarr-installer"
}

main() {
  # The last line of this file starts main with its input from /dev/null. A
  # download cut off inside that line can still call main, with the pipe as
  # its input, and then it must run nothing. Without /proc, where /dev/stdin
  # leads nowhere, there is no telling, and main runs.
  if [ -e /dev/stdin ] && [ ! /dev/stdin -ef /dev/null ]; then
    fail "The installer was cut off while it downloaded; download it again"
  fi

  # A script run as a file in a checkout names it to the installer, which
  # then uses the checkout's files. One read from a pipe has no directory.
  checkout=''
  if [ -f "$0" ]; then
    case "$0" in */*) dir=${0%/*} ;; *) dir=. ;; esac
    dir=$(cd -P -- "$dir" 2>/dev/null && pwd -P) || dir=''
    if [ -n "$dir" ] && [ -f "$dir/Dockerfile" ] &&
      grep -Eq '^name:[[:space:]]*stellarr[[:space:]]*$' "$dir/docker-compose.yml" 2>/dev/null; then
      checkout=$dir
    fi
  fi

  work=''
  trap 'if [ -n "$work" ]; then rm -rf -- "$work"; fi' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  if [ -n "${STELLARR_INSTALLER_BIN:-}" ]; then
    installer=$STELLARR_INSTALLER_BIN
  elif [ -n "$checkout" ] && [ -f "$checkout/installer/Cargo.toml" ] && have cargo; then
    build_installer "$checkout"
  else
    work=$(mktemp -d "${TMPDIR:-/tmp}/stellarr-installer.XXXXXX") ||
      fail "Could not make a temporary directory for the installer"
    fetch_installer
  fi

  STELLARR_INSTALLER_CHECKOUT=$checkout "$installer" "$@"
  status=$?
  if [ "$status" -eq 126 ] && [ -n "$work" ]; then
    fail "Could not run the installer from ${TMPDIR:-/tmp}, which may not allow programs" \
      "Set TMPDIR to another directory and run the installer again."
  fi
  exit "$status"
}

main "$@" </dev/null
