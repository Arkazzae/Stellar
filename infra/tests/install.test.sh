#!/usr/bin/env bash
# Runs the installer, init/installer/ built with its test-seams feature, and the
# install.sh that fetches it, against stub commands, so no Docker, network or
# root is needed. The stubs record every call and answer from files each case
# writes; the installer's HTTPS requests are answered from files as well.
# Each case checks what a person or the machine would notice: the files, the
# calls made to Docker, the exit status, a line on screen.
#
#   bash infra/tests/install.test.sh            every case
#   bash infra/tests/install.test.sh demo NAME  one screen on this terminal:
#                                               fresh, update, no-release or start-failed
#
# It builds the installer with cargo, unless STELLARR_INSTALLER_TEST_BIN names
# one built with --features test-seams already.
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
installer="$repo_root/install.sh"
binary=${STELLARR_INSTALLER_TEST_BIN:-}
if [[ -z $binary ]]; then
  cargo build --quiet --features test-seams --manifest-path "$repo_root/init/installer/Cargo.toml"
  binary="$(cargo metadata --format-version 1 --no-deps --manifest-path "$repo_root/init/installer/Cargo.toml" |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')/debug/stellarr-installer"
fi
[[ -x $binary ]] || { echo "No installer at $binary" >&2; exit 1; }
binary="$(cd -P "$(dirname "$binary")" && pwd -P)/$(basename "$binary")"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/stellarr-install-test.XXXXXX")"
# Without symbolic links, as the installer and the stubs see it.
scratch="$(cd -P "$scratch" && pwd -P)"
mkdir -p "$scratch/locks"
# KEEP=1 leaves the cases' directories, with their screens and calls, to look at.
if [[ -n ${KEEP:-} ]]; then
  trap 'echo "The cases are in $scratch/cases." >&2' EXIT
else
  trap 'rm -rf -- "$scratch"' EXIT
fi

fail() {
  local file
  echo "FAIL: $case_name: $*" >&2
  for file in out err calls; do
    if [[ -s $case_dir/$file ]]; then
      echo "--- $file" >&2
      cat "$case_dir/$file" >&2
    fi
  done
  exit 1
}

# The real tools the installer and the stubs run, and nothing else: a
# command missing here is missing for the installer too, so a stub that a
# case removes is never replaced by the machine's own docker or sudo.
tools="$scratch/tools"
mkdir -p "$tools"
for tool in awk cat chmod cmp cp cut date grep ln mkdir mktemp mv rm sed sha256sum sleep sort tail touch tr sh dash; do
  if path=$(command -v "$tool" 2>/dev/null) && [[ $path == /* ]]; then
    ln -s "$path" "$tools/$tool"
  fi
done
ln -s "$BASH" "$tools/bash"

# One program answers for every stubbed command, by the name it is called
# under. It records "name arguments @directory" in $STUB_LOG.
stub="$scratch/stub"
cat >"$stub" <<EOF
#!$BASH
EOF
cat >>"$stub" <<'EOF'
name=${0##*/}
state=$STUB_STATE
printf '%s %s @%s\n' "$name" "$*" "$PWD" >>"$STUB_LOG"

read_file() { if [[ -f $state/$1 ]]; then cat "$state/$1"; else printf '%s\n' "$2"; fi; }

env_value() {
  local key=$1 value=$2 line file=${compose_env:-.env} pattern
  pattern="^[[:blank:]]*(export[[:blank:]]+)?$key[[:blank:]]*[:=][[:blank:]]*(.*)$"
  if [[ -f $file ]]; then
    while IFS= read -r line || [[ -n $line ]]; do
      if [[ ${line%$'\r'} =~ $pattern ]]; then
        value=${BASH_REMATCH[2]}
        value=${value%% #*}
        value="${value%"${value##*[![:blank:]]}"}"
        if [[ $value == \'*\' || $value == \"*\" ]]; then
          value=${value:1:${#value}-2}
        fi
      fi
    done <"$file"
  fi
  printf '%s' "$value"
}

env_version() { env_value STELLARR_VERSION latest; }
env_image() { env_value STELLARR_IMAGE "ghcr.io/arkazzae/stellarr:$(env_version)"; }

render() {
  local out=$1 before rest expression after value key
  while [[ $out == *'{{'* ]]; do
    before=${out%%'{{'*} rest=${out#*'{{'}
    expression=${rest%%'}}'*} after=${rest#*'}}'}
    case $expression in
      .State.Running) value=$(cat "$state/container/running") ;;
      .State.Status) value=$(cat "$state/container/status") ;;
      .State.StartedAt) value=2026-11-20T18:30:12.000000000Z ;;
      .RestartCount) value=$(read_file container/restarts 0) ;;
      'index .Config.Labels "'*) key=${expression#*\"} key=${key%\"}
        value=$(read_file "container/label.$key" '<no value>') ;;
      *) value="<unknown $expression>" ;;
    esac
    out=$before$value$after
  done
  printf '%s\n' "$out"
}

listening() {
  local proto=$1 port
  [[ -f $state/busy-$proto ]] || return 0
  while IFS= read -r port; do
    case $name in
      ss) printf 'LISTEN 0 128 0.0.0.0:%s 0.0.0.0:*\n' "$port" ;;
      lsof) printf 'proc 1 ada 3u IPv4 0x1 0t0 %s *:%s\n' "$(printf '%s' "$proto" | tr a-z A-Z)" "$port" ;;
      netstat) printf '%s 0 0 0.0.0.0:%s 0.0.0.0:* LISTEN\n' "$proto" "$port" ;;
    esac
  done <"$state/busy-$proto"
}

case $name in
  uname)
    case $1 in -s) read_file os Linux ;; -m) read_file arch x86_64 ;; esac
    ;;
  id)
    case $1 in -u) read_file uid 1000 ;; *) echo ada ;; esac
    ;;
  df)
    printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\n'
    printf '/dev/sda1 100000000 50000000 43000000 54%% /\n'
    ;;
  sleep) ;;
  systemctl)
    [[ ! -e $state/systemctl-starts-docker ]] || rm -f "$state/docker-down"
    ;;
  sudo)
    while [[ $# -gt 0 && $1 == -* ]]; do shift; done
    [[ -e $state/sudo-works ]] || { echo "sudo: a password is required" >&2; exit 1; }
    [[ $# -gt 0 && $1 != true ]] || exit 0
    STUB_SUDO=1 exec "$@"
    ;;
  ss)
    echo 'State  Recv-Q Send-Q Local Address:Port Peer Address:Port'
    case $1 in -ltn) listening tcp ;; -lun) listening udp ;; esac
    ;;
  lsof)
    echo 'COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME'
    case $* in *-iTCP*) listening tcp ;; *) listening udp ;; esac
    ;;
  netstat)
    echo 'Proto Recv-Q Send-Q Local Address Foreign Address State'
    case $1 in -lnt) listening tcp ;; -lnu) listening udp ;; esac
    ;;
  curl | wget)
    url='' target=''
    while [[ $# -gt 0 ]]; do
      case $1 in
        -o | -O) target=$2; shift ;;
        --proto | --proto-redir | --connect-timeout | --max-time | --retry | --retry-delay | -T | -w) shift ;;
        https://* | http://*) url=$1 ;;
      esac
      shift
    done
    [[ ! -e $state/github-down ]] || { echo "$name: Could not resolve host: github.com" >&2; exit 6; }
    case $url in
      https://github.com/Arkazzae/Stellarr/releases/latest)
        if [[ $name == curl ]]; then
          read_file release-redirect https://github.com/Arkazzae/Stellarr/releases/tag/v1.4.0
        else
          echo '  HTTP/1.1 302 Found' >&2
          echo "  Location: $(read_file release-redirect /Arkazzae/Stellarr/releases/tag/v1.4.0)" >&2
          exit 8
        fi
        ;;
      https://github.com/Arkazzae/Stellarr/releases/tag/v1.4.0) ;;
      https://github.com/Arkazzae/Stellarr/releases/download/v*/*)
        file="$STUB_ASSETS/${url##*/}"
        if [[ ! -f $file ]]; then
          echo "$name: The requested URL returned error: 404" >&2
          exit 22
        fi
        cat "$file" >"$target"
        ;;
      *) echo "$name: no answer for $url" >&2; exit 6 ;;
    esac
    ;;
  docker-compose)
    case $* in
      *version*) read_file standalone-compose-version 1.29.2 ;;
      *) exec docker compose "$@" ;;
    esac
    ;;
  docker)
    command=$1
    shift
    case $command in
      version)
        [[ ! -e $state/docker-down ]] || { echo "Cannot connect to the Docker daemon. Is the docker daemon running?" >&2; exit 1; }
        if [[ -e $state/docker-needs-root && -z ${STUB_SUDO:-} ]]; then
          echo "permission denied while trying to connect to the Docker daemon socket" >&2
          exit 1
        fi
        echo 28.3.2
        ;;
      info) printf 'stub-daemon:%s\n' "$state" ;;
      compose)
        compose_env=.env
        compose_project="$PWD"
        compose_name=stellarr
        compose_files=()
        while [[ $1 == -* ]]; do
          case $1 in
            --project-name) compose_name=$2 ;;
            --project-directory) compose_project=$2 ;;
            --env-file) compose_env=$2 ;;
            -f) compose_files+=("$2") ;;
            *) echo "unknown Compose context option: $1" >&2; exit 2 ;;
          esac
          shift 2
        done
        action=$1
        shift
        if [[ $action != version ]]; then
          printf 'compose %s %s @%s\n' "$action" "$*" "$PWD" >>"$STUB_LOG"
        fi
        case $action in
          version)
            [[ ! -e $state/no-compose-plugin ]] || { echo "docker: 'compose' is not a docker command." >&2; exit 1; }
            read_file compose-version 2.39.1
            ;;
          config)
            if [[ -e $state/config-fails || ( -e $state/live-config-fails && $compose_project == "$PWD" ) ]]; then
              echo "invalid interpolation format" >&2
              exit 15
            fi
            if [[ -e $state/pause-live-config && $compose_project == "$PWD" && $* == '--format json' ]]; then
              touch "$state/live-config-paused"
              for ((ticks=0; ticks<250; ticks++)); do
                [[ -e $state/pause-live-config ]] || break
                "$STUB_REAL_SLEEP" .02
              done
            fi
            case $* in
              '--images stellarr') env_image ;;
              '--format json')
                build=''
                for file in "${compose_files[@]}"; do
                  if [[ $file == */build-context.json ]]; then
                    build=$(sed -n 's/.*"build":\({"context":"[^"]*"}\).*/,"build":\1/p' "$file")
                  fi
                done
                extras=$(read_file config-extra-services '')
                extras=${extras//PROJECT_DIR/$compose_project}
                printf '{"name":"%s","volumes":{"stellarr_data":{"name":"%s"}},"services":{"stellarr":{"container_name":"stellarr","image":"%s","volumes":[{"type":"volume","source":"stellarr_data","target":"/data/stellarr"}]%s}%s}}\n' \
                  "$compose_name" "$(read_file config-volume stellarr_stellarr_data)" "$(env_image)" "$build" "$extras"
                ;;
            esac
            ;;
          pull)
            [[ ! -e $state/pull-fails ]] || { echo "Error response from daemon: manifest unknown" >&2; exit 1; }
            if [[ $* == *registry* ]]; then
              touch "$state/image-remote"
            else
              touch "$state/image-$(env_version)"
            fi
            ;;
          build)
            for service in "$@"; do
              case $service in
                stellarr) touch "$state/image-$(env_version)" "$state/built" ;;
                helper) touch "$state/image-stellarr-helper" ;;
                *) exit 2 ;;
              esac
            done
            for file in "${compose_files[@]}"; do
              if [[ $file == */build-paths.json ]]; then
                cat "$file" >>"$STUB_LOG"
                printf '\n' >>"$STUB_LOG"
              fi
            done
            ;;
          up)
            [[ ! -e $state/up-fails ]] || { echo "container startup refused" >&2; exit 15; }
            printf 'compose up sees COMPOSE_PROJECT_NAME=%s STELLARR_TURN_RELAY_RANGE=%s COMPOSE_DISABLE_ENV_FILE=%s\n' \
              "${COMPOSE_PROJECT_NAME-unset}" "${STELLARR_TURN_RELAY_RANGE-unset}" "${COMPOSE_DISABLE_ENV_FILE-unset}" >>"$STUB_LOG"
            tag=$(env_version)
            [[ -e $state/image-$tag ]] || { echo "Error: No such image: ghcr.io/arkazzae/stellarr:$tag" >&2; exit 1; }
            mkdir -p "$state/container"
            read_file after-up running >"$state/container/status"
            echo true >"$state/container/running"
            echo stellarr >"$state/container/label.com.docker.compose.project"
            echo "$compose_project" >"$state/container/label.com.docker.compose.project.working_dir"
            if [[ -e $state/built ]]; then echo dev; else echo "$tag"; fi >"$state/container/label.org.opencontainers.image.version"
            ;;
        esac
        ;;
      container)
        [[ -d $state/container ]] || { echo "Error: No such container: stellarr" >&2; exit 1; }
        [[ $1 == inspect && $2 == --format ]] || exit 1
        if [[ $3 == *'.NetworkSettings.Ports'* ]]; then
          read_file port-bindings ''
        else
          render "$3"
        fi
        ;;
      image)
        [[ -e $state/image-${2##*:} ]]
        ;;
      exec)
        case $3 in
          health)
            count=$(($(read_file health-checks 0) + 1))
            echo "$count" >"$state/health-checks"
            hang_at=$(read_file hung-health 0)
            if [[ -e $state/hung-health ]] && ((count >= ${hang_at:-1})); then
              (
                trap '' INT TERM
                "$STUB_REAL_SLEEP" 3
                touch "$state/health-child-survived"
              ) &
              wait
            fi
            [[ ! -e $state/never-healthy ]] || exit 1
            [[ $count -ge $(read_file healthy-after 1) ]]
            ;;
          status) printf 'nginx    ready\napi      starting: waiting for the database\nmedia    ready\nturn     ready\n' ;;
        esac
        ;;
      logs)
        if [[ $1 == --tail ]]; then tail -n "$2" "$state/logs" 2>/dev/null || :; else cat "$state/logs" 2>/dev/null || :; fi
        ;;
    esac
    ;;
  # A command a case puts in place of a real tool fails, as if broken.
  *) exit 97 ;;
