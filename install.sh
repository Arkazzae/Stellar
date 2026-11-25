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
# lists them. STELLARR_INSTALLER_BIN runs an executable already on the host.
# To run from source, use cargo run --locked --manifest-path installer/Cargo.toml.
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
      fail "There is no Stellarr installer or image for $arch, only for amd64 and arm64"
      ;;
  esac
  target=$cpu-$system
}

# Fetches $1 over HTTPS to $2 and writes the final URL to stdout.
download() {
  if have curl; then
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
      --connect-timeout 15 --max-time 300 --retry 3 --retry-delay 2 \
      -w '%{url_effective}\n' -o "$2" "$1"
  elif have wget; then
    # Follow redirects ourselves so wget cannot switch to an insecure URL.
    url=$1
    redirects=0
    while [ "$redirects" -le 10 ]; do
      case "$url" in https://*) ;; *) return 1 ;; esac
      if wget -S -T 20 --max-redirect=0 -O "$2" "$url" 2>"$work/response"; then
        printf '%s\n' "$url"
        return 0
      fi
      location=$(awk '
        $1 ~ /^HTTP\// { status = $2; target = "" }
        status ~ /^3[0-9][0-9]$/ && tolower($1) == "location:" { target = $2 }
        END { print target }
      ' "$work/response")
      case "$location" in
        https://*) url=$location ;;
        /*)
          case "$location" in //*) return 1 ;; esac
          origin=${url#https://}
          url=https://${origin%%/*}$location
          ;;
        *) return 1 ;;
      esac
      redirects=$((redirects + 1))
    done
    return 1
  else
    return 1
  fi
}

sha256() {
  if have sha256sum; then
    sha256sum <"$1" | cut -d ' ' -f 1
  elif have shasum; then
    shasum -a 256 <"$1" | cut -d ' ' -f 1
  elif have openssl; then
    openssl dgst -sha256 -r <"$1" | cut -d ' ' -f 1
  else
    return 1
  fi
}

release_failed() {
  fail "$1" "Check that this machine reaches github.com. Use curl or GNU wget for HTTPS downloads." '' \
    "If no release carries the installer yet, run from a Stellarr checkout with Rust's cargo:" \
    "  cargo run --locked --manifest-path installer/Cargo.toml -- --dir . --build"
}

# Fetches the installer for this machine into $work and checks it against
# the sums published with it.
fetch_installer() {
  installer_target
  if ! have curl && ! have wget; then
    fail "Neither curl nor wget is installed" "Install curl or GNU wget and run the installer again."
  fi
  if [ -n "${STELLARR_INSTALLER_VERSION:-}" ]; then
    tag=v${STELLARR_INSTALLER_VERSION#v}
  else
    release=$(download "$repo_url/releases/latest" /dev/null) ||
      release_failed "Could not find the latest Stellarr release"
    case "$release" in
      "$repo_url"/releases/tag/*) tag=${release##*/} ;;
      *) release_failed "Could not find the latest Stellarr release" ;;
    esac
  fi
  case "$tag" in
    v[0-9]*.[0-9]*.[0-9]*) ;;
    *) fail "Invalid Stellarr installer release tag: $tag" ;;
  esac
  case "$tag" in
    *[!A-Za-z0-9.-]*) fail "Invalid Stellarr installer release tag: $tag" ;;
  esac
  base="$repo_url/releases/download/$tag"
  name="stellarr-installer-$target"
  if ! download "$base/$name" "$work/$name" >/dev/null ||
    ! download "$base/stellarr-installer.sha256" "$work/sums" >/dev/null; then
    release_failed "Could not download the Stellarr installer for $target from $tag"
  fi
  expected=$(awk -v name="$name" '$2 == name || $2 == "*" name { print $1; exit }' "$work/sums")
  actual=$(sha256 "$work/$name") ||
    fail "Neither sha256sum, shasum nor openssl is installed to check the installer" \
      "Install one of them and run the installer again."
  if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    fail "The downloaded installer does not match its published SHA-256 sum" \
      "Nothing was run. Try again; if it persists, report it at $repo_url/issues"
  fi
  chmod 700 "$work/$name" || fail "Could not make the verified installer executable"
  installer="$work/$name"
}

# The signal traps call this while the shell is waiting for the installer.
# shellcheck disable=SC2329
cancel() {
  pid=${child:-$!}
  if [ -n "$pid" ]; then
    kill -s "$1" "$pid" 2>/dev/null || :
    wait "$pid" 2>/dev/null || :
  fi
  exit "$2"
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

  if [ -n "${STELLARR_INSTALLER_BIN:-}" ]; then
    STELLARR_INSTALLER_CHECKOUT=$checkout
    export STELLARR_INSTALLER_CHECKOUT
    exec "$STELLARR_INSTALLER_BIN" "$@"
  fi

  work=$(mktemp -d "${TMPDIR:-/tmp}/stellarr-installer.XXXXXX") ||
    fail "Could not make a temporary directory for the installer"
  child=''
  trap 'rm -rf -- "$work"' EXIT
  trap 'cancel INT 130' INT
  trap 'cancel TERM 143' TERM
  fetch_installer

  STELLARR_INSTALLER_CHECKOUT=$checkout "$installer" "$@" </dev/null &
  child=$!
  wait "$child"
  status=$?
  if [ "$status" -eq 126 ]; then
    fail "Could not run the installer from ${TMPDIR:-/tmp}, which may not allow programs" \
      "Set TMPDIR to another directory and run the installer again."
  fi
  exit "$status"
}

main "$@" </dev/null
