#!/usr/bin/env bash
# Installs Stellarr with Docker Compose, and updates an installation it made.
#
#   curl -fsSL https://stellarr.online/install.sh | bash
#   curl -fsSL https://stellarr.online/install.sh | bash -s -- --yes --domain chat.example.com
#   ./install.sh               in a checkout
#   ./install.sh --build       in a checkout, building the image from it
#
# --help lists the options. The script is written for the bash 3.2 that macOS
# ships and for the GNU, BSD and BusyBox versions of the tools it runs.

# Any POSIX sh reads this first command and nothing after it. A file is handed
# to bash; a script that arrived through a pipe cannot be read a second time.
if [ -z "${BASH_VERSION:-}" ]; then
  case "$0" in
    sh | -sh | */sh | dash | */dash | ash | */ash | ksh | */ksh | bash | */bash | busybox | */busybox) ;;
    *)
      if [ -f "$0" ] && command -v bash >/dev/null 2>&1; then
        exec bash "$0" "$@"
      fi
      ;;
  esac
  echo "The Stellarr installer needs bash: curl -fsSL https://stellarr.online/install.sh | bash" >&2
  exit 1
fi
if [ "${BASH_VERSINFO[0]}" -lt 3 ] || { [ "${BASH_VERSINFO[0]}" -eq 3 ] && [ "${BASH_VERSINFO[1]}" -lt 2 ]; }; then
  echo "The Stellarr installer needs bash 3.2 or newer; this is bash $BASH_VERSION." >&2
  exit 1
fi
# Started as sh, bash runs in POSIX mode, which reads the rest of this file
# differently.
set +o posix

# Everything the installer shows goes through the ui_ functions, so its look
# changes in one place. Terminal mode has glyphs, colour and a redrawn waiting
# line; plain mode, for logs and provisioning tools, has none of them.
ui_init() {
  local esc=$'\033' locale
  # STELLARR_INSTALLER_UI forces one mode, for the tests and for screen
  # captures, where the output is not a terminal.
  case "${STELLARR_INSTALLER_UI:-}" in
    terminal) UI_TERMINAL=1 ;;
    plain) UI_TERMINAL=0 ;;
    *)
      UI_TERMINAL=0
      if [ -t 1 ] && [ "${TERM:-dumb}" != dumb ]; then UI_TERMINAL=1; fi
      ;;
  esac
  locale="${LC_ALL:-${LC_CTYPE:-${LANG:-}}}"
  UI_UTF8=0
  case "$locale" in *[Uu][Tt][Ff]-8* | *[Uu][Tt][Ff]8*) UI_UTF8=1 ;; esac

  R='' B='' D='' GREEN='' YELLOW='' RED='' ACCENT='' UI_CLEAR=''
  if [ "$UI_TERMINAL" -eq 1 ]; then
    UI_CLEAR="${esc}[K"
    if [ "$UI_COLOR" -eq 1 ] && [ -z "${NO_COLOR:-}" ]; then
      R="${esc}[0m" B="${esc}[1m" D="${esc}[2m"
      # The basic colours follow the terminal's own theme; the accent is the
      # brand's violet, #6f65d1, as near as the terminal can show it.
      GREEN="${esc}[32m" YELLOW="${esc}[33m" RED="${esc}[31m"
      case "${COLORTERM:-}" in
        truecolor | 24bit) ACCENT="${esc}[38;2;111;101;209m" ;;
        *)
          case "${TERM:-}" in
            *256color*) ACCENT="${esc}[38;5;62m" ;;
            *) ACCENT="${esc}[35m" ;;
          esac
          ;;
      esac
    fi
  fi

  if [ "$UI_TERMINAL" -eq 1 ] && [ "$UI_UTF8" -eq 1 ]; then
    G_OK='✓' G_WARN='!' G_FAIL='✗' G_PROMPT='›' G_BULLET='·'
    UI_FRAMES=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏')
  else
    G_OK='ok' G_WARN='!' G_FAIL='x' G_PROMPT='>' G_BULLET='-'
    UI_FRAMES=("-" "\\" "|" "/")
  fi
  UI_OPEN='' UI_ANSWERED=0 UI_FRAME=0 UI_WAIT_NEXT=0 UI_CURSOR_HIDDEN=0
}

ui_banner() {
  local a="$ACCENT" r="$R"
  if [ "$UI_TERMINAL" -eq 0 ]; then
    printf 'Stellarr installer\n'
  elif [ "$UI_UTF8" -eq 0 ]; then
    printf '\n  %sStellarr installer%s\n' "$B" "$r"
  else
    printf '\n'
    printf '  ╭──────────────╮\n'
    printf '  │  %s▂▂%s  %s██%s  %s▆▆%s  │   %sStellarr%s\n' "$a" "$r" "$a" "$r" "$a" "$r" "$B" "$r"
    printf '  │  %s██%s  %s██%s  %s██%s  │   %sSelf-hosted text and voice chat for small groups%s\n' \
      "$a" "$r" "$a" "$r" "$a" "$r" "$D" "$r"
    printf '  │  %s▀▀%s  %s██%s  %s▀▀%s  │\n' "$a" "$r" "$a" "$r" "$a" "$r"
    printf '  ╰─╮ ╭──────────╯\n'
    printf '    ╰─╯\n'
  fi
}

ui_section() {
  ui_settle
  if [ "$UI_TERMINAL" -eq 1 ]; then
    printf '\n  %s%s%s\n' "$B" "$1" "$R"
  else
    printf '==> %s\n' "$1"
  fi
}

ui_ok() { ui_result 1 "$GREEN" "$G_OK" '[ok]' "$@"; }
ui_warn() { ui_result 2 "$YELLOW" "$G_WARN" '[warn]' "$@"; }
ui_fail() { ui_result 2 "$RED" "$G_FAIL" '[error]' "$@"; }

# One result line: the glyph and the text, and any further arguments as lines
# under the text. Plain mode puts the tag first and keeps it to one line. In a
# terminal a line right after an answer is set off from it by an empty line,
# as the next question would be.
ui_result() {
  local fd=$1 colour=$2 glyph=$3 tag=$4 text=$5 more after_answer=$UI_ANSWERED
  shift 5
  ui_settle
  if [ "$UI_TERMINAL" -eq 1 ]; then
    [ "$after_answer" -eq 0 ] || printf '\n' >&"$fd"
    printf '   %s%s%s %s\n' "$colour" "$glyph" "$R" "$text" >&"$fd"
    for more in "$@"; do printf '     %s\n' "$more" >&"$fd"; done
  else
    printf '%s %s' "$tag" "$text" >&"$fd"
    for more in "$@"; do printf ' %s' "$more" >&"$fd"; done
    printf '\n' >&"$fd"
  fi
}

# Lines of text; an empty argument is an empty line, which plain mode leaves
# out. A line that starts with two spaces is a command to type and keeps them.
ui_text() {
  local line
  ui_settle
  for line in "$@"; do
    if [ "$UI_TERMINAL" -eq 0 ]; then
      [ -z "$line" ] || printf '%s\n' "$line"
    elif [ -z "$line" ]; then
      printf '\n'
    else
      printf '  %s\n' "$line"
    fi
  done
}

ui_bullet() {
  local text=$1 more
  shift
  ui_settle
  if [ "$UI_TERMINAL" -eq 1 ]; then
    printf '   %s %s\n' "$G_BULLET" "$text"
    for more in "$@"; do printf '     %s\n' "$more"; done
  else
    printf -- '- %s' "$text"
    for more in "$@"; do printf ' %s' "$more"; done
    printf '\n'
  fi
}