esac
EOF
chmod +x "$stub"

# The releases served by the stubbed GitHub. 1.5.0's docker-compose.yml
# differs from 1.4.0's by one comment.
fixtures="$scratch/fixtures"
mkdir -p "$fixtures"
# shellcheck disable=SC2016 # Compose interpolates the placeholders in the template.
sed -e '/STELLARR_TURN_ENABLED/d' -e '/49000\/tcp/d' \
  -e 's/127.0.0.1:8888/8888/g' -e 's/STELLARR_HTTPS_PORT:-443/STELLARR_HTTPS_PORT:-8443/g' \
  -e '/49000\/udp/a\
      - "${STELLARR_TURN_PORT:-3478}:3478/udp"\
      - "${STELLARR_TURN_RELAY_RANGE:-49160-49200}:${STELLARR_TURN_RELAY_RANGE:-49160-49200}/udp"
' "$repo_root/docker-compose.yml" >"$fixtures/compose-1.4.0"
{ cat "$fixtures/compose-1.4.0"; echo "# 1.5.0"; } >"$fixtures/compose-1.5.0"
cp "$repo_root/.env.example" "$fixtures/env.example"
# Without a checkout beside it, install.sh downloads the installer.
mkdir -p "$scratch/download"
cp "$installer" "$scratch/download/install.sh"
downloaded="$scratch/download/install.sh"
marker='# Written by the Stellarr installer, which changes only the settings it is given.'

# The installer as a release publishes it, for every target, and its sums.
assets="$scratch/assets"
mkdir -p "$assets"
for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-apple-darwin aarch64-apple-darwin; do
  cp "$binary" "$assets/stellarr-installer-$target"
done
(cd "$assets" && sha256sum stellarr-installer-* >stellarr-installer.sha256)

