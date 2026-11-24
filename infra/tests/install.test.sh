#!/usr/bin/env bash
# Runs install.sh against stub commands, so no Docker, network or root is
# needed. The stubs record every call and answer from files each case writes.
# Each case checks what a person or the machine would notice: the files, the
# calls made to Docker, the exit status, a line on screen.
#
#   bash infra/tests/install.test.sh            every case
#   bash infra/tests/install.test.sh demo NAME  one screen on this terminal:
#                                               fresh, update, no-release or start-failed
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
installer="$repo_root/install.sh"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/stellarr-install-test.XXXXXX")"
# Without symbolic links, as the installer and the stubs see it.
scratch="$(cd -P "$scratch" && pwd -P)"
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
for tool in awk cat chmod cmp cp date grep ln mkdir mktemp mv rm sed sleep sort tail touch tr sh dash; do
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

env_version() {
  local value=latest line
  if [[ -f .env ]]; then
    while IFS= read -r line || [[ -n $line ]]; do
      [[ $line == STELLARR_VERSION=* ]] && value=${line#STELLARR_VERSION=}
    done <.env
  fi
  printf '%s' "$value"
}

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
    # GNU wget (with wget-gnu set) and BusyBox's answer --version and report
    # an HTTP error as each really does.
    if [[ $name == wget && $* == --version ]]; then
      [[ -e $state/wget-gnu ]] || { echo "wget: unrecognized option '--version'" >&2; exit 1; }
      echo "GNU Wget 1.21.4 built on linux-gnu."
      exit 0
    fi
    url='' target='' format='' quiet=0
    while [[ $# -gt 0 ]]; do
      case $1 in
        -q) quiet=1 ;;
        -o | -O) target=$2; shift ;;
        -w) format=$2; shift ;;
        --proto | --connect-timeout | --max-time | --retry | --retry-delay | -T | -t) shift ;;
        https://* | http://*) url=$1 ;;
      esac
      shift
    done
    [[ $target != - ]] || target=''
    serve() { if [[ -n $target ]]; then cat "$1" >"$target"; else cat "$1"; fi; }
    not_found() {
      if [[ $name == curl ]]; then
        echo "curl: (22) The requested URL returned error: 404" >&2
        exit 22
      elif [[ -e $state/wget-gnu ]]; then
        # With -q GNU wget prints nothing at all; these are its -nv lines.
        [[ $quiet -eq 1 ]] || printf '%s:\n2026-11-20 22:52:45 ERROR 404: Not Found.\n' "$url" >&2
        exit 8
      fi
      echo "wget: server returned error: HTTP/1.1 404 Not Found" >&2
      exit 1
    }
    latest=$(read_file latest-url https://github.com/Arkazzae/Stellarr/releases)
    case $url in
      https://github.com/Arkazzae/Stellarr/releases/latest)
        [[ ! -e $state/github-down ]] || { echo "curl: (6) Could not resolve host: github.com" >&2; exit 6; }
        printf '%s' "${format//'%{url_effective}'/$latest}"
        ;;
      https://api.github.com/repos/Arkazzae/Stellarr/releases/latest)
        [[ ! -e $state/github-down ]] || { echo "wget: bad address 'api.github.com'" >&2; exit 4; }
        case $latest in
          */releases/tag/*) printf '{\n  "url": "x",\n  "tag_name": "%s",\n  "name": "x"\n}\n' "${latest##*/}" >"$target" ;;
          *) not_found ;;
        esac
        ;;
      https://raw.githubusercontent.com/Arkazzae/Stellarr/*)
        file="$state/raw/${url#https://raw.githubusercontent.com/Arkazzae/Stellarr/}"
        [[ -f $file ]] || not_found
        serve "$file"
        ;;
      https://get.docker.com)
        printf '%s\n' 'echo "get-docker $*" >>"$STUB_LOG"' 'ln -s "$STUB_PROGRAM" "$STUB_BIN/docker"' >"$target"
        ;;
      https://127.0.0.1:*/api/auth/registration)
        [[ -f $state/registration ]] || { echo "curl: (7) Failed to connect" >&2; exit 7; }
        serve "$state/registration"
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
      compose)
        action=$1
        shift
        case $action in
          version)
            [[ ! -e $state/no-compose-plugin ]] || { echo "docker: 'compose' is not a docker command." >&2; exit 1; }
            read_file compose-version 2.39.1
            ;;
          config) [[ ! -e $state/config-fails ]] || { echo "invalid interpolation format" >&2; exit 15; } ;;
          pull)
            [[ ! -e $state/pull-fails ]] || { echo "Error response from daemon: manifest unknown" >&2; exit 1; }
            touch "$state/image-$(env_version)"
            ;;
          build) touch "$state/image-$(env_version)" "$state/built" ;;
          up)
            printf 'compose up sees COMPOSE_PROJECT_NAME=%s STELLARR_TURN_RELAY_RANGE=%s\n' \
              "${COMPOSE_PROJECT_NAME-unset}" "${STELLARR_TURN_RELAY_RANGE-unset}" >>"$STUB_LOG"
            tag=$(env_version)
            [[ -e $state/image-$tag ]] || { echo "Error: No such image: ghcr.io/arkazzae/stellarr:$tag" >&2; exit 1; }
            mkdir -p "$state/container"
            read_file after-up running >"$state/container/status"
            echo true >"$state/container/running"
            echo stellarr >"$state/container/label.com.docker.compose.project"
            echo "$PWD" >"$state/container/label.com.docker.compose.project.working_dir"
            if [[ -e $state/built ]]; then echo dev; else echo "$tag"; fi >"$state/container/label.org.opencontainers.image.version"
            ;;
        esac
        ;;
      container)
        [[ -d $state/container ]] || { echo "Error: No such container: stellarr" >&2; exit 1; }
        [[ $1 == inspect && $2 == --format ]] || exit 1
        render "$3"
        ;;
      image)
        [[ -e $state/image-${2##*:} ]]
        ;;
      exec)
        case $3 in
          health)
            count=$(($(read_file health-checks 0) + 1))
            echo "$count" >"$state/health-checks"
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
cp "$repo_root/docker-compose.yml" "$fixtures/compose-1.4.0"
{ cat "$repo_root/docker-compose.yml"; echo "# 1.5.0"; } >"$fixtures/compose-1.5.0"
cp "$repo_root/.env.example" "$fixtures/env.example"
# Without a checkout beside it, the script downloads its files.
mkdir -p "$scratch/download"
cp "$installer" "$scratch/download/install.sh"
downloaded="$scratch/download/install.sh"
marker='# Written by the Stellarr installer, which changes only the settings it is given.'

# Starts a case: a directory with its own bin, home, working directory and
# stub state, every stub in place, and release 1.4.0 published.
new_case() {
  case_name=$1
  case_dir="$scratch/cases/$1"
  bin="$case_dir/bin" state="$case_dir/state" home="$case_dir/home" work="$case_dir/cwd"
  mkdir -p "$bin" "$state" "$home" "$work"
  : >"$case_dir/calls"
  local name
  for name in docker curl wget sudo uname ss lsof systemctl id df sleep; do
    ln -s "$stub" "$bin/$name"
  done
  release 1.4.0
  echo "2026-11-20T18:30:13Z stellarr-api: Setup code: K7QM-2XWP-9HDA. Open https://chat.example.com:8443/setup to create the owner account." >"$state/logs"
  echo '{"setupRequired":true}' >"$state/registration"
  script=$downloaded
}

release() {
  echo "https://github.com/Arkazzae/Stellarr/releases/tag/v$1" >"$state/latest-url"
  mkdir -p "$state/raw/v$1"
  cp "$fixtures/compose-$1" "$state/raw/v$1/docker-compose.yml"
  cp "$fixtures/env.example" "$state/raw/v$1/.env.example"
}

no_release() { echo https://github.com/Arkazzae/Stellarr/releases >"$state/latest-url"; }
flag() { : >"$state/$1"; }
answer() { printf '%s\n' "$@" >"$case_dir/answers"; }

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
  status=0
  (cd "$work" && env -i PATH="$bin:$tools" HOME="$home" TERM=dumb LANG=C STUB_STATE="$state" \
    STUB_LOG="$case_dir/calls" STUB_PROGRAM="$stub" STUB_BIN="$bin" STELLARR_INSTALLER_TTY="$tty" \
    STELLARR_INSTALLER_UI=plain ${env[@]+"${env[@]}"} "$BASH" "$script" "$@" \
    </dev/null >"$case_dir/out" 2>"$case_dir/err") || status=$?
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
  for ((cut = 0; cut < size; cut += 500)); do cuts+=("$cut"); done
  # The last byte is the final newline; without it the script is whole.
  for ((cut = size - 200; cut < size - 1; cut++)); do cuts+=("$cut"); done
  for cut in "${cuts[@]}"; do
    # BSD head refuses a count of 0.
    if [[ $cut -eq 0 ]]; then : >"$body"; else head -c "$cut" "$installer" >"$body"; fi
    (cd "$work" && env -i PATH="$bin:$tools" HOME="$home" STUB_STATE="$state" STUB_LOG="$case_dir/calls" \
      "$BASH" <"$body" >/dev/null 2>&1) || :
    [[ ! -s $case_dir/calls ]] || fail "the first $cut bytes ran: $(cat "$case_dir/calls")"
    expect_empty_dir "$home"
    expect_empty_dir "$work"
  done
}

# Runs command $@ the way run starts the installer, with output to the
# case's files and the status in $status.
hermetic() {
  status=0
  (cd "$work" && env -i PATH="$bin:$tools" HOME="$home" TERM=dumb STUB_STATE="$state" STUB_LOG="$case_dir/calls" \
    STELLARR_INSTALLER_TTY="$case_dir/no-terminal" STELLARR_INSTALLER_UI=plain "$@" \
    </dev/null >"$case_dir/out" 2>"$case_dir/err") || status=$?
}

case_posix_shells() {
  new_case posix-shells
  local shell
  for shell in sh dash; do
    [[ -e $tools/$shell ]] || continue
    # A file is handed to bash.
    hermetic "$shell" "$downloaded" --help
    [[ $status -eq 0 ]] || fail "$shell install.sh --help exited with $status."
    expect_shown "Usage:"
    expect_not_shown "syntax error"
    # A script on the input cannot be: either bash in POSIX mode reads it, or
    # the guard says to pipe it to bash.
    status=0
    (cd "$work" && env -i PATH="$bin:$tools" HOME="$home" "$shell" -s -- --help <"$downloaded" \
      >"$case_dir/out" 2>"$case_dir/err") || status=$?
    shown "Usage:" || shown "needs bash: curl -fsSL https://stellarr.online/install.sh | bash" ||
      fail "$shell reading the script from its input neither ran it nor asked for bash."
    expect_not_shown "syntax error"
  done
  expect_no_calls
  # Started as sh, bash runs in POSIX mode; the installer leaves it.
  hermetic "$BASH" --posix "$downloaded" --yes
  expect_success
  expect_shown "Stellarr 1.4.0 is running"
}

case_fresh_install() {
  new_case fresh-install
  run --yes --domain chat.example.com --public-ip auto --language pl
  expect_success
  expect_called "curl --proto =https --tlsv1.2 -fsSL --connect-timeout 15 --max-time 120 --retry 3 --retry-delay 2 -o"
  expect_called "https://raw.githubusercontent.com/Arkazzae/Stellarr/v1.4.0/docker-compose.yml"
  expect_called "https://raw.githubusercontent.com/Arkazzae/Stellarr/v1.4.0/.env.example"
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
  expect_called "docker compose pull --quiet @$home/stellarr"
  expect_called "docker compose up -d --no-build @$home/stellarr"
  expect_not_called "sudo"
  expect_shown "Open: https://chat.example.com:8443/setup"
  expect_shown "Setup code: K7QM-2XWP-9HDA"
  # .env.example's own lines for the installer's settings stay comments.
  [[ $(assignments STELLARR_DOMAIN "$env_file") -eq 1 ]] || fail ".env sets STELLARR_DOMAIN more than once."

  # With the release named, nothing is looked up, and the files still come
  # through curl.
  new_case exact-version
  run --yes --version 1.4.0
  expect_success
  expect_called "curl --proto =https --tlsv1.2 -fsSL"
  expect_not_called "releases/latest"
  expect_not_called wget
  expect_line "$home/stellarr/.env" "STELLARR_VERSION=1.4.0"
}

case_piped_without_terminal() {
  new_case piped-without-terminal
  local tty_env=(STELLARR_INSTALLER_TTY="$case_dir/no-terminal")
  # Where there is no controlling terminal, the installer's own /dev/tty is
  # what fails to open.
  if no_controlling_terminal; then tty_env=(); fi
  status=0
  (cd "$work" && env -i PATH="$bin:$tools" HOME="$home" TERM=dumb STUB_STATE="$state" STUB_LOG="$case_dir/calls" \
    STELLARR_INSTALLER_UI=plain ${tty_env[@]+"${tty_env[@]}"} "$BASH" -s -- --yes <"$installer" \
    >"$case_dir/out" 2>"$case_dir/err") || status=$?
  expect_success
  expect_not_called sudo
  expect_not_shown "Where will people open Stellarr?"
  expect_line "$home/stellarr/.env" "STELLARR_DOMAIN=localhost"
  expect_line "$home/stellarr/.env" "STELLARR_PUBLIC_IP="
  expect_line "$home/stellarr/.env" "STELLARR_LANGUAGE=en"
  expect_shown "Open: https://localhost:8443/setup"
  # Without --yes nothing is asked either, and no default is taken for one.
  new_case piped-without-yes
  status=0
  (cd "$work" && env -i PATH="$bin:$tools" HOME="$home" TERM=dumb STUB_STATE="$state" STUB_LOG="$case_dir/calls" \
    STELLARR_INSTALLER_UI=plain ${tty_env[@]+"${tty_env[@]}"} "$BASH" -s <"$installer" \
    >"$case_dir/out" 2>"$case_dir/err") || status=$?
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
  flag github-down
  run --yes
  expect_failure
  expect_shown "GitHub did not answer"
  expect_not_shown "no published release"
  expect_missing "$home/stellarr"
}

case_wget_only() {
  new_case wget-only
  rm "$bin/curl"
  run --yes
  expect_success
  expect_called "wget -q -T 20 -O"
  expect_called "https://api.github.com/repos/Arkazzae/Stellarr/releases/latest"
  expect_line "$home/stellarr/.env" "STELLARR_VERSION=1.4.0"
  new_case wget-no-release
  rm "$bin/curl"
  no_release
  run --yes
  expect_failure
  expect_shown "no published release yet"

  # GNU wget says nothing of a 404 unless it runs with -nv.
  new_case wget-gnu
  rm "$bin/curl"
  flag wget-gnu
  run --yes
  expect_success
  expect_called "wget -nv -T 20 -t 1 -O"
  expect_line "$home/stellarr/.env" "STELLARR_VERSION=1.4.0"

  new_case wget-gnu-no-release
  rm "$bin/curl"
  flag wget-gnu
  no_release
  run --yes
  expect_failure
  expect_shown "no published release yet"
  expect_not_shown "GitHub did not answer"

  new_case wget-gnu-missing-file
  rm "$bin/curl"
  flag wget-gnu
  run --yes --version 1.4.0 --ref no-such-branch
  expect_failure
  expect_shown "Could not download docker-compose.yml for no-such-branch"
  expect_shown "ERROR 404: Not Found."
  expect_not_shown "2026-11-20 22:52:45"
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
  expect_called "sudo -n docker compose up -d --no-build"
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
  expect_called "docker-compose up -d --no-build"

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
  expect_called "docker compose up -d --no-build"
}

case_found_by_label() {
  new_case found-by-label
  local dir="$case_dir/srv/stellarr" before
  installed "$dir" 1.4.0
  container "$dir" 1.4.0
  echo '{"setupRequired":false}' >"$state/registration"
  before=$(snapshot "$dir")
  run --yes
  expect_success
  expect_shown "Stellarr 1.4.0 in $dir is the latest release"
  expect_called "up -d --no-build @$dir"
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
  expect_called "docker compose up -d --no-build @$dir"
  expect_not_called " -f "
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
  expect_called "up -d --no-build @$case_dir/gone"
}

# Compose reads the installation's .env alone: a project name or a setting
# exported in the shell does not reach it.
case_compose_environment() {
  new_case compose-environment
  run COMPOSE_PROJECT_NAME=stellarr-dev STELLARR_TURN_RELAY_RANGE=50000-50040 STELLARR_DOMAIN=chat.example.com --yes
  expect_success
  expect_called "compose up sees COMPOSE_PROJECT_NAME=unset STELLARR_TURN_RELAY_RANGE=unset"
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
  expect_called "pull --quiet @$dir"
  expect_called "up -d --no-build @$dir"

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
  expect_called "up -d --no-build @$dir"
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
  run --yes
  expect_success
  expect_not_called "ss -ltn"

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
  expect_shown "Nothing was removed."
  [[ $(count "stellarr-init health" "$case_dir/calls") -ge 3 ]] || fail "health was not checked once a second."

  new_case exited
  echo exited >"$state/after-up"
  run --yes
  expect_status 1
  expect_shown "The container stopped (exited)"
}

# An unexpected failure is never silent: it names the line and where to
# report it.
case_unexpected_failure() {
  new_case unexpected-failure
  ln -s "$stub" "$bin/date"
  run --yes
  expect_status 97
  expect_shown "The installer stopped at line"
  expect_shown "https://github.com/Arkazzae/Stellarr/issues"
}

case_checkout() {
  new_case checkout
  local checkout="$case_dir/Stellarr"
  mkdir -p "$checkout"
  cp "$installer" "$repo_root/docker-compose.yml" "$repo_root/.env.example" "$checkout/"
  echo 'FROM scratch' >"$checkout/Dockerfile"
  no_release
  script="$checkout/install.sh"
  run --yes --build
  expect_success
  expect_called "compose build @$checkout"
  expect_called "up -d --no-build @$checkout"
  expect_line "$checkout/.env" "STELLARR_VERSION=latest"
  cmp -s "$repo_root/docker-compose.yml" "$checkout/docker-compose.yml" || fail "the checkout's docker-compose.yml changed."
  expect_shown "Stellarr is running"

  new_case checkout-no-release
  checkout="$case_dir/Stellarr"
  mkdir -p "$checkout"
  cp "$installer" "$repo_root/docker-compose.yml" "$repo_root/.env.example" "$checkout/"
  echo 'FROM scratch' >"$checkout/Dockerfile"
  no_release
  script="$checkout/install.sh"
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
  flag never-healthy
  rm "$bin/sleep"
  local pid_file="$case_dir/pid"
  (
    for ((i = 0; i < 60; i++)); do
      if [[ -s $pid_file ]] && grep -q 'stellarr-init health' "$case_dir/calls"; then
        kill -INT "$(cat "$pid_file")"
        exit 0
      fi
      sleep 1
    done
  ) &
  status=0
  (cd "$work" && exec "$BASH" -c "echo \$\$ >\"\$0\"; exec \"\$@\"" "$pid_file" env -i PATH="$bin:$tools" HOME="$home" \
    LANG=C.UTF-8 TERM=xterm-256color STELLARR_INSTALLER_UI=terminal STUB_STATE="$state" STUB_LOG="$case_dir/calls" \
    STELLARR_INSTALLER_TTY="$case_dir/no-terminal" STELLARR_START_TIMEOUT=120 "$BASH" "$script" --yes \
    </dev/null >"$case_dir/out" 2>"$case_dir/err") || status=$?
  wait
  expect_status 130
  expect_shown "Cancelled."
  expect_clean_terminal

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
  rm "$bin/sleep"
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
      echo '{"setupRequired":false}' >"$state/registration"
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
  (cd "$work" && env -i PATH="$bin:$tools" HOME="$home" STUB_STATE="$state" STUB_LOG="$case_dir/calls" \
    STELLARR_INSTALLER_TTY="$tty" "${ui[@]}" "$BASH" "$script" </dev/null) || :
}

if [[ ${1:-} == demo ]]; then
  demo "${2:-fresh}"
  exit 0
fi

case_truncated
case_posix_shells
case_fresh_install
case_piped_without_terminal
case_interactive_answers
case_no_release
case_github_unreachable
case_wget_only
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
case_unexpected_failure
case_terminal_mode
cases=("$scratch"/cases/*)
echo "PASS: install.sh, ${#cases[@]} cases under bash $BASH_VERSION."