# Pairs of a name and a value in two columns. With -b first the values are
# bold: they are what the person has to read or copy.
ui_rows() {
  local strong='' plain='' width=0 i name
  if [ "$1" = -b ]; then
    strong=$B plain=$R
    shift
  fi
  for ((i = 1; i <= $#; i += 2)); do
    name=${!i}
    [ "${#name}" -le "$width" ] || width=${#name}
  done
  ui_settle
  while [ "$#" -ge 2 ]; do
    if [ "$UI_TERMINAL" -eq 1 ]; then
      printf '    %-*s   %s%s%s\n' "$width" "$1" "$strong" "$2" "$plain"
    else
      printf '%s: %s\n' "$1" "$2"
    fi
    shift 2
  done
}

# Another program's output, indented. Its colours, cursor movements and
# carriage returns are taken out, so they cannot disturb the screen or a log.
ui_block() {
  local esc=$'\033' cr=$'\r'
  ui_settle
  sed -e "s/$esc\\[[0-9;?]*[A-Za-z]//g" -e "s/$esc//g" -e "s/$cr//g" -e 's/^/    /'
}

ui_question() {
  local title=$1 line
  shift
  ui_settle
  if [ "$UI_TERMINAL" -eq 1 ]; then
    printf '\n  %s%s%s\n' "$B" "$title" "$R" >&"$UI_ASK_FD"
    for line in "$@"; do printf '  %s%s%s\n' "$D" "$line" "$R" >&"$UI_ASK_FD"; done
  else
    printf '%s\n' "$title" >&"$UI_ASK_FD"
    for line in "$@"; do printf '%s\n' "$line" >&"$UI_ASK_FD"; done
  fi
}

# Says why an answer was not taken, where the question is.
ui_retry() {
  ui_settle
  if [ "$UI_TERMINAL" -eq 1 ]; then
    printf '   %s%s%s %s\n' "$YELLOW" "$G_WARN" "$R" "$1" >&"$UI_ASK_FD"
  else
    printf '[warn] %s\n' "$1" >&"$UI_ASK_FD"
  fi
}

# Shows the prompt with the default in brackets and reads one line from the
# terminal device into UI_ANSWER, without surrounding blanks. When the device
# has nothing more to give, the installer stops: no answer is ever assumed.
ui_prompt() {
  local shown='' line=''
  [ -z "$1" ] || shown="[$1] "
  ui_settle
  if [ "$UI_TERMINAL" -eq 1 ]; then
    printf '  %s%s%s %s' "$ACCENT" "$G_PROMPT" "$R" "$shown" >&"$UI_ASK_FD"
  else
    printf '%s %s' "$G_PROMPT" "$shown" >&"$UI_ASK_FD"
  fi
  UI_OPEN='prompt'
  if ! IFS= read -r line <&3 && [ -z "$line" ]; then
    ui_fail "The terminal gave no answer, so the installer stopped"
    exit 1
  fi
  # A terminal shows what was typed itself; answers from a file are shown
  # here, so the screen and a log read the same.
  [ -t 3 ] || printf '%s\n' "$line" >&"$UI_ASK_FD"
  UI_OPEN='' UI_ANSWERED=1
  line=${line//$'\r'/}
  line="${line#"${line%%[![:space:]]*}"}"
  UI_ANSWER="${line%"${line##*[![:space:]]}"}"
}

# A yes or no question. $1 is what an empty answer means, y or n.
ui_confirm() {
  local default=$1 title=$2 choices=y/N
  shift 2
  [ "$default" != y ] || choices=Y/n
  ui_question "$title" "$@"
  while :; do
    ui_prompt "$choices"
    case "$UI_ANSWER" in
      '') break ;;
      [Yy] | [Yy][Ee][Ss]) return 0 ;;
      [Nn] | [Nn][Oo]) return 1 ;;
    esac
    ui_retry "Answer y or n"
  done
  [ "$default" = y ]
}

# The line shown while Stellarr starts. A terminal redraws it on every check;
# a log gets it once and then every 30 seconds.
ui_wait() {
  local elapsed=$1 limit=$2 frame
  if [ "$UI_TERMINAL" -eq 1 ]; then
    if [ "$UI_CURSOR_HIDDEN" -eq 0 ]; then
      printf '\033[?25l'
      UI_CURSOR_HIDDEN=1
    fi
    frame=${UI_FRAMES[$((UI_FRAME % ${#UI_FRAMES[@]}))]}
    UI_FRAME=$((UI_FRAME + 1))
    printf '\r%s   %s%s%s Waiting for Stellarr to start, %s s' "$UI_CLEAR" "$ACCENT" "$frame" "$R" "$elapsed"
    UI_OPEN='wait'
  elif [ "$elapsed" -ge "$UI_WAIT_NEXT" ]; then
    if [ "$UI_WAIT_NEXT" -eq 0 ]; then
      printf 'Waiting for Stellarr to start, for up to %s s\n' "$limit"
    else
      printf 'Still waiting, %s s\n' "$elapsed"
    fi
    UI_WAIT_NEXT=$((elapsed + 30))
  fi
}

# Ends a waiting line or an unanswered prompt before anything else is shown.
ui_settle() {
  case "$UI_OPEN" in
    wait) printf '\r%s' "$UI_CLEAR" ;;
    prompt) printf '\n' >&"$UI_ASK_FD" ;;
  esac
  UI_OPEN='' UI_ANSWERED=0
  if [ "$UI_CURSOR_HIDDEN" -eq 1 ]; then
    printf '\033[?25h'
    UI_CURSOR_HIDDEN=0
  fi
}

ui_cancelled() {
  ui_settle
  if [ "$UI_TERMINAL" -eq 1 ]; then
    printf '\n  Cancelled.\n' >&2
  else
    printf 'Cancelled.\n' >&2
  fi
}

ui_usage() {
  cat <<'EOF'
Installs Stellarr with Docker Compose, or updates an installation it made.

Usage:
  curl -fsSL https://stellarr.online/install.sh | bash -s -- [options]
  ./install.sh [options]                in a checkout

Where:
  --dir PATH           The installation's directory. Without it: the checkout
                       the script is in, the directory of the installation
                       Docker runs, the current directory when it holds
                       Stellarr's docker-compose.yml, or ~/stellarr.

Settings, kept in .env (a terminal asks for the first three):
  --domain NAME        Domain or IPv4 address people open (default localhost)
  --public-ip IP       IPv4 address voice calls use, or "auto" to resolve
                       --domain (default: none, voice on this machine only)
  --language LANG      Interface language, en or pl (default en)
  --https-port PORT    HTTPS port on this machine (default 8443)
  --public-port PORT   HTTPS port in links, 443 behind a proxy (default: the
                       HTTPS port)
  --http-port PORT     Port that redirects to HTTPS (default 8888)
  --media-port PORT    UDP port of voice and video (default 49000)
  --turn-port PORT     STUN and TURN port, UDP and TCP (default 3478)
  --turns-port PORT    TURN over TLS port, UDP and TCP (default 5349)

Release:
  --version TAG        Image to run: a release such as 1.4.0, or a moving tag
                       such as latest (default: the latest release, pinned)
  --ref REF            Git tag or branch to take docker-compose.yml from
                       (default: the release's tag)
  --build              Build the image from this checkout instead of pulling

Behaviour:
  -y, --yes            Ask nothing: take the defaults and confirm updates
  --no-start           Write the files and check the ports, but start nothing
  --install-docker     Install Docker Engine from https://get.docker.com when
                       it is missing (Linux)
  --force              Back up .env and write a new one from the options and
                       defaults; take over a container whose directory is gone
  --skip-port-check    Do not check that the ports are free
  --no-color           No colour, as with a non-empty NO_COLOR
  -h, --help           Show this help

Each setting can come from the environment too: STELLARR_INSTALL_DIR,
STELLARR_DOMAIN, PUBLIC_IP, STELLARR_LANGUAGE, STELLARR_HTTPS_PORT,
STELLARR_PUBLIC_PORT, STELLARR_HTTP_PORT, STELLARR_MEDIA_PORT,
STELLARR_TURN_PORT, STELLARR_TURNS_PORT, STELLARR_VERSION, STELLARR_REF.
STELLARR_START_TIMEOUT is how long to wait for the start (default 300 s).

Running the installer again updates Stellarr to the latest release. It owns
docker-compose.yml and replaces it on an update, keeping the old one beside
it; local changes belong in .env or in docker-compose.override.yml.
EOF
}

have() { command -v "$1" >/dev/null 2>&1; }

# Stops the installer: one line says what is wrong, the lines after it say
# what to do.
die() {
  local problem=$1
  shift
  ui_fail "$problem"
  if [ "$#" -gt 0 ]; then ui_text '' "$@" >&2; fi
  exit 1
}

# A mistake in how the installer was started: one line, and exit status 2.
usage_error() {
  ui_fail "$1"
  exit 2
}

settings_init() {
  local key
  # The settings the installer writes to .env. Each is read from the
  # environment variable of the same name first, as Compose itself would.
  MANAGED_KEYS='STELLARR_DOMAIN PUBLIC_IP STELLARR_LANGUAGE STELLARR_HTTPS_PORT STELLARR_PUBLIC_PORT STELLARR_HTTP_PORT STELLARR_MEDIA_PORT STELLARR_TURN_PORT STELLARR_TURNS_PORT'
  REPO_SLUG=Arkazzae/Stellarr
  REPO_URL="https://github.com/$REPO_SLUG"
  RAW_URL="https://raw.githubusercontent.com/$REPO_SLUG"
  IMAGE=ghcr.io/arkazzae/stellarr
  # The documentation will move to its own address; every link starts here.
  DOCS_URL=https://arkazzae.github.io/Stellarr
  ISSUES_URL="$REPO_URL/issues"
  ENV_MARKER='# Written by the Stellarr installer, which changes only the settings it is given.'

  for key in $MANAGED_KEYS; do
    printf -v "GIVEN_$key" '%s' 0
    printf -v "VALUE_$key" '%s' ''
    printf -v "FROM_$key" '%s' "$key"
    if [ -n "${!key:-}" ]; then
      printf -v "GIVEN_$key" '%s' 1
      printf -v "VALUE_$key" '%s' "${!key}"
    fi
  done
  VERSION_OPTION=${STELLARR_VERSION:-} VERSION_FROM=STELLARR_VERSION
  REF_OPTION=${STELLARR_REF:-} REF_FROM=STELLARR_REF
  DIR_OPTION=${STELLARR_INSTALL_DIR:-}
  START_TIMEOUT=${STELLARR_START_TIMEOUT:-300}
  # Where the answers are read. STELLARR_INSTALLER_TTY names another file
  # or FIFO, from which the tests feed them.
  TTY=${STELLARR_INSTALLER_TTY:-/dev/tty}
  # The interface language's default follows the person's locale, read before
  # the installer sets its own.
  LOCALE_LANGUAGE=${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}

  ASSUME_YES=0 BUILD=0 START=1 INSTALL_DOCKER=0 FORCE=0 SKIP_PORT_CHECK=0 UI_COLOR=1
  INTERACTIVE=0 UI_ASK_FD=1 DOCKER_SUDO=0 COMPOSE_PLUGIN=1 DOWNLOADER='' WGET_GNU=0
  WORK='' SCRIPT_CHECKOUT='' DIR_IS_CHECKOUT=0 ENV_CREATED=0 ENV_BACKED_UP=0
}

option_key() {
  case "$1" in
    --domain) OPTION_KEY=STELLARR_DOMAIN ;;
    --public-ip) OPTION_KEY=PUBLIC_IP ;;
    --language) OPTION_KEY=STELLARR_LANGUAGE ;;
    --https-port) OPTION_KEY=STELLARR_HTTPS_PORT ;;
    --public-port) OPTION_KEY=STELLARR_PUBLIC_PORT ;;
    --http-port) OPTION_KEY=STELLARR_HTTP_PORT ;;
    --media-port) OPTION_KEY=STELLARR_MEDIA_PORT ;;
    --turn-port) OPTION_KEY=STELLARR_TURN_PORT ;;
    --turns-port) OPTION_KEY=STELLARR_TURNS_PORT ;;
    *) return 1 ;;
  esac
}

# The option that sets a key, for messages that tell the person what to type.
key_option() {
  case "$1" in
    STELLARR_DOMAIN) KEY_OPTION=--domain ;;
    PUBLIC_IP) KEY_OPTION=--public-ip ;;
    STELLARR_LANGUAGE) KEY_OPTION=--language ;;
    STELLARR_HTTPS_PORT) KEY_OPTION=--https-port ;;
    STELLARR_PUBLIC_PORT) KEY_OPTION=--public-port ;;
    STELLARR_HTTP_PORT) KEY_OPTION=--http-port ;;
    STELLARR_MEDIA_PORT) KEY_OPTION=--media-port ;;
    STELLARR_TURN_PORT) KEY_OPTION=--turn-port ;;
    STELLARR_TURNS_PORT) KEY_OPTION=--turns-port ;;
  esac
}

parse_args() {
  local option value inline
  while [ "$#" -gt 0 ]; do
    option=$1 value='' inline=0
    case "$option" in
      --*=*)
        value=${option#*=} option=${option%%=*} inline=1
        ;;
    esac
    case "$option" in
      --dir | --domain | --public-ip | --language | --https-port | --public-port | \
        --http-port | --media-port | --turn-port | --turns-port | --version | --ref)
        if [ "$inline" -eq 0 ]; then
          [ "$#" -ge 2 ] || usage_error "$option needs a value; see --help"
          value=$2
          shift
        fi
        case "$option" in
          --dir) DIR_OPTION=$value ;;
          --version) VERSION_OPTION=$value VERSION_FROM=--version ;;
          --ref) REF_OPTION=$value REF_FROM=--ref ;;
          *)
            option_key "$option"
            printf -v "GIVEN_$OPTION_KEY" '%s' 1
            printf -v "VALUE_$OPTION_KEY" '%s' "$value"
            printf -v "FROM_$OPTION_KEY" '%s' "$option"
            ;;
        esac
        ;;
      -y | --yes | --build | --no-start | --install-docker | --force | --skip-port-check | --no-color | -h | --help)
        [ "$inline" -eq 0 ] || usage_error "$option takes no value; see --help"
        case "$option" in
          -y | --yes) ASSUME_YES=1 ;;
          --build) BUILD=1 ;;
          --no-start) START=0 ;;
          --install-docker) INSTALL_DOCKER=1 ;;
          --force) FORCE=1 ;;
          --skip-port-check) SKIP_PORT_CHECK=1 ;;
          --no-color) UI_COLOR=0 ;;
          -h | --help)
            ui_usage
            exit 0
            ;;
        esac
        ;;
      *) usage_error "Unknown option $option; see --help" ;;
    esac
    shift
  done
}

is_port() {
  case "$1" in '' | *[!0-9]* | 0*) return 1 ;; esac
  [ "${#1}" -le 5 ] && [ "$1" -le 65535 ]
}

is_ipv4() {
  local IFS=. octet count=0
  case "$1" in '' | *[!0-9.]* | .* | *. | *..*) return 1 ;; esac
  for octet in $1; do
    count=$((count + 1))
    case "$octet" in ? | ?? | ???) ;; *) return 1 ;; esac
    [ $((10#$octet)) -le 255 ] || return 1
  done
  [ "$count" -eq 4 ]
}

# Letters, digits, dots and hyphens, at most 253 characters, no label longer
# than 63 or starting or ending with a hyphen. Digits and dots alone would be
# an IP address, which is_ipv4 judges.
is_hostname() {
  local IFS=. label
  [ "${#1}" -ge 1 ] && [ "${#1}" -le 253 ] || return 1
  case "$1" in *[!a-z0-9.-]* | .* | *. | *..*) return 1 ;; esac
  case "$1" in *[!0-9.]*) ;; *) return 1 ;; esac
  for label in $1; do
    [ "${#label}" -le 63 ] || return 1
    case "$label" in -* | *-) return 1 ;; esac
  done
}

is_address() { is_ipv4 "$1" || is_hostname "$1"; }

# Whether people reach the address only on this machine.
is_local_address() {
  case "$1" in localhost | 127.*) return 0 ;; esac
  return 1
}

# A docker tag: the image versions are tags, and so are latest and 1.4.
is_tag() {
  local pattern='^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$'
  [[ $1 =~ $pattern ]]
}

is_exact_version() {
  local pattern='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
  [[ $1 =~ $pattern ]]
}

is_ref() {
  local pattern='^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$'
  [[ $1 =~ $pattern ]] || return 1
  case "$1" in *..* | *//* | */) return 1 ;; esac
}

