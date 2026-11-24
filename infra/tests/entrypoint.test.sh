#!/usr/bin/env bash
# Runs the container entrypoint against stubs of stellarr-init, the TLS script
# and the operator CLI, and checks what it exports to the services and the
# configuration it renders, for the defaults, for each setting it reads and
# for the values it refuses. Then runs the operator CLI's wrapper against a
# stub of the tool, and checks that it hands the CLI the public address the
# entrypoint derives, and nothing else. It writes to /etc, /run, /data, /usr/bin
# and /usr/local, so it runs only as root in a throwaway container of the
# image's base:
#
#   docker run --rm -v "$PWD":/w:ro -w /w ubuntu:24.04 bash infra/tests/entrypoint.test.sh
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
entrypoint="$repo_root/infra/entrypoint.sh"
wrapper="$repo_root/infra/stellarr-cli.sh"

if [[ ! -f /.dockerenv || $EUID -ne 0 ]]; then
  echo "Run this inside a throwaway container as root; see the comment at the top." >&2
  exit 2
fi

out=/tmp/entrypoint-test
stubs=/tmp/entrypoint-stubs
case_name=''
status=0

fail() {
  local file
  echo "FAIL: $case_name: $*" >&2
  for file in stdout stderr env calls; do
    if [[ -s $out/$file ]]; then
      echo "--- $file" >&2
      cat "$out/$file" >&2
    fi
  done
  exit 1
}

# The image's files, and stubs for what the entrypoint and the wrapper hand
# over to. Each stub records its call; stellarr-init and the tool record the
# environment they were given.
mkdir -p /etc/nginx /usr/local/bin /usr/local/lib/stellarr /opt/stellarr/cli "$stubs"
cp "$repo_root/infra/settings.sh" /usr/local/lib/stellarr/settings.sh
cp "$repo_root/infra/turnserver.conf.template" /etc/turnserver.conf.template
cp "$repo_root/infra/nginx.conf" /etc/nginx/nginx.conf.template
cat >/usr/local/bin/stellarr-init <<EOF
#!/bin/bash
echo "stellarr-init \$*" >>$out/calls
env | sort >$out/env
EOF
cat >/opt/stellarr/cli/stellarr <<EOF
#!/bin/bash
echo "cli \$*" >>$out/calls
env | sort >$out/env
exit "\$(cat $stubs/cli-exit 2>/dev/null || echo 0)"
EOF
cat >/usr/local/bin/stellarr-tls-maintenance <<EOF
#!/bin/bash
echo "stellarr-tls-maintenance \$* domain=\${STELLARR_DOMAIN-unset} data=\${STELLARR_DATA_ROOT-unset}" >>$out/calls
EOF
cat >/usr/local/bin/stellarr <<EOF
#!/bin/bash
echo "stellarr \$*" >>$out/calls
exit "\$(cat $out/cli-exit 2>/dev/null || echo 0)"
EOF
# openssl is not in the base image; the entrypoint only asks it for random hex.
cat >"$stubs/openssl" <<'EOF'
#!/bin/bash
[[ $1 == rand && $2 == -hex ]] || exit 1
od -An -tx1 -N"$3" /dev/urandom | tr -d ' \n'
echo
EOF
# One name resolves; anything else does not.
cat >"$stubs/getent" <<'EOF'
#!/bin/bash
if [[ $1 == ahostsv4 && $2 == chat.example.com ]]; then
  printf '198.51.100.4    STREAM chat.example.com\n198.51.100.4    DGRAM\n'
else
  exit 2
fi
EOF
chmod +x /usr/local/bin/stellarr-init /usr/local/bin/stellarr-tls-maintenance /usr/local/bin/stellarr \
  /opt/stellarr/cli/stellarr "$stubs/openssl" "$stubs/getent"

# Runs the entrypoint on an empty volume with only the given variables, as the
# image's ENV and docker run -e would set them. It is always given an
# environment and a listen address that .env could carry, which it must replace. A function named by the
# STAGE variable runs first, to lay down what a case needs.
run() {
  case_name=$1
  shift
  rm -rf /data/stellarr /data/elsewhere /run/stellarr /run/nginx /etc/turnserver.conf /etc/nginx/nginx.conf "$out"
  mkdir -p "$out"
  [[ -z ${STAGE:-} ]] || "$STAGE"
  status=0
  env -i PATH="$stubs:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" HOME=/root \
    STELLARR_ENVIRONMENT=Development STELLARR_API_LISTEN=0.0.0.0:5000 "$@" bash "$entrypoint" >"$out/stdout" 2>"$out/stderr" || status=$?
}