# Starts a case: a directory with its own bin, home, working directory and
# stub state, every stub in place, and release 1.4.0 published. The
# installer's HTTPS requests are answered from $http, by files named after
# their address.
new_case() {
  case_name=$1
  case_dir="$scratch/cases/$1"
  bin="$case_dir/bin" state="$case_dir/state" home="$case_dir/home" work="$case_dir/cwd"
  http="$case_dir/http" case_assets="$case_dir/assets"
  raw="$http/raw.githubusercontent.com/Arkazzae/Stellarr"
  mkdir -p "$bin" "$state" "$home" "$work" "$http/127.0.0.1/api/auth" "$http/get.docker.com" "$case_assets"
  : >"$case_dir/calls"
  local name
  for name in docker curl wget sudo uname ss lsof systemctl id df sleep; do
    ln -s "$stub" "$bin/$name"
  done
  ln -s "$assets"/* "$case_assets/"
  release 1.4.0
  echo "2026-11-20T18:30:13Z stellarr-api: Setup code: K7QM-2XWP-9HDA. Open https://chat.example.com:8443/setup to create the owner account." >"$state/logs"
  registration '{"setupRequired":true}'
  # Docker's install script, as the stubs have it: it logs and puts docker
  # in place.
  # shellcheck disable=SC2016 # The script expands these when it runs.
  printf '%s\n' 'echo "get-docker $*" >>"$STUB_LOG"' 'ln -s "$STUB_PROGRAM" "$STUB_BIN/docker"' \
    >"$http/get.docker.com/index"
  runner=("$binary")
  clock=fast
}

latest_release() {
  mkdir -p "$http/github.com/Arkazzae/Stellarr/releases"
  echo "$1" >"$http/github.com/Arkazzae/Stellarr/releases/latest.redirect"
}

release() {
  latest_release "https://github.com/Arkazzae/Stellarr/releases/tag/v$1"
  mkdir -p "$raw/v$1"
  cp "$fixtures/compose-$1" "$raw/v$1/docker-compose.yml"
  cp "$fixtures/env.example" "$raw/v$1/.env.example"
}

checkout_files() {
  local directory=$1
  mkdir -p "$directory/infra/compose"
  cp "$repo_root/docker-compose.yml" "$repo_root/.env.example" "$directory/"
  cp "$repo_root/infra/compose/build.yml" "$repo_root/infra/compose/turn.yml" "$directory/infra/compose/"
}

no_release() { latest_release https://github.com/Arkazzae/Stellarr/releases; }
github_down() {
  : >"$http/github.com.down"
  : >"$state/github-down"
}
registration() { echo "$1" >"$http/127.0.0.1/api/auth/registration"; }
flag() { : >"$state/$1"; }
answer() { printf '%s\n' "$@" >"$case_dir/answers"; }
# The case runs install.sh, which fetches the installer, in place of the
# installer itself.
through_install_sh() { runner=("$BASH" "${1:-$downloaded}"); }

# A container named stellarr that Compose started from $1, running or not,
# of release $2.
container() {
  mkdir -p "$state/container"
  echo "${3:-true}" >"$state/container/running"
  if [[ ${3:-true} == true ]]; then echo running; else echo exited; fi >"$state/container/status"
  if [[ -n $1 ]]; then
    echo stellarr >"$state/container/label.com.docker.compose.project"
    echo "$1" >"$state/container/label.com.docker.compose.project.working_dir"
  fi
  echo "$2" >"$state/container/label.org.opencontainers.image.version"
  touch "$state/image-$2"
}

# An installation the installer made earlier, of release $2, in $1.
installed() {
  mkdir -p "$1"
  cp "$fixtures/compose-$2" "$1/docker-compose.yml"
  cp "$fixtures/env.example" "$1/.env.example"
  printf '%s\n' "$marker" "STELLARR_DOMAIN=chat.example.com" "STELLARR_PUBLIC_IP=auto" "STELLARR_LANGUAGE=en" \
    "STELLARR_VERSION=$2" "STELLARR_HTTPS_PORT=8443" "STELLARR_HTTP_PORT=8888" \
    "STELLARR_MEDIA_PORT=49000" "STELLARR_TURN_PORT=3478" "STELLARR_TURNS_PORT=5349" "" \
    "# Kept by hand." "STELLARR_GIPHY_API_KEY=abc123" >"$1/.env"
  chmod 600 "$1/.env"
}

# Whether this machine has no controlling terminal: then the installer's own
# default, /dev/tty, can be left to fail as it would for a person.
no_controlling_terminal() { ! (: <>/dev/tty) 2>/dev/null; }

# The environment of every run: the stubs' state and the installer's test
# seams. The case's state may name another OS or user id for the installer.
seams() {
  # A temporary directory of the case's own, which must be empty again after
  # every run.
  mkdir -p "$case_dir/tmp"
  seam_env=(PATH="$bin:$tools" HOME="$home" TMPDIR="$case_dir/tmp" STUB_STATE="$state" STUB_LOG="$case_dir/calls"
    STUB_PROGRAM="$stub" STUB_BIN="$bin" STUB_ASSETS="$case_assets" STUB_REAL_SLEEP="$tools/sleep"
    STELLARR_INSTALLER_TEST_HTTP="$http" STELLARR_INSTALLER_TEST_HTTP_LOG="$case_dir/calls"
    STELLARR_INSTALLER_TEST_LOCKS="$scratch/locks")
  if [[ $clock == fast ]]; then seam_env+=(STELLARR_INSTALLER_TEST_CLOCK=fast); fi
  if [[ -f $state/os ]]; then seam_env+=(STELLARR_INSTALLER_TEST_OS="$(cat "$state/os")"); fi
  # A person, not root, unless the case says otherwise.
  seam_env+=(STELLARR_INSTALLER_TEST_UID="$(cat "$state/uid" 2>/dev/null || echo 1000)")
}

# Runs the installer in the case's working directory with a clean
# environment. VAR=value arguments come first, then the installer's options.
# Without answers there is no terminal to ask on.
run() {
  local env=() tty="$case_dir/no-terminal"
  [[ ! -f $case_dir/answers ]] || tty="$case_dir/answers"
  while [[ $# -gt 0 && $1 == *=* && $1 != -* ]]; do
    env+=("$1")
    shift
  done
  seams
  status=0
  (cd "$work" && env -i "${seam_env[@]}" TERM=dumb LANG=C STELLARR_INSTALLER_TTY="$tty" \
    STELLARR_INSTALLER_UI=plain ${env[@]+"${env[@]}"} "${runner[@]}" "$@" \
    </dev/null >"$case_dir/out" 2>"$case_dir/err") || status=$?
  expect_empty_dir "$case_dir/tmp"
}

expect_status() { [[ $status -eq $1 ]] || fail "exit status $status, not $1."; }
expect_success() { expect_status 0; }
expect_failure() { [[ $status -ne 0 ]] || fail "exit status 0, not a failure."; }
shown() { grep -qF -- "$1" "$case_dir/out" "$case_dir/err"; }
expect_shown() { shown "$1" || fail "the screen lacks: $1"; }
expect_not_shown() { ! shown "$1" || fail "the screen shows: $1"; }
called() { grep -qF -- "$1" "$case_dir/calls"; }
expect_called() { called "$1" || fail "no call matches: $1"; }
expect_not_called() { ! called "$1" || fail "a call matches: $1"; }
expect_no_calls() { [[ ! -s $case_dir/calls ]] || fail "commands were run."; }
expect_line() { grep -qxF -- "$2" "$1" || fail "$1 lacks the line: $2"; }
expect_missing() { [[ ! -e $1 ]] || fail "$1 exists."; }
expect_empty_dir() { [[ -z $(ls -A "$1") ]] || fail "$1 is not empty: $(ls -A "$1")"; }
count() { grep -cF -- "$1" "$2" || :; }
assignments() { grep -c "^$1=" "$2" || :; }
private_file() { [[ -n $(find "$1" -perm 0600) ]]; }

# Every listed file and its inode, so a file rewritten with the same content
# shows as a change too.
snapshot() { (cd "$1" && ls -ai && cat -- * .env); }

case_truncated() {
  new_case truncated
  local size cut cuts=() body="$case_dir/head.sh"
  size=$(wc -c <"$installer")
  size=$((size))
  for ((cut = 0; cut < size; cut += 100)); do cuts+=("$cut"); done
  # The last byte is the final newline; without it the script is whole.
  for ((cut = size - 60; cut < size - 1; cut++)); do cuts+=("$cut"); done
  seams
  for cut in "${cuts[@]}"; do
    # BSD head refuses a count of 0.
    if [[ $cut -eq 0 ]]; then : >"$body"; else head -c "$cut" "$installer" >"$body"; fi
    (cd "$work" && env -i "${seam_env[@]}" "$BASH" <"$body" >/dev/null 2>&1) || :
    [[ ! -s $case_dir/calls ]] || fail "the first $cut bytes ran: $(cat "$case_dir/calls")"
    expect_empty_dir "$home"
    expect_empty_dir "$work"
  done
}

# Runs command $@ the way run starts the installer, with output to the
# case's files and the status in $status.
hermetic() {
  seams
  status=0
  (cd "$work" && env -i "${seam_env[@]}" TERM=dumb STELLARR_INSTALLER_TTY="$case_dir/no-terminal" \
    STELLARR_INSTALLER_UI=plain "$@" </dev/null >"$case_dir/out" 2>"$case_dir/err") || status=$?
}

# install.sh is POSIX sh: any sh runs it, from a file or from its input.
case_posix_shells() {
  new_case posix-shells
  local shell
  for shell in sh dash; do
    [[ -e $tools/$shell ]] || continue
    hermetic "$shell" "$downloaded" --help
    [[ $status -eq 0 ]] || fail "$shell install.sh --help exited with $status."
    expect_shown "Usage:"
    expect_not_shown "syntax error"
    status=0
    (cd "$work" && env -i "${seam_env[@]}" "$shell" -s -- --help <"$downloaded" \
      >"$case_dir/out" 2>"$case_dir/err") || status=$?
    [[ $status -eq 0 ]] || fail "$shell -s <install.sh --help exited with $status."
    expect_shown "Usage:"
  done
  expect_not_called "docker"
  hermetic "$BASH" --posix "$downloaded" --yes
  expect_success
  expect_shown "Stellarr 1.4.0 is running"
}

# install.sh fetches the installer for this machine from the latest release,
# checks its sum and runs it with the same arguments.
case_install_sh() {
  new_case install-sh
  through_install_sh
  run --yes --domain chat.example.com
  expect_success
  expect_called "curl --proto =https --proto-redir =https --tlsv1.2 -fsSL"
  expect_called "https://github.com/Arkazzae/Stellarr/releases/download/v1.4.0/stellarr-installer-x86_64-unknown-linux-musl"
  expect_called "https://github.com/Arkazzae/Stellarr/releases/download/v1.4.0/stellarr-installer.sha256"
  expect_line "$home/stellarr/.env" "STELLARR_DOMAIN=chat.example.com"
  expect_shown "Stellarr 1.4.0 is running"

  new_case install-sh-macos
  through_install_sh
  echo Darwin >"$state/os"
  echo arm64 >"$state/arch"
  run --help
  expect_success
  expect_called "stellarr-installer-aarch64-apple-darwin"

  new_case install-sh-version
  through_install_sh
  run STELLARR_INSTALLER_VERSION=1.5.0 --help
  expect_success
  expect_called "https://github.com/Arkazzae/Stellarr/releases/download/v1.5.0/stellarr-installer-x86_64-unknown-linux-musl"

  new_case install-sh-wget
  through_install_sh
  rm "$bin/curl"
  run --yes
  expect_success
  expect_called "wget -S -T 20 --max-redirect=0 -O"

  new_case install-sh-wget-downgrade
  through_install_sh
  rm "$bin/curl"
  echo 'http://example.invalid/release' >"$state/release-redirect"
  run --help
  expect_failure
  expect_shown "Could not find the latest Stellarr release"
  expect_not_called "http://example.invalid/release"

  new_case install-sh-literal-backslash
  through_install_sh
  local literal_tmp="$case_dir/back\slash"
  mkdir "$literal_tmp"
  run TMPDIR="$literal_tmp" --help
  expect_success
  expect_empty_dir "$literal_tmp"

  new_case install-sh-local-binary
  through_install_sh
  run STELLARR_INSTALLER_BIN="$binary" STELLARR_INSTALLER_VERSION=missing --help
  expect_success
  expect_not_called curl

  new_case install-sh-bad-sum
  through_install_sh
  rm "$case_assets/stellarr-installer.sha256"
  echo "0000  stellarr-installer-x86_64-unknown-linux-musl" >"$case_assets/stellarr-installer.sha256"
  run --yes
  expect_failure
  expect_shown "does not match its published SHA-256 sum"
  expect_not_called docker
  expect_missing "$home/stellarr"

  new_case install-sh-before-first-release
  through_install_sh
  echo https://github.com/Arkazzae/Stellarr/releases >"$state/release-redirect"
  run --help
  expect_failure
  expect_shown "Could not find the latest Stellarr release"
  expect_shown "cargo run --locked --manifest-path init/installer/Cargo.toml -- --dir . --build"
  expect_not_called "releases/download/"
  expect_not_called docker

  new_case install-sh-no-installer
  through_install_sh
  rm "$case_assets/stellarr-installer-x86_64-unknown-linux-musl"
  run --yes
  expect_failure
  expect_shown "Could not download the Stellarr installer for x86_64-unknown-linux-musl"
  expect_shown "cargo run --locked --manifest-path init/installer/Cargo.toml -- --dir . --build"
  expect_empty_dir "$case_dir/tmp"

  # A checkout still gets the release binary; cargo is an explicit alternative.
  new_case install-sh-checkout
  local checkout="$case_dir/Stellarr"
  mkdir -p "$checkout/init/installer"
  checkout_files "$checkout"
  cp "$installer" "$checkout/"
  echo 'FROM scratch' >"$checkout/Dockerfile"
  : >"$checkout/init/installer/Cargo.toml"
  rm "$case_assets/stellarr-installer-x86_64-unknown-linux-musl"
  through_install_sh "$checkout/install.sh"
  run --yes --build
  expect_failure
  expect_shown "cargo run --locked --manifest-path init/installer/Cargo.toml -- --dir . --build"
  expect_not_called cargo

  new_case install-sh-arch
  through_install_sh
  echo armv7l >"$state/arch"
  run --yes
  expect_failure
  expect_shown "There is no Stellarr installer or image for armv7l"
  expect_not_called curl

  new_case install-sh-windows
  through_install_sh
  echo MINGW64_NT-10.0 >"$state/os"
  run --yes
  expect_failure
  expect_shown "On Windows, run the installer inside WSL 2"

  # Exit statuses pass through, 2 for a mistyped option.
  new_case install-sh-status
  through_install_sh
  run --frobnicate
  expect_status 2
  expect_shown "Unknown option --frobnicate"
}

case_fresh_install() {
  new_case fresh-install
  run --yes --domain chat.example.com --public-ip auto --language pl
  expect_success
  expect_called "https GET https://github.com/Arkazzae/Stellarr/releases/latest"
  expect_called "https GET https://raw.githubusercontent.com/Arkazzae/Stellarr/v1.4.0/docker-compose.yml"
  expect_called "https GET https://raw.githubusercontent.com/Arkazzae/Stellarr/v1.4.0/.env.example"
  cmp -s "$fixtures/compose-1.4.0" "$home/stellarr/docker-compose.yml" || fail "docker-compose.yml is not 1.4.0's."
  cmp -s "$fixtures/env.example" "$home/stellarr/.env.example" || fail ".env.example is not 1.4.0's."
  local env_file="$home/stellarr/.env"
  expect_line "$env_file" "$marker"
  expect_line "$env_file" "STELLARR_VERSION=1.4.0"
  expect_line "$env_file" "STELLARR_DOMAIN=chat.example.com"
  expect_line "$env_file" "STELLARR_PUBLIC_IP=auto"
  expect_line "$env_file" "STELLARR_LANGUAGE=pl"
  [[ $(head -n 1 "$env_file") == "$marker" ]] || fail "the marker is not the first line of .env."
  private_file "$env_file" || fail ".env is not readable and writable by its owner alone."
  expect_called "compose pull --quiet --policy always stellarr @$home/stellarr"
  expect_called "compose up -d --no-build --pull never @$home/stellarr"
  expect_not_called "sudo"
  expect_shown "Open: https://chat.example.com:8443/setup"
  expect_shown "Setup code: K7QM-2XWP-9HDA"
  expect_shown "Before you invite anyone"
  expect_shown "Web and API clients use HTTPS on 8443/TCP."
  expect_shown "forward 49000/UDP"
  expect_not_shown "49000/UDP and TCP"
  expect_shown "For TURN, also forward 3478/UDP and the UDP relay range 49160-49200."
  # .env.example's own lines for the installer's settings stay comments.
  [[ $(assignments STELLARR_DOMAIN "$env_file") -eq 1 ]] || fail ".env sets STELLARR_DOMAIN more than once."
  # Every file is written through a temporary one beside it, which goes.
  [[ $(find "$home/stellarr" -mindepth 1 | wc -l) -eq 3 ]] || fail "more than three files: $(ls -A "$home/stellarr")"

  # With the release named, nothing is looked up.
  new_case exact-version
  run --yes --version 1.4.0
  expect_success
  expect_called "https GET https://raw.githubusercontent.com/Arkazzae/Stellarr/v1.4.0/docker-compose.yml"
  expect_not_called "releases/latest"
  expect_line "$home/stellarr/.env" "STELLARR_VERSION=1.4.0"
}

case_piped_without_terminal() {
  new_case piped-without-terminal
  local tty_env=(STELLARR_INSTALLER_TTY="$case_dir/no-terminal")
  # Where there is no controlling terminal, the installer's own /dev/tty is
  # what fails to open.
  if no_controlling_terminal; then tty_env=(); fi
  seams
  status=0
  (cd "$work" && env -i "${seam_env[@]}" TERM=dumb STELLARR_INSTALLER_UI=plain ${tty_env[@]+"${tty_env[@]}"} \
    "$BASH" -s -- --yes <"$installer" >"$case_dir/out" 2>"$case_dir/err") || status=$?
  expect_success
  expect_not_called sudo
  expect_not_shown "Where will people open Stellarr?"
  expect_line "$home/stellarr/.env" "STELLARR_DOMAIN=localhost"
  expect_line "$home/stellarr/.env" "STELLARR_PUBLIC_IP="
  expect_line "$home/stellarr/.env" "STELLARR_LANGUAGE=en"
  expect_shown "Open: https://localhost:8443/setup"
  # Without --yes nothing is asked either, and no default is taken for one.
  new_case piped-without-yes
  seams
  status=0
  (cd "$work" && env -i "${seam_env[@]}" TERM=dumb STELLARR_INSTALLER_UI=plain ${tty_env[@]+"${tty_env[@]}"} \
    "$BASH" -s <"$installer" >"$case_dir/out" 2>"$case_dir/err") || status=$?
  expect_success
  expect_not_shown "Where will people open Stellarr?"
}

case_interactive_answers() {
  new_case interactive-answers
  answer 'not a host!' 'https://Chat.Example.com:8443/setup' '' 'pl'
  run SSH_CONNECTION="198.51.100.20 50122 203.0.113.7 22"
  expect_success
  expect_line "$home/stellarr/.env" "STELLARR_DOMAIN=chat.example.com"
  expect_line "$home/stellarr/.env" "STELLARR_PUBLIC_IP=auto"
  expect_line "$home/stellarr/.env" "STELLARR_LANGUAGE=pl"
  [[ $(count "Where will people open Stellarr?" "$case_dir/out") -eq 1 ]] || fail "the address question was not asked once."
  [[ $(count "> [203.0.113.7]" "$case_dir/out") -eq 2 ]] || fail "the address was not asked again after a bad answer."
  expect_shown '"not a host!" is not a domain or an IPv4 address'
}

case_no_release() {
  new_case no-release
  no_release
  run --yes
  expect_failure
  expect_shown "Stellarr has no published release yet, so there is no image to pull."
  expect_shown "./install.sh --build"
  expect_missing "$home/stellarr"
  expect_not_called "docker compose config"
}

case_github_unreachable() {
  new_case github-unreachable
  github_down
  run --yes
  expect_failure
  expect_shown "GitHub did not answer"
  expect_not_shown "no published release"
  expect_missing "$home/stellarr"
}

case_missing_file() {
  new_case missing-file
  run --yes --version 1.4.0 --ref no-such-branch
  expect_failure
  expect_shown "Could not download docker-compose.yml for no-such-branch"
  expect_shown "The requested URL returned error: 404"
  expect_missing "$home/stellarr"
}

case_docker_missing() {
  new_case docker-missing
  rm "$bin/docker"
  run --yes
  expect_failure
  expect_shown "Docker is not installed"
  expect_not_called sudo
  expect_not_called get.docker.com
  expect_missing "$home/stellarr"

  new_case docker-missing-declined
  rm "$bin/docker"
  answer n
  run
  expect_failure
  expect_shown "Install Docker Engine now?"
  expect_not_called get.docker.com

  # As root with --install-docker, Docker's own script runs with sh.
  new_case docker-installed
  rm "$bin/docker"
  echo 0 >"$state/uid"
  run --yes --install-docker
  expect_success
  expect_called "https://get.docker.com"
  expect_called "get-docker"
  expect_not_called sudo
}

case_docker_through_sudo() {
  new_case docker-through-sudo
  flag docker-needs-root
  flag sudo-works
  run --yes
  expect_success
  expect_called "sudo -n docker compose --project-name stellarr"
  expect_called "compose up -d --no-build --pull never"
  expect_shown "sudo docker compose logs -f"

  # On macOS Docker Desktop runs as the person: sudo is never tried, and
  # the message names Docker Desktop.
  new_case docker-desktop-stopped
  echo Darwin >"$state/os"
  flag docker-down
  answer y
  run
  expect_failure
  expect_shown "Docker Desktop is not running"
  expect_not_called sudo

  new_case docker-desktop-missing
  echo Darwin >"$state/os"
  rm "$bin/docker"
  answer y
  run
  expect_failure
  expect_shown "Install Docker Desktop"
  expect_not_called sudo
  expect_not_called get.docker.com

  # Without a terminal, sudo that wants a password is not waited for.
  new_case docker-sudo-needs-password
  flag docker-needs-root
  run --yes
  expect_failure
  expect_shown "this user cannot reach it"
  expect_not_called "sudo docker"
}

case_compose_v1() {
  new_case compose-v1
  flag no-compose-plugin
  ln -s "$stub" "$bin/docker-compose"
  run --yes
  expect_failure
  expect_shown "Docker Compose 1.29.2 is too old"
  expect_missing "$home/stellarr"

  new_case compose-standalone-v2
  flag no-compose-plugin
  ln -s "$stub" "$bin/docker-compose"
  echo 2.29.7 >"$state/standalone-compose-version"
  run --yes
  expect_success
  expect_called "docker-compose --project-name stellarr"
  expect_called "compose up -d --no-build --pull never"

  # docker-compose.yml needs env_file's required: false, from Compose 2.24.0.
  new_case compose-below-floor
  echo v2.23.3 >"$state/compose-version"
  run --yes
  expect_failure
  expect_shown "Docker Compose 2.23.3 is too old"
  expect_shown "env_file's required: false"
  expect_missing "$home/stellarr"

  new_case compose-at-floor
  echo 2.24.0 >"$state/compose-version"
  run --yes
  expect_success
  expect_called "compose up -d --no-build --pull never"
}

case_found_by_label() {
  new_case found-by-label
  local dir="$case_dir/srv/stellarr" before
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  registration '{"setupRequired":false}'
  before=$(snapshot "$dir")
  run --yes
  expect_success
  expect_shown "Stellarr 1.4.0 in $dir is the latest release"
  expect_called "up -d --no-build --pull never @$dir"
  expect_missing "$home/stellarr"
  # The same release again changes no file and makes no backup.
  [[ $(snapshot "$dir") == "$before" ]] || fail "a file in the installation changed."
  expect_shown "The owner account exists; sign in as usual."
  expect_not_shown "Setup code"
}

case_conflicting_dir() {
  new_case conflicting-dir
  local dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  run --yes --dir "$case_dir/other"
  expect_failure
  expect_shown "Stellarr is installed in $dir, not in $case_dir/other"
  expect_shown "--dir $dir"
  expect_missing "$case_dir/other"
  expect_not_called "compose config"

  # A stopped container belongs to its directory as much.
  new_case conflicting-dir-stopped
  dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0 false
  run --yes --dir "$case_dir/other"
  expect_failure
  expect_missing "$case_dir/other"
}

# Another project's docker-compose.yml is never replaced, a Compose file that
# Compose would read in its place stops the run before anything is written,
# and a local override is left for Compose to find, as it would without the
# installer.
case_compose_files() {
  local file
  for file in compose.yaml compose.yml docker-compose.yaml; do
    new_case "compose-file-$file"
    mkdir -p "$case_dir/inst"
    printf 'name: other\nservices:\n  web:\n    image: nginx\n' >"$case_dir/inst/$file"
    run --yes --dir "$case_dir/inst"
    expect_failure
    expect_shown "holds $file, which Docker Compose reads before docker-compose.yml"
    expect_shown "Choose another directory with --dir."
    expect_missing "$case_dir/inst/docker-compose.yml"
    expect_missing "$case_dir/inst/.env"
    expect_not_called "compose config"
  done

  new_case compose-other-project
  local dir="$case_dir/inst"
  mkdir -p "$dir"
  printf 'name: other\nservices: {}\n' >"$dir/docker-compose.yml"
  cp "$dir/docker-compose.yml" "$case_dir/compose-before"
  run --yes --dir "$dir"
  expect_failure
  expect_shown "not Stellarr's"
  cmp -s "$case_dir/compose-before" "$dir/docker-compose.yml" || fail "the other project's file changed."
  expect_missing "$dir/.env"

  new_case compose-override
  dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  echo 'services: {}' >"$dir/compose.override.yaml"
  run --yes
  expect_success
  expect_called "compose up -d --no-build --pull never @$dir"
  expect_called "-f $dir/compose.override.yaml up -d --no-build --pull never"
}

# A container whose directory is gone is taken over only when asked to.
case_container_dir_gone() {
  new_case container-dir-gone
  container "$case_dir/gone" 1.4.0 false
  run --yes
  expect_failure
  expect_shown "which no longer exists"
  expect_shown "with --force"
  expect_missing "$case_dir/gone"
  expect_missing "$home/stellarr"

  new_case container-dir-gone-forced
  container "$case_dir/gone" 1.4.0 false
  run --yes --force
  expect_success
  expect_called "up -d --no-build --pull never @$case_dir/gone"
}

# Compose reads the installation's .env alone: a project name or a setting
# exported in the shell does not reach it.
case_compose_environment() {
  new_case compose-environment
  run COMPOSE_PROJECT_NAME=stellarr-dev COMPOSE_DISABLE_ENV_FILE=1 STELLARR_TURN_RELAY_RANGE=50000-50040 STELLARR_DOMAIN=chat.example.com --yes
  expect_success
  expect_called "compose up sees COMPOSE_PROJECT_NAME=unset STELLARR_TURN_RELAY_RANGE=unset COMPOSE_DISABLE_ENV_FILE=unset"
  expect_shown "Ignored from the environment, as Compose reads .env: STELLARR_TURN_RELAY_RANGE"
  expect_line "$home/stellarr/.env" "STELLARR_DOMAIN=chat.example.com"
}

case_not_compose() {
  new_case not-compose
  container '' 1.4.0
  run --yes
  expect_failure
  expect_shown "Docker Compose did not start it"
  expect_missing "$home/stellarr"
  expect_not_called "compose config"
}

case_update() {
  new_case update
  local dir="$case_dir/srv/stellarr" backups
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  release 1.5.0
  cp "$dir/.env" "$case_dir/env-before"
  run --yes --dir "$dir"
  expect_success
  expect_shown "Latest release is 1.5.0"
  cmp -s "$fixtures/compose-1.5.0" "$dir/docker-compose.yml" || fail "docker-compose.yml is not 1.5.0's."
  backups=("$dir"/docker-compose.yml.bak-*)
  [[ ${#backups[@]} -eq 1 && -f ${backups[0]} ]] || fail "not exactly one backup of docker-compose.yml."
  cmp -s "$fixtures/compose-1.4.0" "${backups[0]}" || fail "the backup is not the previous docker-compose.yml."
  sed 's/^STELLARR_VERSION=1.4.0$/STELLARR_VERSION=1.5.0/' "$case_dir/env-before" >"$case_dir/env-expected"
  cmp -s "$case_dir/env-expected" "$dir/.env" || fail ".env changed beyond STELLARR_VERSION."
  backups=("$dir"/.env.bak-*)
  [[ ${#backups[@]} -eq 1 ]] || fail ".env has more than one backup."
  cmp -s "$case_dir/env-before" "${backups[0]}" || fail "the backup is not the previous .env."
  expect_called "pull --quiet --policy always stellarr @$dir"
  expect_called "up -d --no-build --pull never @$dir"

  # Without a terminal and without --yes, an update is not confirmed for the
  # person.
  new_case update-unconfirmed
  dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  release 1.5.0
  run --dir "$dir"
  expect_failure
  expect_shown "needs a confirmation"
  cmp -s "$fixtures/compose-1.4.0" "$dir/docker-compose.yml" || fail "docker-compose.yml changed."

  # Declined in a terminal, nothing changes.
  new_case update-declined
  dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  release 1.5.0
  answer n
  run --dir "$dir"
  expect_success
  expect_shown "Update Stellarr from 1.4.0 to 1.5.0?"
  expect_shown "Nothing was changed."
  cmp -s "$fixtures/compose-1.4.0" "$dir/docker-compose.yml" || fail "docker-compose.yml changed."
  expect_not_called "pull"

  # An older release is not started on newer data.
  new_case downgrade
  dir="$case_dir/srv/stellarr"
  installed "$dir" 1.5.0
  container "$dir" 1.5.0
  release 1.5.0
  release 1.4.0
  run --yes --dir "$dir" --version 1.4.0
  expect_failure
  expect_shown "1.4.0 is older"
  expect_not_called "pull"
}

# The container reads .env as it is, so settings without a STELLARR_* name
# live there too, as raw Section__Key settings. An update leaves every such line
# as it was, where it was, and changes only the version.
case_staged_installation() {
  local dir pid
  for problem in config pull volume live-config; do
    new_case "staged-failure-$problem"
    dir="$home/stellarr"
    installed "$dir" 1.4.0
    release 1.5.0
    cp "$dir/.env" "$case_dir/env-before"
    case $problem in
      volume) echo unrelated_volume >"$state/config-volume" ;;
      *) flag "$problem-fails" ;;
    esac
    run --yes
    expect_failure
    expect_not_called "up -d"
    expect_missing "$dir/.stellarr-install-stage"
    cmp -s "$fixtures/compose-1.4.0" "$dir/docker-compose.yml" || fail "replaced the old Compose file"
    cmp -s "$case_dir/env-before" "$dir/.env" || fail "replaced the old environment"
    [[ $problem != live-config ]] || expect_shown "Restored the previous installation files"
  done

  new_case staged-health-failure
  dir="$home/stellarr"
  installed "$dir" 1.4.0
  release 1.5.0
  flag never-healthy
  run STELLARR_START_TIMEOUT=1 --yes
  expect_failure
  expect_called "up -d"
  expect_line "$dir/.env" STELLARR_VERSION=1.5.0
  expect_missing "$dir/.stellarr-install-stage"
  cmp -s "$fixtures/compose-1.5.0" "$dir/docker-compose.yml" || fail "restored old configuration after start"

  new_case staged-interruption-lock-and-recovery
  dir="$home/stellarr"
  installed "$dir" 1.4.0
  release 1.5.0
  echo 'an unrelated operator file' >"$dir/pending"
  flag pause-live-config
  seams
  (
    cd "$work"
    exec env -i "${seam_env[@]}" TERM=dumb STELLARR_INSTALLER_UI=plain \
      "$binary" --yes --no-start </dev/null >"$case_dir/first-out" 2>"$case_dir/first-err"
  ) &
  pid=$!
  for ((tick=0; tick<250; tick++)); do
    [[ ! -e $state/live-config-paused ]] || break
    sleep .02
  done
  if [[ ! -e $state/live-config-paused ]]; then
    kill -TERM "$pid" 2>/dev/null || :
    wait "$pid" || :
    fail "never reached validation after publication"
  fi
  run --yes --no-start
  expect_failure
  expect_shown "another installer is already running"
  kill -KILL "$pid"
  wait "$pid" 2>/dev/null || :
  rm "$state/pause-live-config"
  flag config-fails
  run --yes --no-start
  expect_failure
  expect_shown "Restored the files from before the interrupted installation"
  expect_line "$dir/.env" STELLARR_VERSION=1.4.0
  expect_line "$dir/pending" 'an unrelated operator file'
  expect_missing "$dir/.stellarr-install-stage"
  cmp -s "$fixtures/compose-1.4.0" "$dir/docker-compose.yml" || fail "failed to recover interrupted publication"
}

case_sidecar_preparation() {
  local dir build_mode
  for build_mode in release checkout; do
    new_case "sidecar-$build_mode"
    dir="$home/stellarr"
    mkdir -p "$dir/helper"
    printf ',"helper":{"build":{"context":"PROJECT_DIR/helper"}},"registry":{"image":"registry.example/sidecar:remote"},"local":{"image":"local-only:cached","pull_policy":"never"}\n' >"$state/config-extra-services"
    flag image-cached
    if [[ $build_mode == checkout ]]; then
      checkout_files "$dir"
      echo 'FROM scratch' >"$dir/Dockerfile"
      run --yes --build --dir "$dir"
      expect_called "compose build helper stellarr"
    else
      run --yes
      expect_called "compose build helper"
      expect_line "$dir/.env" STELLARR_VERSION=1.4.0
    fi
    expect_success
    expect_called "compose pull registry"
    expect_not_called "compose pull local"
    expect_called "image inspect local-only:cached"
    expect_called "\"context\":\"$dir/helper\""
    expect_called "compose up -d --no-build --pull never"
  done

  new_case missing-local-sidecar-image
  dir="$home/stellarr"
  printf ',"local":{"image":"local-only:cached","pull_policy":"never"}\n' >"$state/config-extra-services"
  run --yes
  expect_failure
  expect_shown "Compose service local requires local image local-only:cached"
  expect_not_called "compose up"
  expect_missing "$dir/.env"
}

case_env_compose_syntax() {
  local dir
  new_case env-colon-project-name
  dir="$home/stellarr"
  mkdir -p "$dir"
  printf 'COMPOSE_PROJECT_NAME: unrelated\n' >"$dir/.env"
  run --yes
  expect_failure
  expect_shown ".env sets COMPOSE_PROJECT_NAME"
  expect_missing "$dir/docker-compose.yml"

  new_case env-bom-and-multiline
  dir="$home/stellarr"
  mkdir -p "$dir"
  printf "\357\273\277PASSWORD='first\nCOMPOSE_PROJECT_NAME: unrelated\nlast'\n" >"$dir/.env"
  run --yes --no-start
  expect_success
  printf "\357\273\277%s\nSTELLARR_VERSION=1.4.0\nPASSWORD='first\nCOMPOSE_PROJECT_NAME: unrelated\nlast'\n" "$marker" >"$case_dir/expected"
  cmp -s "$case_dir/expected" "$dir/.env" || fail "did not preserve the BOM or multiline value"
}

case_env_raw_keys() {
  new_case env-raw-keys
  local dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  printf '%s\n' '' '# Kept by hand: settings without a STELLARR_* name.' \
    'Logging__LogLevel__Stellarr.Api.Requests=Warning' \
    'Oidc__Providers__work__DisplayName=Work account' \
    '  Oidc__Providers__work__Authority=https://id.example.com/realms/work  ' \
    'STELLARR_SMTP_PASSWORD="pa ss # word"' \
    "Oidc__Providers__work__ClientSecret='s3cr3t with \$pace'" >>"$dir/.env"
  container "$dir" 1.4.0
  release 1.5.0
  cp "$dir/.env" "$case_dir/env-before"
  run --yes --dir "$dir"
  expect_success
  sed 's/^STELLARR_VERSION=1.4.0$/STELLARR_VERSION=1.5.0/' "$case_dir/env-before" >"$case_dir/env-expected"
  cmp -s "$case_dir/env-expected" "$dir/.env" || fail ".env changed beyond STELLARR_VERSION."
  expect_line "$dir/.env" "STELLARR_VERSION=1.5.0"
  expect_line "$dir/.env" "Logging__LogLevel__Stellarr.Api.Requests=Warning"
  expect_line "$dir/.env" "  Oidc__Providers__work__Authority=https://id.example.com/realms/work  "
  expect_called "up -d --no-build --pull never @$dir"
}

case_env_without_marker() {
  new_case env-adopted
  local dir="$case_dir/inst" backups
  mkdir -p "$dir"
  printf 'STELLARR_DOMAIN=chat.example.com\nSTELLARR_VERSION=1.4.0\nSTELLARR_GIPHY_API_KEY=abc\n' >"$dir/.env"
  cp "$dir/.env" "$case_dir/env-before"
  run --yes --dir "$dir"
  expect_success
  { echo "$marker"; cat "$case_dir/env-before"; } >"$case_dir/env-expected"
  cmp -s "$case_dir/env-expected" "$dir/.env" || fail "the adopted .env is not the old one under the marker."
  backups=("$dir"/.env.bak-*)
  cmp -s "$case_dir/env-before" "${backups[0]}" || fail "the adopted .env has no backup."

  new_case env-development
  dir="$case_dir/inst"
  mkdir -p "$dir"
  printf 'COMPOSE_PROJECT_NAME=stellarr-dev\nSTELLARR_DOMAIN=localhost\n' >"$dir/.env"
  cp "$dir/.env" "$case_dir/env-before"
  run --yes --dir "$dir"
  expect_failure
  expect_shown "COMPOSE_PROJECT_NAME"
  cmp -s "$case_dir/env-before" "$dir/.env" || fail "the development .env changed."
  expect_missing "$dir/docker-compose.yml"

  new_case env-forced
  dir="$case_dir/inst"
  mkdir -p "$dir"
  printf 'COMPOSE_PROJECT_NAME=stellarr-dev\nSTELLARR_DOMAIN=old.example.com\n' >"$dir/.env"
  cp "$dir/.env" "$case_dir/env-before"
  run --yes --force --dir "$dir"
  expect_success
  expect_line "$dir/.env" "$marker"
  expect_line "$dir/.env" "STELLARR_DOMAIN=localhost"
  backups=("$dir"/.env.bak-*)
  cmp -s "$case_dir/env-before" "${backups[0]}" || fail "the replaced .env has no backup."

  # An empty .env is adopted: the marker goes first and the release under it.
  new_case env-empty
  dir="$case_dir/inst"
  mkdir -p "$dir"
  : >"$dir/.env"
  run --yes --dir "$dir"
  expect_success
  [[ $(head -n 1 "$dir/.env") == "$marker" ]] || fail "the marker is not the first line of .env."
  [[ $(assignments STELLARR_VERSION "$dir/.env") -eq 1 ]] || fail ".env does not set STELLARR_VERSION once."
  expect_line "$dir/.env" "STELLARR_VERSION=1.4.0"
}

# A Compose variable in a .env the installer wrote stops the run as well,
# naming it, and --force still writes a new .env beside a backup.
case_env_marked_compose_variable() {
  new_case env-marked-compose
  local dir="$case_dir/srv/stellarr" before backups
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  echo COMPOSE_PROJECT_NAME=foo >>"$dir/.env"
  before=$(snapshot "$dir")
  run --yes
  expect_failure
  expect_shown "$dir/.env sets COMPOSE_PROJECT_NAME"
  expect_shown "Remove it from .env"
  expect_not_shown "development stack"
  expect_not_called "compose config"
  [[ $(snapshot "$dir") == "$before" ]] || fail "a file in the installation changed."

  new_case env-marked-compose-forced
  dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  printf 'COMPOSE_PROJECT_NAME=foo\nCOMPOSE_FILE=other.yml\n' >>"$dir/.env"
  cp "$dir/.env" "$case_dir/env-before"
  run --yes --force
  expect_success
  expect_line "$dir/.env" "$marker"
  [[ $(assignments COMPOSE_PROJECT_NAME "$dir/.env") -eq 0 ]] || fail "the new .env still sets COMPOSE_PROJECT_NAME."
  backups=("$dir"/.env.bak-*)
  cmp -s "$case_dir/env-before" "${backups[0]}" || fail "the replaced .env has no backup."
}

case_env_values() {
  new_case env-values
  local dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  printf '%s\n' "$marker" 'STELLARR_DOMAIN="old.example.com"' "STELLARR_DOMAIN='chat.example.com'"$'\r' \
    'export STELLARR_HTTPS_PORT=9443 # moved' 'STELLARR_VERSION=1.4.0' >"$dir/.env"
  container "$dir" 1.4.0
  run --yes
  expect_success
  expect_shown "Open: https://chat.example.com:9443"
  expect_called "https://127.0.0.1:9443/api/auth/registration"
}

# Whatever .env holds is data: nothing in it runs, a file with Windows line
# ends and no final newline is kept as it is, and a value the installer needs
# that is not one stops it with the key named.
case_env_odd_values() {
  new_case env-odd-values
  local dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  printf '%s\r\n' "$marker" "STELLARR_DOMAIN=\`touch backtick\`.example.com" "STELLARR_PUBLIC_IP=\$(touch substituted)" \
    'STELLARR_GIPHY_API_KEY="a b ; touch semicolon"' 'STELLARR_VERSION=1.4.0' >"$dir/.env"
  printf 'STELLARR_LANGUAGE=en' >>"$dir/.env"
  cp "$dir/.env" "$case_dir/env-before"
  run --yes
  expect_success
  cmp -s "$case_dir/env-before" "$dir/.env" || fail ".env changed."
  [[ -z $(find "$case_dir" -name backtick -o -name substituted -o -name semicolon) ]] || fail "a value in .env ran."

  new_case env-bad-port
  dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0 false
  printf '%s\n' "$marker" 'STELLARR_HTTPS_PORT="84 43"' 'STELLARR_VERSION=1.4.0' >"$dir/.env"
  run --yes
  expect_failure
  expect_shown 'STELLARR_HTTPS_PORT in'
  expect_not_called "compose config"
}

case_ports() {
  new_case port-busy
  echo 8443 >"$state/busy-tcp"
  run --yes
  expect_failure
  expect_shown "8443 (HTTPS): move it with --https-port"
  expect_not_called "up -d"
  # The check comes before the files, so a second run still asks its
  # questions.
  expect_missing "$home/stellarr"

  new_case port-turns
  echo 5349 >"$state/busy-udp"
  run --yes
  expect_failure
  expect_shown "--turns-port"

  new_case port-relay
  echo 49170 >"$state/busy-udp"
  run --yes
  expect_failure
  expect_shown "STELLARR_TURN_RELAY_RANGE"
  expect_shown "rtc-and-ports"

  new_case port-given
  echo 9443 >"$state/busy-tcp"
  run --yes --https-port 9443
  expect_failure
  expect_shown "choose another with --https-port"

  new_case port-stopped-container
  local dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0 false
  echo 8443 >"$state/busy-tcp"
  run --yes
  expect_failure
  expect_shown "--https-port"
  expect_called "ss -ltn"

  new_case port-running-container
  dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  echo 8443 >"$state/busy-tcp"
  echo '{"443/tcp":[{"HostPort":"8443","HostIp":"0.0.0.0"}]}' >"$state/port-bindings"
  run --yes
  expect_success
  expect_called "ss -ltn"
  expect_shown "held by Stellarr"

  new_case port-running-container-new-conflict
  dir="$case_dir/srv/stellarr"
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  printf '8443\n9443\n' >"$state/busy-tcp"
  echo '{"443/tcp":[{"HostPort":"8443","HostIp":"0.0.0.0"}]}' >"$state/port-bindings"
  run --yes --https-port 9443
  expect_failure
  expect_shown "9443 (HTTPS), as given"
  expect_not_called "up -d"

  new_case port-asked
  printf '8443\n8444\n' >"$state/busy-tcp"
  answer ''
  run --domain localhost --language en
  expect_success
  expect_shown "Which port should HTTPS use?"
  expect_shown "> [8445]"
  expect_line "$home/stellarr/.env" "STELLARR_HTTPS_PORT=8445"
  [[ $(assignments STELLARR_HTTPS_PORT "$home/stellarr/.env") -eq 1 ]] || fail "the port is set twice in .env."
  expect_shown "Ports are free: 8445, 8888"

  # In a terminal the warning after an answer is set off by an empty line.
  new_case port-asked-terminal
  echo 8888 >"$state/busy-tcp"
  answer '' '' ''
  run STELLARR_INSTALLER_UI=terminal LANG=C.UTF-8 TERM=xterm-256color
  expect_success
  [[ -z $(sed -n 1p "$case_dir/err") ]] || fail "no empty line before the port warning."
  sed -n 2p "$case_dir/err" | grep -qF "Port 8888 (HTTP) is in use on this machine" ||
    fail "the port warning does not follow the empty line."
  expect_line "$home/stellarr/.env" "STELLARR_HTTP_PORT=8889"

  new_case port-lsof
  rm "$bin/ss"
  echo 3478 >"$state/busy-udp"
  run --yes
  expect_failure
  expect_shown "--turn-port"

  new_case port-no-tool
  rm "$bin/ss" "$bin/lsof"
  run --yes
  expect_success
  expect_shown "Could not check the ports"
}

case_invalid_options() {
  local options n=0 words
  for options in "--domain bad_host" "--domain 1.2.3.999" "--domain -x.example.com" "--public-ip 300.1.1.1" \
    "--https-port 0" "--https-port 70000" "--media-port abc" "--turns-port=08443" "--language de" \
    "--version bad!tag" "--ref ../main" "--ref main/" "--dir"; do
    n=$((n + 1))
    new_case "invalid-$n"
    case_name="invalid option $options"
    read -r -a words <<<"$options"
    run --yes "${words[@]}"
    expect_status 2
    expect_no_calls
    expect_empty_dir "$home"
  done
  new_case unknown-option
  run --frobnicate
  expect_status 2
  [[ $(wc -l <"$case_dir/err") -eq 1 ]] || fail "an unknown option printed more than one line."
  new_case help
  run --help
  expect_success
  expect_shown "Usage:"
  expect_no_calls
  expect_empty_dir "$home"
}

case_no_start() {
  new_case no-start
  run --yes --no-start --dir "$case_dir/inst"
  expect_success
  [[ -f $case_dir/inst/docker-compose.yml && -f $case_dir/inst/.env ]] || fail "the files were not written."
  expect_not_called "pull"
  expect_not_called "up -d"
  expect_shown "docker compose up -d"
}

case_never_healthy() {
  new_case never-healthy
  flag never-healthy
  run STELLARR_START_TIMEOUT=3 --yes
  expect_status 1
  expect_shown "Stellarr did not start within 3 s"
  expect_shown "api      starting: waiting for the database"
  expect_shown "Setup code: K7QM-2XWP-9HDA"
  expect_shown "The saved configuration is in place."
  [[ $(count "stellarr-init health" "$case_dir/calls") -ge 3 ]] || fail "health was not checked once a second."

  new_case hung-health
  clock=real
  flag hung-health
  local started=$SECONDS
  run STELLARR_START_TIMEOUT=1 --yes
  expect_status 1
  expect_shown "Stellarr did not start within 1 s"
  [[ $((SECONDS - started)) -lt 3 ]] || fail "a hung Docker probe overran the startup timeout."
  sleep 3
  expect_missing "$state/health-child-survived"

  new_case exited
  echo exited >"$state/after-up"
  run --yes
  expect_status 1
  expect_shown "The container stopped (exited)"
}

# A fault in the installer is never silent: it names where, says that
# nothing was removed and where to report it.
case_unexpected_failure() {
  new_case unexpected-failure
  run STELLARR_INSTALLER_TEST_PANIC=1 --yes
  expect_status 70
  expect_shown "The installer stopped at "
  expect_shown "nothing was removed"
  expect_shown "https://github.com/Arkazzae/Stellarr/issues"
}

modern_release() {
  cp "$repo_root/docker-compose.yml" "$fixtures/compose-1.6.0"
  release 1.6.0
  mkdir -p "$raw/v1.6.0/infra/compose"
  cp "$repo_root/infra/compose/turn.yml" "$raw/v1.6.0/infra/compose/turn.yml"
}

case_direct_and_turn() {
  new_case direct-defaults
  modern_release
  echo 3478 >"$state/busy-udp"
  run --yes --domain chat.example.com
  expect_success
  expect_shown "Before you invite anyone"
  expect_shown "Web and API clients use HTTPS on 443/TCP."
  expect_shown "forward 49000/UDP and TCP"
  expect_not_shown "For TURN, also forward"
  expect_not_shown "49160-49200"
  expect_line "$home/stellarr/.env" "STELLARR_HTTPS_PORT=443"
  expect_line "$home/stellarr/.env" "STELLARR_HTTP_PORT=127.0.0.1:8888"
  expect_line "$home/stellarr/.env" "STELLARR_PUBLIC_IP=auto"
  expect_line "$home/stellarr/.env" "STELLARR_TURN_ENABLED=false"
  expect_missing "$home/stellarr/docker-compose.turn.yml"

  new_case direct-media-tcp
  modern_release
  echo 49000 >"$state/busy-tcp"
  run --yes --no-start
  expect_failure
  expect_shown "49000 (voice media): move it with --media-port"

  new_case turn-opt-in
  modern_release
  run --yes --domain chat.example.com --enable-turn --http-port 127.0.0.1:8888
  expect_success
  expect_shown "Before you invite anyone"
  expect_shown "Web and API clients use HTTPS on 443/TCP."
  expect_shown "forward 49000/UDP and TCP"
  expect_shown "For TURN, also forward 3478/UDP and the UDP relay range 49160-49200."
  expect_shown "If you enable TURN/TCP, forward 3478/TCP."
  expect_shown "For TURN over TLS or DTLS,"
  expect_line "$home/stellarr/.env" "STELLARR_TURN_ENABLED=true"
  expect_line "$home/stellarr/.env" "STELLARR_HTTP_PORT=127.0.0.1:8888"
  expect_line "$home/stellarr/.env" "COMPOSE_FILE=docker-compose.yml:docker-compose.turn.yml"
  cmp -s "$repo_root/infra/compose/turn.yml" "$home/stellarr/docker-compose.turn.yml" ||
    fail "the TURN overlay was not downloaded."
  expect_called "compose up -d --no-build --pull never @$home/stellarr"

  expect_called "https GET https://raw.githubusercontent.com/Arkazzae/Stellarr/v1.6.0/infra/compose/turn.yml"
  expect_not_called "https GET https://raw.githubusercontent.com/Arkazzae/Stellarr/v1.6.0/docker-compose.turn.yml"

  new_case turn-overlay-old-release-location
  modern_release
  mv "$raw/v1.6.0/infra/compose/turn.yml" "$raw/v1.6.0/docker-compose.turn.yml"
  run --yes --no-start --enable-turn
  expect_success
  expect_called "https GET https://raw.githubusercontent.com/Arkazzae/Stellarr/v1.6.0/infra/compose/turn.yml"
  expect_called "https GET https://raw.githubusercontent.com/Arkazzae/Stellarr/v1.6.0/docker-compose.turn.yml"
  expect_line "$home/stellarr/.env" "COMPOSE_FILE=docker-compose.yml:docker-compose.turn.yml"
  cmp -s "$repo_root/infra/compose/turn.yml" "$home/stellarr/docker-compose.turn.yml" ||
    fail "the legacy TURN overlay was not downloaded."

  new_case turn-upgrade
  modern_release
  local dir="$home/stellarr"
  installed "$dir" 1.4.0
  printf 'services: {}\n' >"$dir/docker-compose.override.yml"
  run --yes --no-start
  expect_success
  expect_line "$dir/.env" "COMPOSE_FILE=docker-compose.yml:docker-compose.turn.yml:docker-compose.override.yml"
  expect_line "$dir/.env" "STELLARR_TURN_ENABLED=true"
  expect_line "$dir/.env" "STELLARR_HTTPS_PORT=8443"
  expect_line "$dir/.env" "STELLARR_HTTP_PORT=8888"
  expect_shown "docker compose up -d"
  run --yes --no-start --disable-turn
  expect_success
  ! grep -q '^COMPOSE_FILE=' "$dir/.env" || fail "TURN Compose selector was not removed."
  expect_line "$dir/.env" "STELLARR_TURN_ENABLED=false"
  expect_line "$dir/docker-compose.override.yml" "services: {}"

  new_case turn-bad-relay-range
  modern_release
  dir="$home/stellarr"
  installed "$dir" 1.4.0
  printf 'STELLARR_TURN_RELAY_RANGE=54999\n' >>"$dir/.env"
  run --yes --no-start
  expect_failure
  expect_shown "STELLARR_TURN_RELAY_RANGE in $dir/.env needs an ascending port range"
  expect_missing "$dir/docker-compose.turn.yml"

  new_case turn-upgrade-opt-out
  modern_release
  dir="$home/stellarr"
  installed "$dir" 1.4.0
  run --yes --no-start --disable-turn
  expect_success
  expect_line "$dir/.env" "STELLARR_TURN_ENABLED=false"
  expect_line "$dir/.env" "STELLARR_HTTPS_PORT=8443"
  expect_missing "$dir/docker-compose.turn.yml"
}

case_checkout() {
  new_case checkout
  local checkout="$case_dir/Stellarr"
  mkdir -p "$checkout"
  checkout_files "$checkout"
  cp "$installer" "$checkout/"
  echo 'FROM scratch' >"$checkout/Dockerfile"
  no_release
  # install.sh names the checkout it is in to the installer.
  through_install_sh "$checkout/install.sh"
  run --yes --build
  expect_success
  expect_called "compose build stellarr @$checkout"
  expect_called "up -d --no-build --pull never @$checkout"
  expect_line "$checkout/.env" "STELLARR_VERSION=dev"
  expect_line "$checkout/.env" "STELLARR_IMAGE=stellarr-local:dev"
  cmp -s "$repo_root/docker-compose.yml" "$checkout/docker-compose.yml" || fail "the checkout's docker-compose.yml changed."
  expect_shown "Stellarr is running"

  new_case checkout-migrate-turn-path
  checkout="$case_dir/Stellarr"
  checkout_files "$checkout"
  echo 'FROM scratch' >"$checkout/Dockerfile"
  printf '%s\n' "$marker" 'COMPOSE_FILE=docker-compose.yml:docker-compose.turn.yml' >"$checkout/.env"
  printf 'services: {}\n' >"$checkout/compose.override.yml"
  run --yes --build --no-start --dir "$checkout"
  expect_success
  expect_line "$checkout/.env" "STELLARR_TURN_ENABLED=true"
  expect_line "$checkout/.env" "COMPOSE_FILE=docker-compose.yml:infra/compose/turn.yml:compose.override.yml"
  expect_shown "-f infra/compose/turn.yml -f infra/compose/build.yml -f compose.override.yml up -d --build"
  expect_missing "$checkout/docker-compose.turn.yml"

  new_case checkout-retains-legacy-overlays
  checkout="$case_dir/Stellarr"
  checkout_files "$checkout"
  mv "$checkout/infra/compose/turn.yml" "$checkout/docker-compose.turn.yml"
  mv "$checkout/infra/compose/build.yml" "$checkout/docker-compose.build.yml"
  echo 'FROM scratch' >"$checkout/Dockerfile"
  run --yes --build --no-start --enable-turn --dir "$checkout"
  expect_success
  expect_line "$checkout/.env" "STELLARR_TURN_ENABLED=true"
  expect_line "$checkout/.env" "COMPOSE_FILE=docker-compose.yml:docker-compose.turn.yml"
  expect_shown "-f docker-compose.turn.yml -f docker-compose.build.yml up -d --build"

  new_case checkout-no-release
  checkout="$case_dir/Stellarr"
  mkdir -p "$checkout"
  checkout_files "$checkout"
  cp "$installer" "$checkout/"
  echo 'FROM scratch' >"$checkout/Dockerfile"
  no_release
  through_install_sh "$checkout/install.sh"
  run --yes
  expect_failure
  expect_shown "  ./install.sh --build"
  expect_missing "$checkout/.env"
}

# The output in terminal mode, forced, must leave the terminal as it found
# it: the cursor shown again and no colour left on.
expect_clean_terminal() {
  local esc=$'\033' hidden shown_again cursor colour
  hidden=$(grep -oF "${esc}[?25l" "$case_dir/out" | wc -l)
  shown_again=$(grep -oF "${esc}[?25h" "$case_dir/out" | wc -l)
  [[ $hidden -ge 1 ]] || fail "the waiting line never hid the cursor."
  [[ $shown_again -eq $hidden ]] || fail "the cursor was hidden $hidden times and shown $shown_again times."
  cursor=$(cat "$case_dir/out" "$case_dir/err" | grep -o "$esc\\[?25[hl]" | tail -n 1)
  [[ $cursor == "${esc}[?25h" ]] || fail "the cursor is hidden at the end."
  colour=$(cat "$case_dir/out" "$case_dir/err" | grep -o "$esc\\[[0-9;]*m" | tail -n 1)
  [[ $colour == "${esc}[0m" ]] || fail "a colour is left on at the end."
}

case_terminal_mode() {
  local ui=(STELLARR_INSTALLER_UI=terminal LANG=C.UTF-8 TERM=xterm-256color)
  new_case terminal-success
  echo 3 >"$state/healthy-after"
  run "${ui[@]}" --yes
  expect_success
  expect_shown "Stellarr is up after"
  expect_clean_terminal

  new_case terminal-failure
  flag never-healthy
  run "${ui[@]}" STELLARR_START_TIMEOUT=3 --yes
  expect_status 1
  expect_clean_terminal

  # The installer runs in the foreground, as a person's would, so that it
  # receives the interrupt; a helper sends it once the waiting has begun.
  new_case terminal-interrupt
  printf '2\n' >"$state/hung-health"
  printf '3\n' >"$state/healthy-after"
  clock=real
  seams
  local pid_file="$case_dir/pid"
  (
    for ((i = 0; i < 60; i++)); do
      if [[ -s $pid_file && -s $state/health-checks ]] && (( $(cat "$state/health-checks") >= 2 )); then
        kill -INT "$(cat "$pid_file")"
        exit 0
      fi
      sleep 1
    done
  ) &
  status=0
  (cd "$work" && exec "$BASH" -c "echo \$\$ >\"\$0\"; exec \"\$@\"" "$pid_file" env -i "${seam_env[@]}" \
    LANG=C.UTF-8 TERM=xterm-256color STELLARR_INSTALLER_UI=terminal \
    STELLARR_INSTALLER_TTY="$case_dir/no-terminal" STELLARR_START_TIMEOUT=120 "$BASH" "$downloaded" --yes \
    </dev/null >"$case_dir/out" 2>"$case_dir/err") || status=$?
  wait
  expect_status 130
  expect_shown "Cancelled."
  expect_clean_terminal
  expect_empty_dir "$case_dir/tmp"
  sleep 3
  expect_missing "$state/health-child-survived"

  # Without UTF-8 the glyphs are plain characters, and without colour no
  # colour is set.
  new_case terminal-ascii
  echo 2 >"$state/healthy-after"
  run STELLARR_INSTALLER_UI=terminal LANG=C TERM=xterm-256color --yes
  expect_success
  [[ $(tr -d '\000-\177' <"$case_dir/out" | wc -c) -eq 0 ]] || fail "a glyph outside ASCII was shown without UTF-8."
  expect_shown "Stellarr installer"

  new_case terminal-no-color
  echo 2 >"$state/healthy-after"
  run STELLARR_INSTALLER_UI=terminal LANG=C.UTF-8 TERM=xterm-256color NO_COLOR=1 --yes
  expect_success
  ! grep -q "$(printf '\033')\\[[0-9;]*m" "$case_dir/out" "$case_dir/err" || fail "NO_COLOR still set colours."
  expect_shown "✓"

  new_case plain-mode
  run LANG=C.UTF-8 TERM=xterm-256color COLORTERM=truecolor --yes
  expect_success
  ! grep -q $'\033' "$case_dir/out" "$case_dir/err" || fail "plain mode printed an escape sequence."
}

# Plays one case in terminal mode on this terminal, so a change to the ui_
# functions can be looked at without Docker.
demo() {
  local ui=(STELLARR_INSTALLER_UI=terminal "LANG=${LC_ALL:-${LANG:-C.UTF-8}}" "TERM=${TERM:-xterm}"
    "COLORTERM=${COLORTERM:-}" "NO_COLOR=${NO_COLOR:-}")
  new_case "demo-$1"
  clock=real
  case $1 in
    fresh)
      echo 4 >"$state/healthy-after"
      answer 'chat.example.com' '' 'pl'
      ui+=(SSH_CONNECTION="198.51.100.20 50122 203.0.113.7 22")
      ;;
    update)
      installed "$home/stellarr" 1.4.0
      container "$home/stellarr" 1.4.0
      release 1.5.0
      echo 2 >"$state/healthy-after"
      registration '{"setupRequired":false}'
      answer ''
      ;;
    no-release) no_release ;;
    start-failed)
      flag never-healthy
      ui+=(STELLARR_START_TIMEOUT=5)
      ;;
    *)
      echo "Unknown demo: $1 (fresh, update, no-release or start-failed)" >&2
      exit 2
      ;;
  esac
  local tty="$case_dir/no-terminal"
  [[ ! -f $case_dir/answers ]] || tty="$case_dir/answers"
  seams
  (cd "$work" && env -i "${seam_env[@]}" STELLARR_INSTALLER_TTY="$tty" "${ui[@]}" "$binary" </dev/null) || :
}

if [[ ${1:-} == demo ]]; then
  demo "${2:-fresh}"
  exit 0
fi

case_truncated
case_posix_shells
case_install_sh
case_fresh_install
case_piped_without_terminal
case_interactive_answers
case_no_release
case_github_unreachable
case_missing_file
case_docker_missing
case_docker_through_sudo
case_compose_v1
case_found_by_label
case_conflicting_dir
case_compose_files
case_container_dir_gone
case_compose_environment
case_not_compose
case_update
case_staged_installation
case_sidecar_preparation
case_env_compose_syntax
# shellcheck source=infra/tests/install-release.test.sh
source "$repo_root/infra/tests/install-release.test.sh"
case_release_choices
case_env_raw_keys
case_env_without_marker
case_env_marked_compose_variable
case_env_values
case_env_odd_values
case_ports
case_invalid_options
case_no_start
case_never_healthy
case_checkout
case_direct_and_turn
case_unexpected_failure
case_terminal_mode
cases=("$scratch"/cases/*)
echo "PASS: install.sh, ${#cases[@]} cases under bash $BASH_VERSION."