# Sets ADDRESS to the bare host in what was typed or passed: a pasted scheme,
# path or port goes, and letters become lower case.
normalize_address() {
  local value
  value=$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  value=${value#*://}
  value=${value%%[/?#]*}
  value=${value##*@}
  case "$value" in *:*) value=${value%:*} ;; esac
  ADDRESS=${value%.}
}

# Whether release $1 is older than release $2. A pre-release comes before
# the release of the same number.
version_older() {
  local a=${1%%-*} b=${2%%-*} a_pre='' b_pre='' i x y
  case "$1" in *-*) a_pre=${1#*-} ;; esac
  case "$2" in *-*) b_pre=${2#*-} ;; esac
  for i in 1 2 3; do
    x=${a%%.*} y=${b%%.*}
    a=${a#*.} b=${b#*.}
    if [ "$x" -lt "$y" ]; then return 0; fi
    if [ "$x" -gt "$y" ]; then return 1; fi
  done
  [ -n "$a_pre" ] || return 1
  [ -n "$b_pre" ] || return 0
  [[ $a_pre < $b_pre ]]
}

# Checks the values given before anything else happens, so that a mistyped
# option stops the installer with nothing written.
validate_options() {
  local key given value from
  for key in $MANAGED_KEYS; do
    given="GIVEN_$key" value="VALUE_$key" from="FROM_$key"
    [ "${!given}" -eq 1 ] || continue
    case "$key" in
      STELLARR_DOMAIN)
        normalize_address "${!value}"
        is_address "$ADDRESS" || usage_error "${!from} \"${!value}\" is not a domain or an IPv4 address"
        printf -v "VALUE_$key" '%s' "$ADDRESS"
        ;;
      PUBLIC_IP)
        case "${!value}" in '' | auto) ;; *)
          is_ipv4 "${!value}" || usage_error "${!from} \"${!value}\" is not an IPv4 address or auto"
          ;;
        esac
        ;;
      STELLARR_LANGUAGE)
        case "${!value}" in en | pl) ;; *) usage_error "${!from} \"${!value}\" is not en or pl" ;; esac
        ;;
      *)
        is_port "${!value}" || usage_error "${!from} \"${!value}\" is not a port from 1 to 65535"
        ;;
    esac
  done
  if [ -n "$VERSION_OPTION" ] && ! is_tag "$VERSION_OPTION"; then
    usage_error "$VERSION_FROM \"$VERSION_OPTION\" is not a release such as 1.4.0, or latest"
  fi
  if [ -n "$REF_OPTION" ] && ! is_ref "$REF_OPTION"; then
    usage_error "$REF_FROM \"$REF_OPTION\" is not a git tag or branch"
  fi
  case "$START_TIMEOUT" in '' | *[!0-9]* | 0) usage_error "STELLARR_START_TIMEOUT \"$START_TIMEOUT\" is not a number of seconds" ;; esac
}

# Decides whether the installer may ask. /dev/tty exists without a
# controlling terminal too; only opening it tells the difference. The device
# must exist first, as opening for reading and writing would create a file.
open_terminal() {
  INTERACTIVE=0 UI_ASK_FD=1
  [ "$ASSUME_YES" -eq 0 ] || return 0
  [ -e "$TTY" ] || return 0
  if ! (: <>"$TTY") 2>/dev/null; then return 0; fi
  if ! { exec 3<"$TTY"; } 2>/dev/null; then return 0; fi
  INTERACTIVE=1
  # With the output going to a log, questions still go where the person is.
  if [ -t 3 ] && [ ! -t 1 ] && { exec 4>>"$TTY"; } 2>/dev/null; then
    UI_ASK_FD=4
  fi
}

on_error() {
  local status=$1 line=$2 command=$3
  # A failure inside a subshell reaches the command that started it, which
  # reports it.
  [ "${BASH_SUBSHELL:-0}" -eq 0 ] || return 0
  trap - ERR
  ui_fail "The installer stopped at line $line: \"$command\" exited with status $status"
  ui_text '' "This is a fault in the installer, and nothing was removed. Please report it" \
    "with the output above at $ISSUES_URL" >&2
  exit "$status"
}

on_exit() {
  [ "${BASH_SUBSHELL:-0}" -eq 0 ] || return 0
  ui_settle
  if [ -n "$WORK" ] && [ -d "$WORK" ]; then rm -rf -- "$WORK"; fi
}

on_signal() {
  trap - INT TERM
  ui_cancelled
  exit "$1"
}

check_host() {
  local raw_arch kib='' free_kib='' disk_path summary memory_text='' disk_text='' low=''
  OS=$(uname -s 2>/dev/null) || OS=unknown
  raw_arch=$(uname -m 2>/dev/null) || raw_arch=unknown
  case "$OS" in
    Linux) OS_NAME=Linux ;;
    Darwin) OS_NAME=macOS ;;
    MINGW* | MSYS* | CYGWIN*)
      die "On Windows, run the installer inside WSL 2, with Docker Desktop"
      ;;
    *) die "The installer supports Linux and macOS, and this machine runs $OS" ;;
  esac
  case "$raw_arch" in
    x86_64 | amd64 | aarch64 | arm64) ;;
    *)
      [ "$BUILD" -eq 1 ] || die "There is no Stellarr image for $raw_arch, only for amd64 and arm64" \
        "In a checkout, ./install.sh --build builds an image for it."
      ;;
  esac

  if [ -r /proc/meminfo ]; then
    kib=$(awk '$1 == "MemTotal:" { print $2; exit }' /proc/meminfo 2>/dev/null) || kib=''
  elif [ "$OS" = Darwin ] && have sysctl; then
    kib=$(sysctl -n hw.memsize 2>/dev/null) || kib=''
    case "$kib" in '' | *[!0-9]*) kib='' ;; *) kib=$((kib / 1024)) ;; esac
  fi
  # Images and the data volume live where Docker keeps its data, not in the
  # installation's directory, so that is the disk that has to have room.
  if [ "$OS" = Darwin ]; then
    disk_path=$HOME
  else
    for disk_path in /var/lib/docker /var/lib /; do [ ! -d "$disk_path" ] || break; done
  fi
  free_kib=$(df -Pk "$disk_path" 2>/dev/null | awk 'NR == 2 { print $4 }') || free_kib=''

  case "$kib" in '' | *[!0-9]*) ;; *)
    memory_text="$(gib "$kib") memory"
    [ "$kib" -ge 2097152 ] || low="$memory_text"
    ;;
  esac
  case "$free_kib" in '' | *[!0-9]*) ;; *)
    disk_text="$(gib "$free_kib") free"
    [ "$free_kib" -ge 2097152 ] || low="${low:+$low and }$disk_text"
    ;;
  esac
  summary="$OS_NAME $raw_arch"
  if [ -z "$low" ]; then
    [ -z "$memory_text" ] || summary="$summary, $memory_text"
    [ -z "$disk_text" ] || summary="$summary, $disk_text"
  fi
  ui_ok "$summary"
  [ -z "$low" ] || ui_warn "Only $low; Stellarr wants 2 GiB of each"
}

# KiB as GiB, with one decimal below 10.
gib() {
  awk -v k="$1" 'BEGIN { g = k / 1048576; if (g < 10) printf "%.1f GiB", g; else printf "%d GiB", g }'
}

# Runs docker as this user, or through sudo when only root can reach the
# daemon. Without a terminal sudo runs with -n, which fails instead of
# asking for a password nobody can type.
dk() {
  if [ "$DOCKER_SUDO" -eq 1 ]; then
    run_sudo docker "$@"
  else
    docker "$@"
  fi
}

run_sudo() {
  if [ "$INTERACTIVE" -eq 1 ]; then
    sudo "$@"
  else
    sudo -n "$@"
  fi
}

# Compose, with the project's files named: a compose.yaml left in the
# directory would otherwise be read instead. Named files turn off Compose's
# own search for an override, so the first override it would have found is
# named too.
compose() {
  local files=(-f docker-compose.yml) override
  for override in compose.override.yml compose.override.yaml docker-compose.override.yml docker-compose.override.yaml; do
    if [ -f "$override" ]; then
      files+=(-f "$override")
      break
    fi
  done
  if [ "$COMPOSE_PLUGIN" -eq 1 ]; then
    dk compose "${files[@]}" "$@"
  elif [ "$DOCKER_SUDO" -eq 1 ]; then
    run_sudo docker-compose "${files[@]}" "$@"
  else
    docker-compose "${files[@]}" "$@"
  fi
}

# The command a person types later for Compose in the installation, as this
# run needed it: with sudo when Docker needed it.
compose_command() {
  local tool='docker compose'
  [ "$COMPOSE_PLUGIN" -eq 1 ] || tool=docker-compose
  [ "$DOCKER_SUDO" -eq 0 ] || tool="sudo $tool"
  COMPOSE_COMMAND="cd $(shell_word "$INSTALL_DIR") && $tool $*"
}

shell_word() {
  case "$1" in
    *[!A-Za-z0-9_./-]*) printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")" ;;
    *) printf '%s' "$1" ;;
  esac
}

check_docker() {
  if ! have docker; then docker_missing; fi
  docker_reach
  compose_find
  ui_ok "Docker $DOCKER_VERSION, Compose $COMPOSE_VERSION"
}

docker_missing() {
  local how='with sudo'
  if [ "$OS" = Darwin ]; then
    die "Docker is not installed" "Install Docker Desktop, start it, and run the installer again:" \
      "  https://docs.docker.com/desktop/setup/install/mac-install/"
  fi
  if [ "$(id -u)" -eq 0 ]; then
    how='as root'
  elif ! have sudo; then
    die "Docker is not installed, and installing it needs root" \
      "Run the installer as root, or install Docker Engine yourself:" \
      "  https://docs.docker.com/engine/install/"
  fi
  if [ "$INSTALL_DOCKER" -eq 0 ]; then
    if [ "$INTERACTIVE" -eq 0 ]; then
      die "Docker is not installed" "Install Docker Engine and run the installer again:" \
        "  https://docs.docker.com/engine/install/" \
        "or run the installer with --install-docker to have it install Docker."
    fi
    ui_fail "Docker is not installed"
    if ! ui_confirm y "Install Docker Engine now?" \
      "Stellarr runs in one container. This downloads Docker's own script" \
      "from https://get.docker.com and runs it $how."; then
      ui_text '' "Install Docker Engine and run the installer again:" \
        "  https://docs.docker.com/engine/install/" >&2
      exit 1
    fi
  fi
  docker_install
}

# Installs Docker Engine with Docker's own script. The person is not added to
# the docker group, which would give them root without a password; this run
# uses sudo instead, and the summary says so.
docker_install() {
  pick_downloader
  download https://get.docker.com "$WORK/get-docker.sh" ||
    die "Could not download Docker's install script from https://get.docker.com" "$DOWNLOAD_ERROR"
  ui_text '' "Installing Docker Engine with its own script; its output follows." ''
  if [ "$(id -u)" -eq 0 ]; then
    sh "$WORK/get-docker.sh" || die "Docker's install script failed" "Its output is above."
  else
    run_sudo sh "$WORK/get-docker.sh" || die "Docker's install script failed" "Its output is above."
  fi
  have docker || die "Docker's install script finished, but there is no docker command"
  ui_ok "Installed Docker Engine"
}