# Runs the operator CLI's wrapper with only the given variables, as docker exec
# starts it with the container's configured environment, for one config get.
run_cli() {
  case_name=$1
  shift
  rm -rf "$out"
  mkdir -p "$out"
  status=0
  env -i PATH="$stubs:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" HOME=/root \
    "$@" bash "$wrapper" --json config get public-url >"$out/stdout" 2>"$out/stderr" || status=$?
}

image_env=(STELLARR_DATA_ROOT=/data/stellarr)

expect_started() {
  [[ $status -eq 0 ]] || fail "the entrypoint exited with $status"
  grep -qx 'stellarr-init run' "$out/calls" || fail "stellarr-init was not started"
}

expect_cli_started() {
  [[ $status -eq 0 ]] || fail "the wrapper exited with $status"
  expect_call 'cli --json config get public-url'
}

# The public address in the environment last recorded, as the two keys that
# carry it, each "unset" when it is not there.
public_address() {
  local name line values=()
  for name in Operations__PublicUrl Desktop__CanonicalOrigin; do
    if line=$(grep -m1 "^$name=" "$out/env"); then
      values+=("${line#*=}")
    else
      values+=(unset)
    fi
  done
  printf '%s %s' "${values[@]}"
}

expect_failure() {
  [[ $status -ne 0 ]] || fail "the entrypoint accepted the settings"
  grep -qF -- "$1" "$out/stderr" || fail "no message: $1"
  ! grep -q '^stellarr-init' "$out/calls" 2>/dev/null || fail "stellarr-init was started"
}

# The value stellarr-init was given for $1; fails when it was not exported.
exported() {
  local line
  line=$(grep -m1 "^$1=" "$out/env") || fail "$1 is not exported"
  printf '%s' "${line#*=}"
}

expect_export() {
  local value
  value=$(exported "$1")
  [[ $value == "$2" ]] || fail "$1 is '$value', not '$2'"
}

expect_absent() {
  local name
  for name in "$@"; do
    ! grep -q "^$name=" "$out/env" || fail "$name is exported"
  done
}

expect_line() {
  grep -qxF -- "$2" "$1" || fail "$1 lacks the line: $2"
}

expect_no_line() {
  ! grep -qxF -- "$2" "$1" || fail "$1 has the line: $2"
}

expect_call() {
  grep -qxF -- "$1" "$out/calls" || fail "no call: $1"
}

# The HTTP redirect nginx was rendered with: $1 is the port, or nothing for 443.
expect_redirect() {
  # shellcheck disable=SC2016 # nginx's variables, not the shell's
  local line='return 308 https://$host'"${1:+:$1}"'$request_uri;'
  grep -qF -- "$line" /etc/nginx/nginx.conf || fail "the redirect is not: $line"
}

secret_file() {
  local value
  value=$(cat "/data/stellarr/secrets/$1") || fail "no secret file $1"
  [[ $value =~ ^[0-9a-f]{64}$ ]] || fail "secret $1 is not 64 hex digits: $value"
  printf '%s' "$value"
}

# The defaults: what the image does with no setting at all.
run defaults "${image_env[@]}"
expect_started
expect_export STELLARR_DATA_ROOT /data/stellarr
expect_export STELLARR_DOMAIN ''
expect_export STELLARR_LANGUAGE en
expect_export STELLARR_MEDIA_PORT 49000
expect_export STELLARR_TURN_TCP_ENABLED false
expect_export STELLARR_TURNS_ENABLED false
expect_export STELLARR_TURN_ENABLED false
[[ ! -e /data/stellarr/secrets/turn-shared-secret ]] || fail "created a TURN secret with TURN disabled"
[[ ! -e /etc/turnserver.conf ]] || fail "rendered a TURN config with TURN disabled"
media_secret=$(secret_file media-control-secret)
expect_absent Rtc__TurnSharedSecret
expect_export Rtc__MediaControlSecret "$media_secret"
expect_export STELLARR_MEDIA_CONTROL_SECRET "$media_secret"
expect_export Rtc__MediaControlUrl http://127.0.0.1:5200
expect_export STELLARR_MEDIA_BIND 0.0.0.0:49000
expect_export STELLARR_MEDIA_PUBLIC_IP 127.0.0.1
expect_export STELLARR_MEDIA_PUBLIC_PORT 49000
expect_export STELLARR_MEDIA_CONTROL_BIND 127.0.0.1:5200
expect_export STELLARR_MEDIA_LOG info
expect_export STELLARR_API_LISTEN 127.0.0.1:5000
expect_export STELLARR_ENVIRONMENT Production
expect_absent STELLARR_MEDIA_ADVERTISE Operations__PublicUrl Desktop__CanonicalOrigin
# Besides what the shell itself sets, everything it exports is a STELLARR_*
# name, or one of the values that have no STELLARR_* name.
others=$(sed 's/=.*//' "$out/env" | grep -vxE 'STELLARR_[A-Z0-9_]+|PATH|HOME|PWD|OLDPWD|SHLVL|_' |
  sort | tr '\n' ' ')