# Finds how this user reaches the Docker daemon: directly, or on Linux
# through sudo when the socket is root's. Docker Desktop runs as the person,
# so sudo cannot help on macOS. A stopped daemon on Linux is started once.
docker_reach() {
  local attempt root=0 told=0
  [ "$(id -u)" -ne 0 ] || root=1
  for attempt in 1 2; do
    if docker_server_version docker; then
      DOCKER_SUDO=0
      return 0
    fi
    if [ "$OS" = Linux ] && [ "$root" -eq 0 ] && have sudo; then
      if sudo -n true 2>/dev/null && docker_server_version sudo -n docker; then
        DOCKER_SUDO=1
        return 0
      fi
      if [ "$INTERACTIVE" -eq 1 ]; then
        if [ "$told" -eq 0 ]; then
          ui_text "This user cannot reach Docker, so the installer runs it with sudo."
          told=1
        fi
        if docker_server_version sudo docker; then
          DOCKER_SUDO=1
          return 0
        fi
      fi
    fi
    if [ "$attempt" -eq 2 ] || [ "$OS" != Linux ] || ! have systemctl; then break; fi
    if [ "$root" -eq 1 ]; then
      systemctl start docker >/dev/null 2>&1 || break
    elif have sudo; then
      run_sudo systemctl start docker >/dev/null 2>&1 || break
    else
      break
    fi
  done
  if [ "$OS" = Darwin ]; then
    die "Docker Desktop is not running" "Start Docker Desktop and run the installer again."
  fi
  if [ "$INTERACTIVE" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then
    die "Docker is installed, but this user cannot reach it" \
      "Without a terminal the installer uses sudo only when sudo asks for no" \
      "password. Run it in a terminal or as root, or check that Docker runs:" \
      "  sudo systemctl start docker"
  fi
  die "Docker is installed, but its daemon does not answer" "Start it and run the installer again:" \
    "  sudo systemctl start docker"
}

docker_server_version() {
  DOCKER_VERSION=$("$@" version --format '{{.Server.Version}}' 2>/dev/null) || return 1
  [ -n "$DOCKER_VERSION" ]
}

compose_find() {
  local version='' major
  if version=$(dk compose version --short 2>/dev/null) && [ -n "$version" ]; then
    COMPOSE_PLUGIN=1
  elif have docker-compose && version=$(docker-compose version --short 2>/dev/null) && [ -n "$version" ]; then
    COMPOSE_PLUGIN=0
  else
    die "Docker Compose is missing" "Install the Compose plugin, the docker-compose-plugin package, and run the" \
      "installer again: https://docs.docker.com/compose/install/linux/"
  fi
  version=${version#v}
  major=${version%%.*}
  case "$major" in '' | *[!0-9]*) die "Could not read the version of Docker Compose: $version" ;; esac
  # Compose 1 cannot read the top-level name or the nested defaults in
  # Stellarr's docker-compose.yml.
  if [ "$major" -lt 2 ]; then
    die "Docker Compose $version is too old for Stellarr's docker-compose.yml" \
      "Install Compose 2 or newer, the docker-compose-plugin package, and run the" \
      "installer again: https://docs.docker.com/compose/install/linux/"
  fi
  COMPOSE_VERSION=$version
}

pick_downloader() {
  [ -z "$DOWNLOADER" ] || return 0
  if have curl; then
    DOWNLOADER=curl
  elif have wget; then
    DOWNLOADER=wget
    case "$(wget --version 2>/dev/null)" in *"GNU Wget"*) WGET_GNU=1 ;; esac
  else
    die "Neither curl nor wget is installed" "Install one of them and run the installer again."
  fi
}

# Downloads $1 to $2 over HTTPS only. The transfer goes to a file in the work
# directory first, so a cut-off one never leaves half a file in place.
download() {
  local url=$1 target=$2 part="$WORK/download.part" try
  DOWNLOAD_ERROR=''
  pick_downloader
  rm -f "$part"
  if [ "$DOWNLOADER" = curl ]; then
    if ! curl --proto '=https' --tlsv1.2 -fsSL --connect-timeout 15 --max-time 120 \
      --retry 3 --retry-delay 2 -o "$part" "$url" 2>"$WORK/download.err"; then
      DOWNLOAD_ERROR=$(tail -n 1 "$WORK/download.err" 2>/dev/null) || DOWNLOAD_ERROR=''
      return 1
    fi
  else
    # wget follows a redirect to plain HTTP, and BusyBox's knows no option
    # against it, so only an https address is accepted at all.
    case "$url" in https://*) ;; *) return 1 ;; esac
    for try in 1 2 3; do
      if wget_get "$url" "$part"; then break; fi
      if [ "$try" -eq 3 ]; then
        DOWNLOAD_ERROR=$(tail -n 1 "$WORK/download.err" 2>/dev/null) || DOWNLOAD_ERROR=''
        # GNU wget starts its error with the date and time.
        DOWNLOAD_ERROR=${DOWNLOAD_ERROR#[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9] [0-9][0-9]:[0-9][0-9]:[0-9][0-9] }
        return 1
      fi
      sleep 2
    done
  fi
  mv -f "$part" "$target"
}

# GNU wget says nothing of an HTTP error with -q, and with -nv only errors
# and one line per file, so a 404 can be told from a network failure.
# BusyBox's prints its errors even with -q, and knows no -nv.
wget_get() {
  if [ "$WGET_GNU" -eq 1 ]; then
    wget -nv -T 20 -t 1 -O "$2" "$1" 2>"$WORK/download.err"
  else
    wget -q -T 20 -O "$2" "$1" 2>"$WORK/download.err"
  fi
}

# Sets LATEST_TAG and LATEST_VERSION from the newest release. Otherwise it
# returns 1 with RELEASE_PROBLEM set: "none" when GitHub says there is no
# release, "unreachable" when it did not answer. With curl the release page's
# redirect tells, which has no rate limit, unlike GitHub's API.
find_latest_release() {
  local final='' tag='' pattern='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
  LATEST_TAG='' LATEST_VERSION='' RELEASE_PROBLEM=''
  if [ "$DOWNLOADER" = curl ]; then
    if ! final=$(curl --proto '=https' --tlsv1.2 -fsSIL --connect-timeout 15 --max-time 60 --retry 2 \
      -o /dev/null -w '%{url_effective}' "$REPO_URL/releases/latest" 2>"$WORK/release.err"); then
      RELEASE_PROBLEM=unreachable
      return 1
    fi
    case "$final" in
      */releases/tag/*) tag=${final##*/releases/tag/} ;;
      */releases | */releases/)
        RELEASE_PROBLEM=none
        return 1
        ;;
    esac
  else
    if ! wget_get "https://api.github.com/repos/$REPO_SLUG/releases/latest" "$WORK/release.json"; then
      case "$(cat "$WORK/download.err" 2>/dev/null)" in
        *" 404"*) RELEASE_PROBLEM=none ;;
        *) RELEASE_PROBLEM=unreachable ;;
      esac
      return 1
    fi
    tag=$(awk -F'"' '$2 == "tag_name" { print $4; exit }' "$WORK/release.json") || tag=''
  fi
  if ! [[ $tag =~ $pattern ]]; then
    RELEASE_PROBLEM=unreachable
    return 1
  fi
  LATEST_TAG=$tag
  LATEST_VERSION=${tag#v}
}

is_stellarr_compose() {
  [ -f "$1" ] && grep -Eq '^name:[[:space:]]*stellarr[[:space:]]*$' "$1"
}

is_checkout() {
  [ -f "$1/Dockerfile" ] && is_stellarr_compose "$1/docker-compose.yml"
}