[[ $others == 'Rtc__MediaControlSecret Rtc__MediaControlUrl ' ]] ||
  fail "it exports: $others"
! grep -q '__[A-Z_]*__' /etc/nginx/nginx.conf || fail "nginx config keeps a placeholder"
expect_redirect ''
expect_line /run/stellarr/stellarr-instance.js 'window.stellarrInstanceLanguage="en";'
expect_call 'stellarr-tls-maintenance prepare domain= data=/data/stellarr'
grep -qF 'STELLARR_DOMAIN is not set' "$out/stderr" || fail "no word about the missing domain"
! grep -q '^stellarr ' "$out/calls" || fail "the CLI ran without a staged backup"

# Without the image's ENV the data root is the same.
run data-root-default
expect_started
expect_export STELLARR_DATA_ROOT /data/stellarr

run data-root "${image_env[@]}" STELLARR_DATA_ROOT=/data/elsewhere
expect_started
expect_export STELLARR_DATA_ROOT /data/elsewhere
[[ -s /data/elsewhere/secrets/media-control-secret ]] || fail "the secrets are not under the data root"

# The optional profile still provisions coturn and reuses the persisted secret.
run turn "${image_env[@]}" STELLARR_TURN_ENABLED=true
expect_started
expect_export STELLARR_TURN_ENABLED true
turn_secret=$(secret_file turn-shared-secret)
expect_export Rtc__TurnSharedSecret "$turn_secret"
expect_line /etc/turnserver.conf 'realm=localhost'
expect_line /etc/turnserver.conf "static-auth-secret=$turn_secret"
expect_line /etc/turnserver.conf 'no-tcp'
expect_line /etc/turnserver.conf 'no-tls'
expect_line /etc/turnserver.conf 'no-dtls'
expect_line /etc/turnserver.conf 'min-port=49160'
expect_line /etc/turnserver.conf 'max-port=49200'
expect_line /etc/turnserver.conf '# external-ip is not set; set STELLARR_PUBLIC_IP when the host is behind NAT'

run custom-relay-range "${image_env[@]}" STELLARR_TURN_ENABLED=true STELLARR_TURN_RELAY_RANGE=55000-55080
expect_started
expect_line /etc/turnserver.conf 'min-port=55000'
expect_line /etc/turnserver.conf 'max-port=55080'

# Exercise settings shared by the entrypoint and the optional relay.

run compose "${image_env[@]}" STELLARR_TURN_ENABLED=true STELLARR_DOMAIN=chat.example.com STELLARR_HTTPS_PORT=8443 \
  STELLARR_MEDIA_PORT=49000 STELLARR_TURN_PORT=3478 STELLARR_TURNS_PORT=5349
expect_started
expect_export STELLARR_DOMAIN chat.example.com
expect_export Operations__PublicUrl https://chat.example.com:8443
expect_export Desktop__CanonicalOrigin https://chat.example.com:8443
expect_export STELLARR_TURN_PORT 3478
expect_export STELLARR_TURNS_PORT 5349
expect_line /etc/turnserver.conf 'realm=chat.example.com'
expect_redirect 8443
expect_call 'stellarr-tls-maintenance prepare domain=chat.example.com data=/data/stellarr'

# STELLARR_PUBLIC_IP, which coturn and the media server advertise.
run public-ip "${image_env[@]}" STELLARR_TURN_ENABLED=true STELLARR_PUBLIC_IP=203.0.113.7
expect_started
expect_export STELLARR_MEDIA_PUBLIC_IP 203.0.113.7
expect_line /etc/turnserver.conf 'external-ip=203.0.113.7'
! grep -qF 'STELLARR_PUBLIC_IP is empty' "$out/stderr" || fail "warned about a public IP that is set"

run public-ip-auto "${image_env[@]}" STELLARR_TURN_ENABLED=true STELLARR_PUBLIC_IP=auto STELLARR_DOMAIN=chat.example.com
expect_started
expect_export STELLARR_MEDIA_PUBLIC_IP 198.51.100.4
expect_line /etc/turnserver.conf 'external-ip=198.51.100.4'

# STELLARR_HTTPS_PORT, the host port of 443, and STELLARR_PUBLIC_PORT, the one
# browsers use behind a proxy.
run https-port "${image_env[@]}" STELLARR_DOMAIN=chat.example.com STELLARR_HTTPS_PORT=09443
expect_started
expect_export Operations__PublicUrl https://chat.example.com:9443
expect_redirect 9443

run public-port "${image_env[@]}" STELLARR_DOMAIN=chat.example.com STELLARR_HTTPS_PORT=9443 STELLARR_PUBLIC_PORT=443
expect_started
expect_export Operations__PublicUrl https://chat.example.com
expect_redirect 9443

run public-url "${image_env[@]}" STELLARR_TURN_ENABLED=true STELLARR_PUBLIC_URL=https://proxy.example/
expect_started
expect_export Operations__PublicUrl https://proxy.example
expect_export STELLARR_DOMAIN ''
expect_line /etc/turnserver.conf 'realm=localhost'

# The secrets are the ones in the data volume, read back on a later start,
# whatever arrives in the environment under the names the services read.
kept_secrets() {
  mkdir -p /data/stellarr/secrets
  echo kept-turn >/data/stellarr/secrets/turn-shared-secret
  echo kept-media >/data/stellarr/secrets/media-control-secret
}
STAGE=kept_secrets run secrets "${image_env[@]}" STELLARR_TURN_ENABLED=true STELLARR_MEDIA_CONTROL_SECRET=operator-media \
  Rtc__TurnSharedSecret=operator-turn Rtc__MediaControlSecret=operator-media
expect_started
expect_export Rtc__TurnSharedSecret kept-turn
expect_export Rtc__MediaControlSecret kept-media
expect_export STELLARR_MEDIA_CONTROL_SECRET kept-media
expect_line /etc/turnserver.conf 'static-auth-secret=kept-turn'
! grep -q 'operator-' "$out/env" /etc/turnserver.conf || fail "a secret from the environment was kept"

# The other settings it shares with the API or hands to the media server.
run transports-and-language "${image_env[@]}" STELLARR_TURN_ENABLED=true STELLARR_TURN_TCP_ENABLED=true STELLARR_TURNS_ENABLED=true \
  STELLARR_LANGUAGE=pl STELLARR_MEDIA_PORT=49010 STELLARR_MEDIA_ADVERTISE=127.0.0.1:49100 STELLARR_MEDIA_LOG=debug
expect_started
expect_export STELLARR_TURN_TCP_ENABLED true
expect_export STELLARR_TURNS_ENABLED true
expect_export STELLARR_LANGUAGE pl
expect_export STELLARR_MEDIA_PORT 49010
expect_export STELLARR_MEDIA_PUBLIC_PORT 49010
expect_export STELLARR_MEDIA_ADVERTISE 127.0.0.1:49100
expect_export STELLARR_MEDIA_LOG debug
expect_no_line /etc/turnserver.conf 'no-tcp'
expect_no_line /etc/turnserver.conf 'no-tls'
expect_no_line /etc/turnserver.conf 'no-dtls'
expect_line /run/stellarr/stellarr-instance.js 'window.stellarrInstanceLanguage="pl";'
! grep -qF 'STELLARR_PUBLIC_IP is empty' "$out/stderr" || fail "warned although the candidates are given"

# A blank value counts as not set, as it does in the API.
run blank "${image_env[@]}" STELLARR_DOMAIN=' ' STELLARR_LANGUAGE='  ' STELLARR_MEDIA_PORT=' ' \
  STELLARR_HTTPS_PORT=' ' STELLARR_TURN_TCP_ENABLED=' ' STELLARR_PUBLIC_IP=' ' STELLARR_MEDIA_ADVERTISE=' '
expect_started
expect_export STELLARR_DOMAIN ''
expect_export STELLARR_LANGUAGE en
expect_export STELLARR_MEDIA_PORT 49000
expect_export STELLARR_TURN_TCP_ENABLED false
expect_export STELLARR_MEDIA_PUBLIC_IP 127.0.0.1
expect_absent STELLARR_MEDIA_ADVERTISE Operations__PublicUrl
expect_redirect ''