# The directory of this script when it is a file in a checkout: Stellarr's
# docker-compose.yml and Dockerfile are beside it. A script read from a pipe
# has none.
find_checkout() {
  local source=${BASH_SOURCE[0]:-} dir
  SCRIPT_CHECKOUT=''
  case "${source##*/}" in '' | bash | sh | -bash) return 0 ;; esac
  [ -f "$source" ] || return 0
  case "$source" in */*) dir=${source%/*} ;; *) dir=. ;; esac
  dir=$(cd -P -- "$dir" 2>/dev/null && pwd -P) || return 0
  is_checkout "$dir" || return 0
  SCRIPT_CHECKOUT=$dir
}

# What Docker knows of the container named stellarr. Compose labels it with
# its project and the directory it was started from.
find_container() {
  local info='' running project directory version
  local format=$'{{.State.Running}}\n{{index .Config.Labels "com.docker.compose.project"}}\n{{index .Config.Labels "com.docker.compose.project.working_dir"}}\n{{index .Config.Labels "org.opencontainers.image.version"}}'
  CONTAINER_EXISTS=0 CONTAINER_RUNNING=0 CONTAINER_PROJECT='' CONTAINER_DIR='' CONTAINER_VERSION=''
  info=$(dk container inspect --format "$format" stellarr 2>/dev/null) || return 0
  {
    IFS= read -r running || :
    IFS= read -r project || :
    IFS= read -r directory || :
    IFS= read -r version || :
  } <<EOF
$info
EOF
  CONTAINER_EXISTS=1
  [ "$running" != true ] || CONTAINER_RUNNING=1
  [ "$project" = '<no value>' ] || CONTAINER_PROJECT=$project
  [ "$directory" = '<no value>' ] || CONTAINER_DIR=$directory
  [ "$version" = '<no value>' ] || CONTAINER_VERSION=$version
}

# Sets ABSOLUTE to $1 as an absolute path without symbolic links, as far as
# the directories in it exist, so two names of one directory compare equal.
# A leading ~ is the home directory, as --dir=~/x reaches the installer
# without the shell expanding it.
absolute_path() {
  local path=$1 tilde=\~ existing rest=''
  case "$path" in
    "$tilde") path=$HOME ;;
    "$tilde"/*) path=$HOME/${path#"$tilde"/} ;;
  esac
  case "$path" in /*) ;; *) path=$PWD/$path ;; esac
  while :; do
    case "$path" in *//*) path=${path//\/\//\/} ;; *) break ;; esac
  done
  [ "$path" = / ] || path=${path%/}
  existing=$path
  while [ ! -d "$existing" ]; do
    rest="/${existing##*/}$rest"
    existing=${existing%/*}
    [ -n "$existing" ] || existing=/
  done
  existing=$(cd -P -- "$existing" && pwd -P)
  ABSOLUTE=${existing%/}$rest
  [ -n "$ABSOLUTE" ] || ABSOLUTE=/
}

# The installation's directory, in this order: --dir, the checkout this
# script is in, the directory of the installation Docker runs, the current
# directory when it holds Stellarr's docker-compose.yml, and ~/stellarr.
choose_directory() {
  local chosen running_dir=''
  if [ -n "$DIR_OPTION" ]; then
    absolute_path "$DIR_OPTION"
    chosen=$ABSOLUTE
  elif [ -n "$SCRIPT_CHECKOUT" ]; then
    chosen=$SCRIPT_CHECKOUT
  elif [ "$CONTAINER_EXISTS" -eq 1 ] && [ -n "$CONTAINER_DIR" ]; then
    absolute_path "$CONTAINER_DIR"
    chosen=$ABSOLUTE
  elif is_stellarr_compose "$PWD/docker-compose.yml"; then
    absolute_path "$PWD"
    chosen=$ABSOLUTE
  else
    [ -n "${HOME:-}" ] || die "HOME is not set, so there is no default directory" "Choose one with --dir."
    absolute_path "$HOME/stellarr"
    chosen=$ABSOLUTE
  fi
  INSTALL_DIR=$chosen
  OWNS_CONTAINER=0

  if [ "$CONTAINER_EXISTS" -eq 1 ]; then
    if [ -z "$CONTAINER_PROJECT" ] || [ -z "$CONTAINER_DIR" ]; then
      die "A container named stellarr exists, but Docker Compose did not start it" \
        "The installer manages installations made with Docker Compose only. To" \
        "update this one, follow $DOCS_URL/install/upgrading"
    fi
    if [ "$CONTAINER_PROJECT" != stellarr ]; then
      die "A container named stellarr belongs to the Compose project \"$CONTAINER_PROJECT\"" \
        "This installer manages the project named stellarr only. To update this one," \
        "follow $DOCS_URL/install/upgrading"
    fi
    if [ -d "$CONTAINER_DIR" ]; then
      running_dir=$(cd -P -- "$CONTAINER_DIR" && pwd -P)
    fi
    if [ -n "$running_dir" ] && [ "$running_dir" = "$INSTALL_DIR" ]; then
      OWNS_CONTAINER=1
    elif [ -n "$running_dir" ]; then
      # Compose's project name is fixed, so starting it from a second
      # directory would recreate the running container with that
      # directory's settings.
      die "Stellarr is installed in $running_dir, not in $INSTALL_DIR" \
        "Docker Compose would recreate its container with the settings in the" \
        "other directory. To update the installation, run the installer again with" \
        "  --dir $(shell_word "$running_dir")"
    else
      ui_warn "The container named stellarr was started from $CONTAINER_DIR," "which no longer exists"
      if [ "$FORCE" -eq 0 ]; then
        if [ "$INTERACTIVE" -eq 0 ] || ! ui_confirm n "Take the container over from $INSTALL_DIR?" \
          "Compose recreates it with the settings there. Its data volume stays."; then
          die "The container's directory is gone, so the installer stopped" \
            "To take the container over from $INSTALL_DIR, run the installer again" \
            "with --force."
        fi
      fi
      OWNS_CONTAINER=1
    fi
  fi

  # The installer replaces docker-compose.yml on an update, which must never
  # happen to another project's.
  if [ -f "$INSTALL_DIR/docker-compose.yml" ] && ! is_stellarr_compose "$INSTALL_DIR/docker-compose.yml"; then
    die "$INSTALL_DIR holds a docker-compose.yml that is not Stellarr's" \
      "Choose another directory with --dir."
  fi
  if is_checkout "$INSTALL_DIR"; then DIR_IS_CHECKOUT=1; fi
  if [ "$BUILD" -eq 1 ] && [ "$DIR_IS_CHECKOUT" -eq 0 ]; then
    die "--build builds the image from a checkout, and $INSTALL_DIR is not one" \
      "Clone the repository and run the installer there:" \
      "  git clone $REPO_URL.git" "  cd Stellarr && ./install.sh --build"
  fi
  read_existing_env
}

# Reads the .env already in the directory. Values may be quoted, end in a
# carriage return or follow "export"; the last assignment of a key wins, as
# in Compose.
read_existing_env() {
  local key value line keys="$MANAGED_KEYS STELLARR_VERSION STELLARR_TURN_RELAY_RANGE COMPOSE_PROJECT_NAME COMPOSE_FILE COMPOSE_PROFILES"
  ENV_STATE=none
  for key in $keys; do
    printf -v "EXIST_$key" '%s' ''
    printf -v "HAS_$key" '%s' 0
  done
  [ -f "$INSTALL_DIR/.env" ] || return 0
  [ -r "$INSTALL_DIR/.env" ] || die "$INSTALL_DIR/.env cannot be read by this user" \
    "Run the installer as the user that owns the installation."
  ENV_STATE=foreign
  IFS= read -r line <"$INSTALL_DIR/.env" || :
  case "${line%$'\r'}" in "${ENV_MARKER%%,*}"*) ENV_STATE=marked ;; esac
  while IFS= read -r line || [ -n "$line" ]; do
    key=${line%%=*} value=${line#*=}
    printf -v "EXIST_$key" '%s' "$value"
    printf -v "HAS_$key" '%s' 1
  done <<EOF
$(env_values "$INSTALL_DIR/.env" "$keys")
EOF
}

# Prints KEY=value for the effective assignment in file $1 of each key in $2,
# unquoted.
env_values() {
  ENV_KEYS=$2 awk '
    BEGIN { n = split(ENVIRON["ENV_KEYS"], k, " "); for (i = 1; i <= n; i++) want[k[i]] = 1 }
    {
      line = $0
      sub(/\r$/, "", line)
      if (!match(line, /^[ \t]*(export[ \t]+)?[A-Za-z_][A-Za-z0-9_]*[ \t]*=/)) next
      name = substr(line, 1, RLENGTH - 1)
      sub(/^[ \t]*(export[ \t]+)?/, "", name)
      sub(/[ \t]+$/, "", name)
      if (!(name in want)) next
      v = substr(line, RLENGTH + 1)
      sub(/^[ \t]+/, "", v)
      q = substr(v, 1, 1)
      if (q == "\"" || q == "\047") {
        v = substr(v, 2)
        stop = index(v, q)
        if (stop) v = substr(v, 1, stop - 1)
      } else {
        sub(/[ \t]+#.*$/, "", v)
        sub(/[ \t]+$/, "", v)
      }
      value[name] = v
      seen[name] = 1
    }
    END { for (i = 1; i <= n; i++) if (seen[k[i]]) print k[i] "=" value[k[i]] }
  ' "$1"
}

# Which release to install and where docker-compose.yml comes from. The
# file, .env.example and the image must be of the same release.
choose_release() {
  local existing_tag=$EXIST_STELLARR_VERSION required=0 wanted=0
  TARGET_TAG='' TARGET_VERSION='' FILES_REF='' TARGET_KEPT=''
  LATEST_TAG='' LATEST_VERSION=''

  if [ -n "$VERSION_OPTION" ]; then
    TARGET_TAG=$VERSION_OPTION
  elif [ "$BUILD" -eq 1 ]; then
    TARGET_TAG=${existing_tag:-latest}
  elif [ "$ENV_STATE" != none ] && is_tag "$existing_tag" && ! is_exact_version "$existing_tag"; then
    # A moving tag such as latest stays; pulling it again is the update.
    TARGET_TAG=$existing_tag TARGET_KEPT=moving
  fi
  # The latest release decides the image when nothing else does, and the
  # files when they are downloaded for a tag that names no release.
  [ -n "$TARGET_TAG" ] || required=1
  if [ "$DIR_IS_CHECKOUT" -eq 0 ] && [ -z "$SCRIPT_CHECKOUT" ] && [ -z "$REF_OPTION" ] && ! is_exact_version "$TARGET_TAG"; then
    required=1
  fi
  # Without a checkout the files are downloaded, so a missing curl and wget
  # stops the installer here, with the other checks.
  if [ "$DIR_IS_CHECKOUT" -eq 0 ] && [ -z "$SCRIPT_CHECKOUT" ]; then pick_downloader; fi
  # For latest it only names the version on screen.
  wanted=$required
  if [ "$TARGET_TAG" = latest ] && [ "$BUILD" -eq 0 ]; then wanted=1; fi
  if [ "$wanted" -eq 1 ]; then
    pick_downloader
    if ! find_latest_release && [ "$required" -eq 1 ]; then release_unavailable; fi
  fi
  if [ -z "$TARGET_TAG" ]; then
    TARGET_TAG=$LATEST_VERSION
    # An update never goes back: a newer release pinned in .env stays.
    if [ "$ENV_STATE" != none ] && is_exact_version "$existing_tag" && version_older "$LATEST_VERSION" "$existing_tag"; then
      TARGET_TAG=$existing_tag TARGET_KEPT=newer
    fi
  fi

  INSTALLED_VERSION=''
  if [ "$OWNS_CONTAINER" -eq 1 ]; then
    INSTALLED_VERSION=${CONTAINER_VERSION:-$existing_tag}
  elif [ "$ENV_STATE" != none ]; then
    INSTALLED_VERSION=$existing_tag
  fi
  # Starting an older image on data a newer one has used is not supported;
  # going back is a restore of the backup made before the update.
  if is_exact_version "$TARGET_TAG" && is_exact_version "$INSTALLED_VERSION" &&
    version_older "$TARGET_TAG" "$INSTALLED_VERSION"; then
    die "Stellarr $INSTALLED_VERSION is installed, and $TARGET_TAG is older" \
      "Going back to an older release means restoring the backup made before the" \
      "update: $DOCS_URL/install/upgrading#rollback"
  fi

  TARGET_VERSION=$TARGET_TAG
  if [ "$TARGET_TAG" = latest ] && [ -n "$LATEST_VERSION" ] && [ "$BUILD" -eq 0 ]; then
    TARGET_VERSION=$LATEST_VERSION
  fi
  if [ -n "$REF_OPTION" ]; then
    FILES_REF=$REF_OPTION
  elif is_exact_version "$TARGET_TAG"; then
    FILES_REF=v$TARGET_TAG
  else
    FILES_REF=$LATEST_TAG
  fi

  show_release
}

show_release() {
  local found=''
  if [ "$OWNS_CONTAINER" -eq 1 ]; then
    case "$CONTAINER_VERSION" in
      '') found="Stellarr" ;;
      dev) found="Stellarr built from source" ;;
      *) found="Stellarr $CONTAINER_VERSION" ;;
    esac
  elif [ "$ENV_STATE" = marked ]; then
    found="the settings of an installation"
  fi
  if [ -n "$found" ] && [ -n "$LATEST_VERSION" ] && [ "$BUILD" -eq 0 ] &&
    [ "$CONTAINER_VERSION" = "$LATEST_VERSION" ] && [ "$TARGET_VERSION" = "$LATEST_VERSION" ]; then
    ui_ok "$found in $INSTALL_DIR is the latest release"
    return 0
  fi
  [ -z "$found" ] || ui_ok "Found $found in $INSTALL_DIR"
  if [ "$BUILD" -eq 1 ]; then
    ui_ok "The image will be built from the checkout"
  elif [ -n "$VERSION_OPTION" ] && [ "$TARGET_TAG" = latest ] && [ -n "$LATEST_VERSION" ]; then
    ui_ok "Latest release is $LATEST_VERSION, as $VERSION_FROM asks"
  elif [ -n "$VERSION_OPTION" ]; then
    ui_ok "Version $TARGET_TAG, as $VERSION_FROM asks"
  elif [ "$TARGET_KEPT" = newer ]; then
    ui_ok "Latest release is $LATEST_VERSION; $TARGET_TAG, which is newer, stays"
  elif [ "$TARGET_KEPT" = moving ] && [ -n "$LATEST_VERSION" ]; then
    ui_ok "Latest release is $LATEST_VERSION; .env follows the tag $TARGET_TAG"
  elif [ "$TARGET_KEPT" = moving ]; then
    ui_ok ".env follows the tag $TARGET_TAG"
  else
    ui_ok "Latest release is $LATEST_VERSION"
  fi
}

release_unavailable() {
  if [ "$RELEASE_PROBLEM" = none ]; then
    if [ -n "$SCRIPT_CHECKOUT" ] || [ "$DIR_IS_CHECKOUT" -eq 1 ]; then
      die "Stellarr has no published release yet, so there is no image to pull." \
        "To build it from this checkout instead:" "  ./install.sh --build"
    fi
    die "Stellarr has no published release yet, so there is no image to pull." \
      "To build it from the source instead:" "  git clone $REPO_URL.git" "  cd Stellarr && ./install.sh --build"
  fi
  die "GitHub did not answer, so the latest release is unknown" \
    "Check that this machine reaches github.com and run the installer again," \
    "or name the release with --version."
}

# What happens to .env, decided before anything is written: write a new one,
# keep one the installer wrote, adopt one written by hand, or replace it.
decide_env() {
  ENV_ACTION=new
  case "$ENV_STATE" in
    none) return 0 ;;
    marked) ENV_ACTION=keep ;;
    foreign) ENV_ACTION=adopt ;;
  esac
  if [ "$FORCE" -eq 1 ]; then
    ENV_ACTION=replace
    return 0
  fi
  [ "$ENV_STATE" = foreign ] || return 0
  # These change which project and files Compose uses, which would start the
  # installation on another, empty volume or with services it does not have.
  if [ -n "$EXIST_COMPOSE_PROJECT_NAME$EXIST_COMPOSE_FILE$EXIST_COMPOSE_PROFILES" ]; then
    die "$INSTALL_DIR/.env sets COMPOSE_PROJECT_NAME, COMPOSE_FILE or COMPOSE_PROFILES" \
      "It looks like a development stack's .env, which the installer must not run" \
      "the production stack with. Choose another directory with --dir, or add" \
      "--force to keep it as a backup and write a new one."
  fi
  if [ "$INTERACTIVE" -eq 1 ] && ! ui_confirm y "Keep the settings in the existing .env?" \
    "The installer did not write $INSTALL_DIR/.env. Keeping it marks it as the" \
    "installer's, which then changes only what it is given. Otherwise it is kept" \
    "as a backup and a new one is written."; then
    ENV_ACTION=replace
  fi
}

# The settings this run uses: a given value, else the one in .env, else the
# answer to a question, else the default.
settle_settings() {
  local key given value has exist
  for key in $MANAGED_KEYS; do
    given="GIVEN_$key" value="VALUE_$key" has="HAS_$key" exist="EXIST_$key"
    if [ "${!given}" -eq 1 ]; then
      printf -v "SET_$key" '%s' "${!value}"
    elif [ "$ENV_ACTION" = keep ] || [ "$ENV_ACTION" = adopt ]; then
      printf -v "SET_$key" '%s' "${!exist}"
    else
      printf -v "SET_$key" '%s' ''
    fi
  done

  if [ "$ENV_ACTION" = new ] || [ "$ENV_ACTION" = replace ]; then
    ask_questions
  fi
  : "${SET_STELLARR_DOMAIN:=localhost}"
  : "${SET_STELLARR_LANGUAGE:=en}"
  : "${SET_STELLARR_HTTPS_PORT:=8443}" "${SET_STELLARR_HTTP_PORT:=8888}" "${SET_STELLARR_MEDIA_PORT:=49000}"
  : "${SET_STELLARR_TURN_PORT:=3478}" "${SET_STELLARR_TURNS_PORT:=5349}"
  RELAY_RANGE=${EXIST_STELLARR_TURN_RELAY_RANGE:-49160-49200}
  [ "$ENV_ACTION" = keep ] || [ "$ENV_ACTION" = adopt ] || RELAY_RANGE=49160-49200

  for key in STELLARR_HTTPS_PORT STELLARR_PUBLIC_PORT STELLARR_HTTP_PORT STELLARR_MEDIA_PORT STELLARR_TURN_PORT STELLARR_TURNS_PORT; do
    value="SET_$key"
    [ -n "${!value}" ] || continue
    key_option "$key"
    is_port "${!value}" || die "$key in $INSTALL_DIR/.env is not a port: \"${!value}\"" \
      "Correct it there, or set it with $KEY_OPTION."
  done
  # The entrypoint resolves auto from the domain, so it needs a domain. A
  # value .env had before is the person's own and stays as it is.
  if [ "$SET_PUBLIC_IP" = auto ] && { ! is_hostname "$SET_STELLARR_DOMAIN" || is_local_address "$SET_STELLARR_DOMAIN"; } &&
    { [ "$GIVEN_PUBLIC_IP" -eq 1 ] || [ "$GIVEN_STELLARR_DOMAIN" -eq 1 ]; }; then
    usage_error "PUBLIC_IP=auto needs a domain to resolve, and $SET_STELLARR_DOMAIN is none"
  fi
  check_port_overlap
}

# Two settings on one port would only fail later, inside Docker.
check_port_overlap() {
  local tcp=" $SET_STELLARR_HTTPS_PORT " udp=' ' key port value first last
  for key in STELLARR_HTTP_PORT STELLARR_TURN_PORT STELLARR_TURNS_PORT; do
    value="SET_$key" port=${!value}
    case "$tcp" in *" $port "*) port_overlap "$port" ;; esac
    tcp="$tcp$port "
  done
  for key in STELLARR_TURN_PORT STELLARR_TURNS_PORT STELLARR_MEDIA_PORT; do
    value="SET_$key" port=${!value}
    case "$udp" in *" $port "*) port_overlap "$port" ;; esac
    udp="$udp$port "
    if relay_bounds; then
      first=$RELAY_FIRST last=$RELAY_LAST
      [ "$port" -lt "$first" ] || [ "$port" -gt "$last" ] || port_overlap "$port"
    fi
  done
}

port_overlap() {
  usage_error "Two of Stellarr's ports are set to $1; each needs its own"
}

relay_bounds() {
  RELAY_FIRST=${RELAY_RANGE%-*} RELAY_LAST=${RELAY_RANGE#*-}
  is_port "$RELAY_FIRST" && is_port "$RELAY_LAST" && [ "$RELAY_FIRST" -le "$RELAY_LAST" ]
}

# Up to three questions, for a new installation in a terminal; a value given
# as an option is not asked for.
ask_questions() {
  local default server=''
  if [ "$GIVEN_STELLARR_DOMAIN" -eq 0 ]; then
    SET_STELLARR_DOMAIN=localhost
    if [ "$INTERACTIVE" -eq 1 ]; then
      # Over SSH, the address this machine was reached at is the likely one.
      default=localhost
      read -r _ _ server _ <<EOF || :
${SSH_CONNECTION:-}
EOF
      if is_ipv4 "${server:-}"; then default=$server; fi
      ui_question "Where will people open Stellarr?" \
        "A domain or an IP address. \"localhost\" works on this machine only."
      while :; do
        ui_prompt "$default"
        normalize_address "${UI_ANSWER:-$default}"
        if is_address "$ADDRESS"; then break; fi
        ui_retry "\"$UI_ANSWER\" is not a domain or an IPv4 address"
      done
      SET_STELLARR_DOMAIN=$ADDRESS
    fi
  fi

  if [ "$GIVEN_PUBLIC_IP" -eq 0 ]; then
    if is_local_address "$SET_STELLARR_DOMAIN"; then
      SET_PUBLIC_IP=''
    elif is_ipv4 "$SET_STELLARR_DOMAIN"; then
      SET_PUBLIC_IP=$SET_STELLARR_DOMAIN
    elif [ "$INTERACTIVE" -eq 1 ]; then
      ui_question "Which IPv4 address should voice calls use?" \
        "Other devices send their audio here. \"auto\" reads the DNS record of" \
        "$SET_STELLARR_DOMAIN. \"none\" leaves it empty to decide later."
      while :; do
        ui_prompt auto
        case "${UI_ANSWER:-auto}" in
          auto) SET_PUBLIC_IP=auto ;;
          none | -) SET_PUBLIC_IP='' ;;
          *)
            if ! is_ipv4 "$UI_ANSWER"; then
              ui_retry "\"$UI_ANSWER\" is not an IPv4 address"
              continue
            fi
            SET_PUBLIC_IP=$UI_ANSWER
            ;;
        esac
        break
      done
    else
      SET_PUBLIC_IP=''
    fi
  fi

  if [ "$GIVEN_STELLARR_LANGUAGE" -eq 0 ]; then
    SET_STELLARR_LANGUAGE=en
    if [ "$INTERACTIVE" -eq 1 ]; then
      default=en
      case "$LOCALE_LANGUAGE" in pl*) default=pl ;; esac
      ui_question "Interface language, en or pl"
      while :; do
        ui_prompt "$default"
        case "${UI_ANSWER:-$default}" in
          en | pl) break ;;
        esac
        ui_retry "Answer en or pl"
      done
      SET_STELLARR_LANGUAGE=${UI_ANSWER:-$default}
    fi
  fi
}

# An update asks first in a terminal. Without one it needs --yes, as nothing
# may be confirmed on the person's behalf.
confirm_update() {
  local from=$INSTALLED_VERSION
  UPDATING=0
  [ "$BUILD" -eq 0 ] || return 0
  [ "$INSTALLED_VERSION" != "$TARGET_VERSION" ] || return 0
  # A moving tag in .env names no version to update from.
  if [ "$INSTALLED_VERSION" = dev ]; then
    from="a build of the source"
  elif ! is_exact_version "$INSTALLED_VERSION"; then
    return 0
  fi
  UPDATING=1
  if [ "$ASSUME_YES" -eq 1 ]; then return 0; fi
  if [ "$INTERACTIVE" -eq 0 ]; then
    die "Updating Stellarr from $from to $TARGET_VERSION needs a confirmation" \
      "Run the installer in a terminal, or again with --yes."
  fi
  if ! ui_confirm y "Update Stellarr from $from to $TARGET_VERSION?" \
    "Settings and data stay. If you have no backup yet, make one first:" \
    "$DOCS_URL/install/upgrading"; then
    ui_text '' "Nothing was changed."
    exit 0
  fi
}

prepare_directory() {
  if [ "$ENV_ACTION" = new ] && [ "$OWNS_CONTAINER" -eq 0 ]; then
    ui_section "Installing into $INSTALL_DIR"
  elif [ "$UPDATING" -eq 1 ]; then
    ui_section "Updating $INSTALL_DIR"
  else
    ui_section "Using $INSTALL_DIR"
  fi
  mkdir -p -- "$INSTALL_DIR" 2>/dev/null || die "Could not create $INSTALL_DIR" "Choose another directory with --dir."
  cd -P -- "$INSTALL_DIR" || die "Could not enter $INSTALL_DIR"
  INSTALL_DIR=$(pwd -P)
  [ -w . ] || die "This user cannot write to $INSTALL_DIR" \
    "Run the installer as a user who can, or choose another directory with --dir."
  place_compose_files
}

place_compose_files() {
  local label
  if [ "$DIR_IS_CHECKOUT" -eq 1 ]; then
    ui_ok "Using docker-compose.yml from the checkout"
    return 0
  fi
  if [ -n "$SCRIPT_CHECKOUT" ]; then
    cp "$SCRIPT_CHECKOUT/docker-compose.yml" "$WORK/docker-compose.yml"
    [ ! -f "$SCRIPT_CHECKOUT/.env.example" ] || cp "$SCRIPT_CHECKOUT/.env.example" "$WORK/env.example"
    label="from $SCRIPT_CHECKOUT"
  else
    label="for $FILES_REF"
    case "$FILES_REF" in v*) if is_exact_version "${FILES_REF#v}"; then label="for ${FILES_REF#v}"; fi ;; esac
    download "$RAW_URL/$FILES_REF/docker-compose.yml" "$WORK/docker-compose.yml" ||
      die "Could not download docker-compose.yml $label" "$RAW_URL/$FILES_REF/docker-compose.yml" "$DOWNLOAD_ERROR"
    is_stellarr_compose "$WORK/docker-compose.yml" ||
      die "The docker-compose.yml downloaded $label is not Stellarr's" "$RAW_URL/$FILES_REF/docker-compose.yml"
    download "$RAW_URL/$FILES_REF/.env.example" "$WORK/env.example" ||
      die "Could not download .env.example $label" "$RAW_URL/$FILES_REF/.env.example" "$DOWNLOAD_ERROR"
  fi

  install_file "$WORK/docker-compose.yml" docker-compose.yml backup
  case "$FILE_RESULT" in
    same) ui_ok "docker-compose.yml $label is in place" ;;
    new) ui_ok "Saved docker-compose.yml $label" ;;
    changed) ui_ok "Saved docker-compose.yml $label, previous kept as" "$FILE_BACKUP" ;;
  esac
  if [ -f "$WORK/env.example" ]; then install_file "$WORK/env.example" .env.example; fi
}

# Puts file $1 at $2 unless it already has the same content. With "backup" as
# $3, the file it replaces is kept beside it, named after the time.
install_file() {
  local source=$1 target=$2 temp
  FILE_BACKUP=''
  if [ -f "$target" ]; then
    if cmp -s "$source" "$target"; then
      FILE_RESULT=same
      return 0
    fi
    FILE_RESULT=changed
    if [ "${3:-}" = backup ]; then
      backup_file "$target"
      FILE_BACKUP=$BACKUP_NAME
    fi
  else
    FILE_RESULT=new
  fi
  temp=$(mktemp "$target.XXXXXX")
  cat "$source" >"$temp"
  chmod 644 "$temp"
  mv -f "$temp" "$target"
}

backup_file() {
  local name="$1.bak-$RUN_STAMP" n=1
  while [ -e "$name" ]; do
    name="$1.bak-$RUN_STAMP-$n"
    n=$((n + 1))
  done
  cp -p "$1" "$name"
  BACKUP_NAME=$name
}

write_env() {
  case "$ENV_ACTION" in
    new)
      env_write_new
      ui_ok "Wrote .env"
      ;;
    replace)
      backup_file .env
      ENV_BACKED_UP=1
      env_write_new
      ui_ok "Wrote a new .env, previous kept as" "$BACKUP_NAME"
      ;;
    keep | adopt)
      env_changes
      if [ "$ENV_ACTION" = adopt ]; then
        env_backup_once
        env_rewrite .env marker
        ui_ok "Kept the settings in .env and marked it as the installer's"
      fi
      if [ "${#ENV_CHANGES[@]}" -gt 0 ]; then
        env_set "${ENV_CHANGES[@]}"
      elif [ "$ENV_ACTION" = keep ]; then
        ui_ok "Kept .env as it is"
      fi
      ;;
  esac
}

# The KEY=value changes an existing .env needs: what this run was given, and
# the release on an update. A value it already has is no change.
env_changes() {
  local key given set has exist
  ENV_CHANGES=()
  for key in $MANAGED_KEYS; do
    given="GIVEN_$key" set="SET_$key" has="HAS_$key" exist="EXIST_$key"
    [ "${!given}" -eq 1 ] || continue
    [ "${!has}" -eq 0 ] || [ "${!exist}" != "${!set}" ] || continue
    ENV_CHANGES+=("$key=${!set}")
  done
  if [ "$HAS_STELLARR_VERSION" -eq 0 ] || [ "$EXIST_STELLARR_VERSION" != "$TARGET_TAG" ]; then
    ENV_CHANGES+=("STELLARR_VERSION=$TARGET_TAG")
  fi
}

env_backup_once() {
  [ "$ENV_CREATED" -eq 0 ] && [ "$ENV_BACKED_UP" -eq 0 ] || return 0
  backup_file .env
  ENV_BACKED_UP=1
}

# Sets each KEY=value argument in .env, in place: the effective assignment
# of a key is rewritten, and a key it lacks goes under the first line.
env_set() {
  local pair shown=''
  env_backup_once
  env_rewrite .env set "$@"
  for pair in "$@"; do shown="$shown, $pair"; done
  shown=${shown#, }
  if [ "$#" -eq 1 ]; then
    ui_ok "Set $shown in .env"
  else
    ui_ok "Set in .env:" "$@"
  fi
}

# Rewrites .env through a temporary file in the same directory, so the file
# is never half written. "marker" puts the installer's first line on top;
# "set" takes KEY=value arguments. They reach awk through its environment,
# where nothing in them is interpreted; the values have no line breaks.
env_rewrite() {
  local file=$1 mode=$2 temp old_umask pairs=''
  shift 2
  if [ "$#" -gt 0 ]; then pairs=$(printf '%s\n' "$@"); fi
  old_umask=$(umask)
  umask 077
  temp=$(mktemp "$file.XXXXXX")
  ENV_MODE=$mode ENV_MARKER=$ENV_MARKER ENV_PAIRS=$pairs awk '
    BEGIN {
      n = split(ENVIRON["ENV_PAIRS"], pair, "\n")
      for (i = 1; i <= n; i++) {
        eq = index(pair[i], "=")
        k[i] = substr(pair[i], 1, eq - 1)
        want[k[i]] = substr(pair[i], eq + 1)
      }
    }
    { line[NR] = $0 }
    END {
      if (ENVIRON["ENV_MODE"] == "marker") {
        print ENVIRON["ENV_MARKER"]
        for (r = 1; r <= NR; r++) print line[r]
        exit
      }
      for (r = 1; r <= NR; r++) {
        text = line[r]
        sub(/\r$/, "", text)
        if (!match(text, /^[ \t]*(export[ \t]+)?[A-Za-z_][A-Za-z0-9_]*[ \t]*=/)) continue
        name = substr(text, 1, RLENGTH - 1)
        sub(/^[ \t]*(export[ \t]+)?/, "", name)
        sub(/[ \t]+$/, "", name)
        last[name] = r
      }
      for (i = 1; i <= n; i++) if (k[i] in last) at[last[k[i]]] = k[i]
      for (r = 1; r <= NR; r++) {
        if (r in at) {
          cr = (line[r] ~ /\r$/) ? "\r" : ""
          print at[r] "=" want[at[r]] cr
        } else {
          print line[r]
        }
        if (r == 1) for (i = 1; i <= n; i++) if (!(k[i] in last)) print k[i] "=" want[k[i]]
      }
    }
  ' "$file" >"$temp"
  umask "$old_umask"
  chmod 600 "$temp"
  mv -f "$temp" "$file"
}

# A new .env: the installer's line, its settings, and .env.example of the
# same release below them for reference. An assignment there of a setting
# above is commented out, as it would override it.
env_write_new() {
  local temp old_umask key value
  old_umask=$(umask)
  umask 077
  temp=$(mktemp .env.XXXXXX)
  {
    printf '%s\n' "$ENV_MARKER"
    for key in STELLARR_DOMAIN PUBLIC_IP STELLARR_LANGUAGE; do
      value="SET_$key"
      printf '%s=%s\n' "$key" "${!value}"
    done
    printf 'STELLARR_VERSION=%s\n' "$TARGET_TAG"
    for key in STELLARR_HTTPS_PORT STELLARR_PUBLIC_PORT STELLARR_HTTP_PORT STELLARR_MEDIA_PORT STELLARR_TURN_PORT STELLARR_TURNS_PORT; do
      value="SET_$key"
      [ -z "${!value}" ] || printf '%s=%s\n' "$key" "${!value}"
    done
    if [ -f .env.example ]; then
      printf '\n# Below is .env.example of the same version as docker-compose.yml, for reference.\n'
      ENV_KEYS="$MANAGED_KEYS STELLARR_VERSION" awk '
        BEGIN { n = split(ENVIRON["ENV_KEYS"], k, " "); for (i = 1; i <= n; i++) ours[k[i]] = 1 }
        {
          text = $0
          sub(/\r$/, "", text)
          if (match(text, /^[ \t]*(export[ \t]+)?[A-Za-z_][A-Za-z0-9_]*[ \t]*=/)) {
            name = substr(text, 1, RLENGTH - 1)
            sub(/^[ \t]*(export[ \t]+)?/, "", name)
            sub(/[ \t]+$/, "", name)
            if (name in ours) { print "# " $0; next }
          }
          print
        }
      ' .env.example
    fi
  } >"$temp"
  umask "$old_umask"
  chmod 600 "$temp"
  mv -f "$temp" .env
  ENV_CREATED=1
}

# Compose takes a variable from the environment over the one in .env, and
# COMPOSE_PROJECT_NAME over the project's own name, which would start the
# installation on another, empty volume. The installer runs Compose on .env
# alone, as a later "docker compose up" from another shell would; what it was
# given in the environment is in .env by now.
clear_compose_environment() {
  local name names ignored=''
  unset COMPOSE_PROJECT_NAME COMPOSE_FILE COMPOSE_PROFILES COMPOSE_ENV_FILES COMPOSE_PATH_SEPARATOR
  names=$(grep -oE '\$\{[A-Za-z_][A-Za-z0-9_]*' docker-compose.yml | sed 's/^..//' | sort -u) || names=''
  for name in $names; do
    [ -n "${!name+set}" ] || continue
    case " $MANAGED_KEYS STELLARR_VERSION " in
      *" $name "*) ;;
      *) ignored="$ignored $name" ;;
    esac
    unset "$name"
  done
  [ -z "$ignored" ] || ui_warn "Ignored from the environment, as Compose reads .env:$ignored"
}

# Fills LISTEN_TCP and LISTEN_UDP with the ports something listens on, from
# one listing per protocol. Returns 1 when none of ss, lsof and netstat is
# there to list them, or the one there fails.
list_listening() {
  local tcp udp tool
  if have ss; then
    tool=ss
    tcp=$(ss -ltn 2>/dev/null) || return 1
    udp=$(ss -lun 2>/dev/null) || return 1
  elif have lsof; then
    # lsof finds nothing and fails when nothing listens, which is no error.
    tcp=$(lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null) || :
    udp=$(lsof -nP -iUDP 2>/dev/null) || :
    tool=lsof
  elif have netstat; then
    tool=netstat
    tcp=$(netstat -lnt 2>/dev/null) || return 1
    udp=$(netstat -lnu 2>/dev/null) || return 1
  else
    return 1
  fi
  LISTEN_TCP=" $(port_numbers "$tool" "$tcp") "
  LISTEN_UDP=" $(port_numbers "$tool" "$udp") "
}

# The local port of each line of tool $1's listing $2: the last part of the
# address, after a colon or, in BSD's netstat, a dot. ss and netstat show the
# address in the fourth column, lsof after the protocol.
port_numbers() {
  printf '%s\n' "$2" | awk -v tool="$1" '
    {
      a = $4
      if (tool == "lsof") {
        a = ""
        for (i = 1; i < NF; i++) if ($i == "TCP" || $i == "UDP") { a = $(i + 1); break }
        sub(/->.*/, "", a)
      }
      n = split(a, p, /[:.]/)
      if (n > 1 && p[n] ~ /^[0-9]+$/) printf "%s ", p[n]
    }'
}

# Whether port $1 is in use for any of the protocols in $2.
port_busy() {
  local port=$1 proto
  for proto in $2; do
    case "$proto" in
      tcp) case "$LISTEN_TCP" in *" $port "*) return 0 ;; esac ;;
      udp) case "$LISTEN_UDP" in *" $port "*) return 0 ;; esac ;;
    esac
  done
  return 1
}

# The role of each port setting: its name on screen and its protocols.
port_role() {
  case "$1" in
    STELLARR_HTTPS_PORT) ROLE_NAME=HTTPS ROLE_PROTOS=tcp ;;
    STELLARR_HTTP_PORT) ROLE_NAME=HTTP ROLE_PROTOS=tcp ;;
    STELLARR_TURN_PORT) ROLE_NAME=TURN ROLE_PROTOS='tcp udp' ;;
    STELLARR_TURNS_PORT) ROLE_NAME='TURN over TLS' ROLE_PROTOS='tcp udp' ;;
    STELLARR_MEDIA_PORT) ROLE_NAME='voice media' ROLE_PROTOS=udp ;;
  esac
}

port_list_text() {
  PORTS_TEXT="$SET_STELLARR_HTTPS_PORT, $SET_STELLARR_HTTP_PORT, $SET_STELLARR_TURN_PORT, $SET_STELLARR_TURNS_PORT, $SET_STELLARR_MEDIA_PORT, $RELAY_RANGE"
}