# The entrypoint goes by the STELLARR_* names alone. A raw key for the same
# setting changes nothing it renders, and the API takes the exported name over
# that key.
run raw-keys "${image_env[@]}" STELLARR_TURN_ENABLED=true Rtc__TurnTcpEnabled=true Rtc__MediaPort=49999 Rtc__PublicHost=raw.example \
  Stellarr__Language=pl
expect_started
expect_export STELLARR_MEDIA_PORT 49000
expect_export STELLARR_MEDIA_PUBLIC_PORT 49000
expect_export STELLARR_TURN_TCP_ENABLED false
expect_export STELLARR_LANGUAGE en
expect_line /etc/turnserver.conf 'realm=localhost'
expect_line /etc/turnserver.conf 'no-tcp'
expect_line /run/stellarr/stellarr-instance.js 'window.stellarrInstanceLanguage="en";'

# The values it refuses, before any service starts, each with its one message.
# The wrapper prints the same message and runs the CLI all the same, without
# the public address: the CLI is how an instance that does not start is
# repaired.
expect_refused() {
  local name=$1 message=$2
  shift 2
  run "$name" "${image_env[@]}" STELLARR_DOMAIN=chat.example.com "$@"
  expect_failure "$message"
  [[ $(<"$out/stderr") == "$message" ]] || fail "the entrypoint said more than: $message"
  run_cli "$name (wrapper)" "${image_env[@]}" STELLARR_DOMAIN=chat.example.com "$@"
  expect_cli_started
  [[ $(<"$out/stderr") == "$message" ]] || fail "the wrapper did not say only: $message"
  expect_absent Operations__PublicUrl Desktop__CanonicalOrigin
}
expect_refused bad-public-ip "STELLARR_PUBLIC_IP must be an IPv4 address or the value 'auto'." \
  STELLARR_PUBLIC_IP=203.0.113.300
expect_refused bad-https-port "STELLARR_HTTPS_PORT must be an integer between 1 and 65535." STELLARR_HTTPS_PORT=0
expect_refused bad-public-port "STELLARR_PUBLIC_PORT must be an integer between 1 and 65535." \
  STELLARR_PUBLIC_PORT=70000
expect_refused bad-media-port "STELLARR_MEDIA_PORT must be an integer between 1 and 65535." STELLARR_MEDIA_PORT=udp
for range in 49160 49200-49160 0-49200 49160-65536; do
  expect_refused bad-turn-range 'STELLARR_TURN_RELAY_RANGE must be an ascending port range from 1 to 65535.' \
    STELLARR_TURN_ENABLED=true STELLARR_TURN_RELAY_RANGE="$range"
done
expect_refused bad-turn-tcp "STELLARR_TURN_TCP_ENABLED must be 'true' or 'false'." STELLARR_TURN_TCP_ENABLED=yes
expect_refused bad-turns "STELLARR_TURNS_ENABLED must be 'true' or 'false'." STELLARR_TURNS_ENABLED=1
# Only the entrypoint looks the domain up, so the wrapper still hands the CLI
# the address.
run unresolved-public-ip "${image_env[@]}" STELLARR_PUBLIC_IP=auto STELLARR_DOMAIN=nowhere.example
expect_failure "STELLARR_PUBLIC_IP=auto could not resolve a public IPv4 address for STELLARR_DOMAIN=nowhere.example."
run_cli unresolved-public-ip-wrapper "${image_env[@]}" STELLARR_PUBLIC_IP=auto STELLARR_DOMAIN=nowhere.example
expect_cli_started
expect_export Operations__PublicUrl https://nowhere.example

# A staged backup is restored by the CLI before anything else; a refused one
# leaves the data as it was, and a failed one stops the start.
staged() { mkdir -p /data/stellarr/restore && : >/data/stellarr/restore/staged.zip; }
refused() { staged && echo 4 >"$out/cli-exit"; }
failed() { staged && echo 1 >"$out/cli-exit"; }
broken() { mkdir -p /data/stellarr/restore && : >/data/stellarr/restore/broken; }
STAGE=staged run restore "${image_env[@]}"
expect_started
expect_call 'stellarr --data-root /data/stellarr backup apply --yes'
STAGE=refused run restore-refused "${image_env[@]}"
expect_started
grep -qF 'The staged backup was not restored' "$out/stderr" || fail "no word about the refused backup"
STAGE=failed run restore-failed "${image_env[@]}"
expect_failure "Restoring the staged backup failed"
STAGE=broken run restore-broken "${image_env[@]}"
expect_failure "A restore failed part-way"