# The ports Docker will publish must be free, unless Stellarr's own
# container holds them. This runs before any file is written, so a busy port
# leaves nothing behind. In a terminal a busy port that was not chosen gets
# the next free one; otherwise the installer stops and says what moves it.
# The result is shown with the files, by show_ports.
check_ports() {
  local key value given port busy='' relay_busy=0 lines=() suggestion
  PORTS_RESULT=free
  if [ "$SKIP_PORT_CHECK" -eq 1 ]; then
    PORTS_RESULT=skipped
    return 0
  fi
  if [ "$OWNS_CONTAINER" -eq 1 ] && [ "$CONTAINER_RUNNING" -eq 1 ]; then
    PORTS_RESULT=held
    return 0
  fi
  if ! list_listening; then
    PORTS_RESULT=unknown
    return 0
  fi

  for key in STELLARR_HTTPS_PORT STELLARR_HTTP_PORT STELLARR_TURN_PORT STELLARR_TURNS_PORT STELLARR_MEDIA_PORT; do
    value="SET_$key"
    port_role "$key"
    if port_busy "${!value}" "$ROLE_PROTOS"; then busy="$busy $key"; fi
  done
  if relay_bounds; then
    port=$RELAY_FIRST
    while [ "$port" -le "$RELAY_LAST" ]; do
      if port_busy "$port" udp; then
        relay_busy=1
        break
      fi
      port=$((port + 1))
    done
  fi
  if [ -z "$busy" ] && [ "$relay_busy" -eq 0 ]; then return 0; fi

  if [ "$INTERACTIVE" -eq 1 ] && [ "$relay_busy" -eq 0 ] && ! busy_includes_given "$busy"; then
    for key in $busy; do
      value="SET_$key"
      port_role "$key"
      ui_warn "Port ${!value} ($ROLE_NAME) is in use on this machine"
      next_free_port "$key"
      suggestion=$FREE_PORT
      ui_question "Which port should $ROLE_NAME use?"
      while :; do
        ui_prompt "$suggestion"
        port=${UI_ANSWER:-$suggestion}
        if ! is_port "$port"; then
          ui_retry "\"$port\" is not a port from 1 to 65535"
        elif ! port_is_available "$key" "$port"; then
          ui_retry "Port $port is in use, or another of Stellarr's ports has it"
        else
          break
        fi
      done
      # A port chosen here is the person's own choice, written to .env.
      printf -v "SET_$key" '%s' "$port"
      printf -v "GIVEN_$key" '%s' 1
    done
    return 0
  fi

  for key in $busy; do
    value="SET_$key" given="GIVEN_$key"
    port_role "$key"
    key_option "$key"
    if [ "${!given}" -eq 1 ]; then
      lines+=("  ${!value} ($ROLE_NAME), as given: choose another with $KEY_OPTION")
    else
      lines+=("  ${!value} ($ROLE_NAME): move it with $KEY_OPTION")
    fi
  done
  # The container maps the range one to one, so no option moves it.
  if [ "$relay_busy" -eq 1 ]; then
    lines+=("  $RELAY_RANGE (TURN relay): set STELLARR_TURN_RELAY_RANGE in .env to" \
      "  another free range of $((RELAY_LAST - RELAY_FIRST + 1)) ports, as described in" "  $DOCS_URL/rtc-and-ports")
  fi
  die "Ports Stellarr needs are in use on this machine" "${lines[@]}" '' \
    "Free them, or run the installer again with the changes above."
}

show_ports() {
  port_list_text
  case "$PORTS_RESULT" in
    free) ui_ok "Ports are free: $PORTS_TEXT" ;;
    held) ui_ok "The ports are held by Stellarr's running container" ;;
    skipped) ui_warn "Did not check the ports, as asked" ;;
    unknown) ui_warn "Could not check the ports: no ss, lsof or netstat that works" ;;
  esac
}

busy_includes_given() {
  local key given
  for key in $1; do
    given="GIVEN_$key"
    [ "${!given}" -eq 0 ] || return 0
  done
  return 1
}

# Whether the port setting $1 can take port $2: free on this machine for its
# protocols, and not one of Stellarr's other ports.
port_is_available() {
  local key=$1 port=$2 other value
  port_role "$key"
  if port_busy "$port" "$ROLE_PROTOS"; then return 1; fi
  for other in STELLARR_HTTPS_PORT STELLARR_HTTP_PORT STELLARR_TURN_PORT STELLARR_TURNS_PORT STELLARR_MEDIA_PORT; do
    [ "$other" != "$key" ] || continue
    value="SET_$other"
    [ "${!value}" != "$port" ] || return 1
  done
  if relay_bounds && [ "$port" -ge "$RELAY_FIRST" ] && [ "$port" -le "$RELAY_LAST" ]; then return 1; fi
  return 0
}

next_free_port() {
  local value="SET_$1" port
  port=${!value}
  FREE_PORT=''
  while [ "$port" -lt 65535 ]; do
    port=$((port + 1))
    if port_is_available "$1" "$port"; then
      FREE_PORT=$port
      return 0
    fi
  done
}

start_stellarr() {
  local log="$WORK/compose.log"
  if ! compose config -q >"$log" 2>&1; then
    ui_fail "Docker Compose cannot read the configuration in $INSTALL_DIR"
    ui_block <"$log" >&2
    exit 1
  fi
  [ "$START" -eq 1 ] || return 0
  if [ "$BUILD" -eq 1 ]; then
    ui_text "Building the image; Docker's output follows. This takes a while."
    compose build || die "Building the image failed" "Docker's output is above."
    ui_ok "Built the image from the checkout"
  else
    pull_image
  fi
  if ! compose up -d --no-build >"$log" 2>&1; then
    ui_fail "Docker Compose could not start the container"
    ui_block <"$log" >&2
    compose_command logs
    ui_text '' "Nothing was removed. When the cause is fixed, run the installer again." \
      "All logs: $COMPOSE_COMMAND" >&2
    exit 1
  fi
  if [ "$OWNS_CONTAINER" -eq 1 ] && [ "$CONTAINER_RUNNING" -eq 1 ]; then
    ui_ok "Brought the container up to date"
  else
    ui_ok "Started the container"
  fi
  wait_until_up
}

pull_image() {
  local image="$IMAGE:$TARGET_TAG" log="$WORK/pull.log" status=0
  if [ "$UI_TERMINAL" -eq 1 ]; then
    compose pull || status=$?
  else
    compose pull --quiet >"$log" 2>&1 || status=$?
  fi
  if [ "$status" -eq 0 ] && dk image inspect "$image" >/dev/null 2>&1; then
    ui_ok "Pulled $image"
    return 0
  fi
  if dk image inspect "$image" >/dev/null 2>&1; then
    ui_warn "Could not pull $image; using the copy already on this machine"
    return 0
  fi
  ui_fail "Could not pull $image"
  if [ "$UI_TERMINAL" -eq 1 ]; then
    ui_text '' "Docker's message is above." >&2
  else
    ui_block <"$log" >&2
  fi
  ui_text '' "Check that this machine reaches ghcr.io and that $TARGET_TAG is a release:" \
    "$REPO_URL/releases" >&2
  exit 1
}

# Waits until stellarr-init reports every service ready, checking once a
# second. Docker's own health status lags by up to 30 seconds, so the
# installer asks stellarr-init itself.
wait_until_up() {
  local started=$SECONDS elapsed checks=0 info state restarts base=''
  while :; do
    # Each check waits a second, so the time counts at least one per check,
    # even where the clock does not move.
    elapsed=$((SECONDS - started))
    [ "$elapsed" -ge "$checks" ] || elapsed=$checks
    info=$(dk container inspect --format '{{.State.Status}} {{.RestartCount}}' stellarr 2>/dev/null) || info=''
    state=${info%% *} restarts=${info##* }
    case "$restarts" in '' | *[!0-9]*) restarts=0 ;; esac
    [ -n "$base" ] || base=$restarts
    case "$state" in
      exited | dead) start_failed "The container stopped ($state) while Stellarr was starting" ;;
    esac
    if [ $((restarts - base)) -ge 2 ]; then
      start_failed "The container restarted $((restarts - base)) times while Stellarr was starting"
    fi
    if [ "$state" = running ] && dk exec stellarr stellarr-init health >/dev/null 2>&1; then
      if [ "$elapsed" -eq 0 ]; then ui_ok "Stellarr is up"; else ui_ok "Stellarr is up after $elapsed s"; fi
      return 0
    fi
    [ "$elapsed" -lt "$START_TIMEOUT" ] || start_failed "Stellarr did not start within $START_TIMEOUT s"
    ui_wait "$elapsed" "$START_TIMEOUT"
    sleep 1
    checks=$((checks + 1))
  done
}

start_failed() {
  ui_fail "$1"
  ui_section "What its services report"
  if dk exec stellarr stellarr-init status >"$WORK/status.log" 2>&1 || [ -s "$WORK/status.log" ]; then
    ui_block <"$WORK/status.log"
  else
    ui_text "Nothing: the container is not running."
  fi
  ui_section "Last lines of the container's log"
  dk logs --tail 40 stellarr >"$WORK/container.log" 2>&1 || :
  ui_block <"$WORK/container.log"
  compose_command logs
  ui_text '' "Nothing was removed. When the cause is fixed, run the installer again." \
    "All logs: $COMPOSE_COMMAND"
  exit 1
}

# Asks the instance whether it still waits for its owner: SETUP is yes, no
# or unknown. The certificate is not checked for this one request to the
# loopback address, where it is self-signed and names another host.
query_setup() {
  local url="https://127.0.0.1:$SET_STELLARR_HTTPS_PORT/api/auth/registration" body=''
  SETUP=unknown
  if have curl; then
    body=$(curl --proto '=https' -ksS --max-time 10 "$url" 2>/dev/null) || body=''
  elif have wget; then
    body=$(wget -q -T 10 --no-check-certificate -O - "$url" 2>/dev/null) || body=''
  fi
  body=${body//[[:space:]]/}
  case "$body" in
    *'"setupRequired":true'*) SETUP=yes ;;
    *'"setupRequired":false'*) SETUP=no ;;
  esac
}

# The newest setup code in the container's log since it last started; each
# start draws a new one.
find_setup_code() {
  local started logs since=()
  SETUP_CODE=''
  started=$(dk container inspect --format '{{.State.StartedAt}}' stellarr 2>/dev/null) || started=''
  [ -z "$started" ] || since=(--since "$started")
  logs=$(dk logs ${since[@]+"${since[@]}"} stellarr 2>&1) || logs=''
  SETUP_CODE=$(printf '%s\n' "$logs" | sed -n 's/.*Setup code: \([A-Za-z0-9-]*\).*/\1/p' | tail -n 1) || SETUP_CODE=''
}

show_not_started() {
  local up='up -d'
  [ "$BUILD" -eq 0 ] || up='up -d --build'
  compose_command "$up"
  ui_section "Ready to start"
  ui_text "Nothing was started, as asked. To start Stellarr:" "  $COMPOSE_COMMAND"
}

show_summary() {
  local version='' url port title
  version=$(dk container inspect --format '{{index .Config.Labels "org.opencontainers.image.version"}}' stellarr 2>/dev/null) || version=''
  case "$version" in '' | dev | '<no value>') title="Stellarr is running" ;; *) title="Stellarr $version is running" ;; esac
  port=${SET_STELLARR_PUBLIC_PORT:-$SET_STELLARR_HTTPS_PORT}
  url="https://$SET_STELLARR_DOMAIN"
  [ "$port" = 443 ] || url="$url:$port"

  query_setup
  [ "$SETUP" = no ] || find_setup_code
  ui_section "$title"
  ui_text ''
  case "$SETUP" in
    yes)
      if [ -n "$SETUP_CODE" ]; then
        ui_rows -b Open "$url/setup" 'Setup code' "$SETUP_CODE"
        ui_text '' "Enter the code there to create the owner account. A restart of the" \
          "container draws a new code."
      else
        ui_rows -b Open "$url/setup"
        compose_command 'logs stellarr | grep "Setup code"'
        ui_text '' "Create the owner account there with the setup code from the log:" "  $COMPOSE_COMMAND"
      fi
      ;;
    no)
      ui_rows -b Open "$url"
      ui_text '' "The owner account exists; sign in as usual."
      ;;
    *)
      if [ -n "$SETUP_CODE" ]; then ui_rows -b Open "$url" 'Setup code' "$SETUP_CODE"; else ui_rows -b Open "$url"; fi
      compose_command 'logs stellarr | grep "Setup code"'
      ui_text '' "The instance did not say whether it has an owner yet. If not, create the" \
        "account at $url/setup with the newest code from:" "  $COMPOSE_COMMAND"
      ;;
  esac
  [ "$SETUP" != no ] || return 0
  show_notes
  ui_section "Later"
  compose_command 'logs -f'
  local logs_command=$COMPOSE_COMMAND
  compose_command stop
  ui_rows Update "run this installer again" Logs "$logs_command" Stop "$COMPOSE_COMMAND"
  ui_text ''
}

show_notes() {
  local voice="UDP $SET_STELLARR_MEDIA_PORT, $SET_STELLARR_TURN_PORT and $RELAY_RANGE"
  ui_section "Before you invite anyone"
  ui_bullet "Browsers warn about the self-signed certificate. For a trusted one:" \
    "$DOCS_URL/install/reverse-proxy-and-tls"
  if is_local_address "$SET_STELLARR_DOMAIN"; then
    ui_bullet "$SET_STELLARR_DOMAIN opens on this machine only. For other devices, run the" \
      "installer again with --domain and this machine's domain or IP address."
  elif [ -z "$SET_PUBLIC_IP" ]; then
    ui_bullet "PUBLIC_IP is empty, so voice works on this machine only. Set it with" \
      "--public-ip or in .env, and forward $voice" "to this machine."
  else
    ui_bullet "For voice from other networks, forward $voice" "to this machine."
  fi
}

main() {
  set -Eeuo pipefail
  settings_init
  ui_init
  # The last line of this file starts main with its input from /dev/null. A
  # download cut off inside that line can still call main, with the pipe as
  # its input, and then it must run nothing. Without /proc, where /dev/stdin
  # leads nowhere, there is no telling, and main runs.
  if [ -e /dev/stdin ] && [ ! /dev/stdin -ef /dev/null ]; then
    ui_fail "The installer was cut off while it downloaded; download it again"
    exit 1
  fi
  parse_args "$@"
  ui_init
  # Bytes are bytes from here on, whatever the files hold; the screen's
  # glyphs were chosen from the person's locale above.
  export LC_ALL=C
  unset CDPATH
  trap 'on_error "$?" "$LINENO" "$BASH_COMMAND"' ERR
  trap on_exit EXIT
  trap 'on_signal 130' INT
  trap 'on_signal 143' TERM
  validate_options
  open_terminal
  WORK=$(mktemp -d "${TMPDIR:-/tmp}/stellarr-install.XXXXXX")
  RUN_STAMP=$(date +%Y%m%d-%H%M%S)
  find_checkout

  ui_banner
  ui_section "Checking this machine"
  check_host
  check_docker
  find_container
  choose_directory
  choose_release
  decide_env
  settle_settings
  confirm_update
  check_ports

  prepare_directory
  write_env
  show_ports
  clear_compose_environment
  start_stellarr
  if [ "$START" -eq 0 ]; then
    show_not_started
    return 0
  fi
  show_summary
}

main "$@" </dev/null