# The operator CLI's wrapper. On the same variables it gives the CLI the
# public address the entrypoint gives the API, and so it does on the
# environment the entrypoint exported, as when the entrypoint runs it to
# restore a backup. It sets the address in $address.
agree() {
  local name=$1 entrypoint_gave
  local -a exported_env
  shift
  run "$name" "${image_env[@]}" "$@"
  expect_started
  entrypoint_gave=$(public_address)
  mapfile -t exported_env <"$out/env"
  run_cli "$name (wrapper)" "${image_env[@]}" "$@"
  expect_cli_started
  [[ ! -s $out/stderr ]] || fail "the wrapper printed something"
  [[ $(public_address) == "$entrypoint_gave" ]] ||
    fail "the wrapper gave '$(public_address)', the entrypoint '$entrypoint_gave'"
  run_cli "$name (wrapper after the entrypoint)" "${exported_env[@]}"
  expect_cli_started
  [[ $(public_address) == "$entrypoint_gave" ]] ||
    fail "the wrapper gave '$(public_address)', the entrypoint '$entrypoint_gave'"
  address=$entrypoint_gave
}

expect_address() {
  [[ $address == "$1 $1" ]] || fail "the public address is '$address', not '$1'"
}

agree domain-and-port STELLARR_DOMAIN=chat.example.com STELLARR_HTTPS_PORT=8443
expect_address https://chat.example.com:8443
agree public-port-443 STELLARR_DOMAIN=chat.example.com STELLARR_HTTPS_PORT=9443 STELLARR_PUBLIC_PORT=443
expect_address https://chat.example.com
agree public-port STELLARR_DOMAIN=chat.example.com STELLARR_PUBLIC_PORT=09443
expect_address https://chat.example.com:9443
agree explicit-url STELLARR_DOMAIN=chat.example.com STELLARR_PUBLIC_URL=https://proxy.example/
expect_address https://proxy.example
agree ipv6-host STELLARR_DOMAIN=2001:db8::5 STELLARR_HTTPS_PORT=8443
expect_address 'https://[2001:db8::5]:8443'
agree no-domain
expect_address unset
agree blank-domain STELLARR_DOMAIN=' ' STELLARR_PUBLIC_URL=' '
expect_address unset

# The wrapper adds the address and nothing else. A default exported under an
# operator's name would make the CLI report it as set in the environment, and
# a value it was given reaches the CLI as it was written.
given=(STELLARR_DATA_ROOT=/data/stellarr STELLARR_DOMAIN=chat.example.com STELLARR_PUBLIC_PORT=09443)
run_cli nothing-else "${given[@]}"
expect_cli_started
expect_export STELLARR_PUBLIC_PORT 09443
expect_absent STELLARR_HTTPS_PORT STELLARR_LANGUAGE STELLARR_MEDIA_PORT STELLARR_TURN_TCP_ENABLED
added=$(sed 's/=.*//' "$out/env" | grep -vxE 'PATH|HOME|PWD|OLDPWD|SHLVL|_' |
  { grep -vxF "$(printf '%s\n' "${given[@]%%=*}")" || true; } | sort | tr '\n' ' ')
[[ $added == 'Desktop__CanonicalOrigin Operations__PublicUrl ' ]] || fail "the wrapper adds: $added"

# The CLI's exit status is the wrapper's; the entrypoint tells a refused
# restore by it.
echo 4 >"$stubs/cli-exit"
run_cli exit-status "${image_env[@]}"
rm "$stubs/cli-exit"
[[ $status -eq 4 ]] || fail "the wrapper exited with $status, not the CLI's 4"

# Without the shared rules the CLI still runs, without the address.
mv /usr/local/lib/stellarr/settings.sh /usr/local/lib/stellarr/settings.sh.away
run_cli without-settings "${image_env[@]}" STELLARR_DOMAIN=chat.example.com
mv /usr/local/lib/stellarr/settings.sh.away /usr/local/lib/stellarr/settings.sh
expect_cli_started
expect_absent Operations__PublicUrl Desktop__CanonicalOrigin

echo 'PASS: the entrypoint exports and renders what its settings say, and refuses the values it cannot use;' \
  'the wrapper hands the CLI the same public address.'
