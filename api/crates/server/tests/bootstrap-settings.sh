#!/bin/bash
# What the bootstrap says about voice, and the GIF configuration, under every
# combination of the relay and media settings this server and the first one
# read: each case starts both fresh with the settings, founds an instance, asks
# (bootstrap-settings.mjs), and the two answers are compared. A setting that
# stops one from starting must stop the other.
#
# Needs docker, curl, Node 26, the first server's image (the one `oracle`
# runs) and the path of the `oracle` script's folder (ORACLE_ROOT, the one
# that holds `ref/publish/out`).
#
#   ORACLE=bootsettings ORACLE_PORT=5231 STELLARR_API=path/to/stellarr-api \
#     ORACLE_ROOT=path/to/folder bootstrap-settings.sh
#
# Prints the cases where the two differ, and says how many there were. The
# container and the work folder are removed when it ends, whatever the way.
set -uo pipefail

name=${ORACLE:-bootsettings}
port=${ORACLE_PORT:-5231}
binary=${STELLARR_API:?the path of the stellarr-api binary}
root=${ORACLE_ROOT:?the folder the oracle script keeps ref/publish/out in}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
work=$(mktemp -d)
secret=oracle-turn-secret-oracle-turn-secret-0123
pid=

cleanup() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  docker stop -t 2 "$container" >/dev/null 2>&1 || true
  docker container rm "$container" >/dev/null 2>&1 || true
  [ -n "${KEEP:-}" ] || rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# $@: NAME=value settings.
oracle_up() {
  local extra=()
  for setting in "$@"; do extra+=(-e "$setting"); done
  docker container rm -f "$container" >/dev/null 2>&1 || true
  docker run -d --name "$container" -p "127.0.0.1:$port:5000" ${extra[@]+"${extra[@]}"} \
    -v "$root/ref/publish/out":/app:ro \
    -e ASPNETCORE_ENVIRONMENT=Production -e ASPNETCORE_URLS=http://0.0.0.0:5000 -e STELLARR_DATA_ROOT=/tmp/data \
    -e "ConnectionStrings__Database=Data Source=/tmp/data/stellarr.db;Pooling=True" \
    -e Rtc__MediaControlUrl=http://127.0.0.1:5200 -e Rtc__MediaControlSecret=oracle-media-secret-oracle-media-secret \
    -e "Rtc__TurnSharedSecret=$secret" \
    -e "Operations__PublicUrl=http://127.0.0.1:$port" -e "Desktop__CanonicalOrigin=http://127.0.0.1:$port" \
    stellarr-api-dev:local \
    sh -c 'mkdir -p /tmp/data /opt/api && cp -r /app/. /opt/api/ && cd /opt/api && exec dotnet Stellarr.Api.dll' >/dev/null
  for _ in $(seq 1 100); do
    [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/health/ready" 2>/dev/null)" = 200 ] && return 0
    [ "$(docker inspect -f '{{.State.Running}}' "$container" 2>/dev/null)" = true ] || return 1
    perl -e 'select(undef,undef,undef,0.3)'
  done
  return 1
}

rust_up() {
  rm -rf "$work/rust"; mkdir -p "$work/rust"
  env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=$secret STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 "$@" \
    "$binary" > "$work/rust.log" 2>&1 &
  pid=$!
  for _ in $(seq 1 60); do
    grep -q 'listening on' "$work/rust.log" && break
    kill -0 "$pid" 2>/dev/null || return 1
    perl -e 'select(undef,undef,undef,0.2)'
  done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust.log")
  for _ in $(seq 1 60); do
    [ "$(curl -s -o /dev/null -w '%{http_code}' "$rust/health/ready")" = 200 ] && return 0
    perl -e 'select(undef,undef,undef,0.2)'
  done
  return 1
}

differences=0
cases=0
run_case() { # NAME=value ...
  local label=${*:-defaults}
  # ONLY=text runs the cases whose settings contain it.
  [ -z "${ONLY:-}" ] || [[ "$label" == *"$ONLY"* ]] || return 0
  cases=$((cases + 1))
  local o r
  if oracle_up "$@"; then
    setup=$(docker logs "$container" 2>&1 | sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' | tail -1)
    o=$(node "$here/bootstrap-settings.mjs" --base "http://127.0.0.1:$port" --setup "$setup" --secret "$secret" 2>&1)
  else
    o="did not start"
  fi
  docker stop -t 2 "$container" >/dev/null 2>&1; docker container rm "$container" >/dev/null 2>&1
  if rust_up "$@"; then
    setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/rust.log" | tail -1)
    r=$(node "$here/bootstrap-settings.mjs" --base "$rust" --setup "$setup" --secret "$secret" 2>&1)
  else
    r="did not start"
  fi
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; pid=
  if [ "$o" = "$r" ]; then
    echo "same     $label  ->  $(echo "$o" | head -1)"
  else
    differences=$((differences + 1))
    echo "DIFFERS  $label"
    echo "  first server: $(echo "$o" | tr '\n' ' ' | cut -c1-400)"
    echo "  this server : $(echo "$r" | tr '\n' ' ' | cut -c1-400)"
  fi
}

run_case
run_case Rtc__PublicHost=voice.example.test
run_case Rtc__PublicHost=
run_case "Rtc__PublicHost= spaced.example "
run_case Rtc__TurnTcpEnabled=true
run_case Rtc__TurnTcpEnabled=TRUE
run_case "Rtc__TurnTcpEnabled= true "
run_case Rtc__TurnTcpEnabled=false
for value in 1 yes abc ""; do run_case "Rtc__TurnTcpEnabled=$value"; done
run_case Rtc__TurnsEnabled=true Rtc__TurnTcpEnabled=true
run_case Rtc__TurnsEnabled=true Rtc__TurnsPort=443
for value in 1 yes abc ""; do run_case "Rtc__TurnsEnabled=$value"; done
for value in 0 65535 65536 -1 abc "" " 443 " "+443" "0x1BB"; do
  run_case Rtc__TurnsEnabled=true "Rtc__TurnsPort=$value"
  run_case Rtc__TurnsEnabled=false "Rtc__TurnsPort=$value"
done
for value in 1 65535 0 65536 -1 99999999999 abc "" " 3478 " "+3478" "0x10" "0X10" "&h10" "#10" "3478.0" "01" "1_0" "٣٤٧٨" "1,000" "3 478"; do
  run_case "Rtc__TurnPort=$value"
done
# The media port is read when the server starts, and a wrong one stops it.
for value in 1 65535 0 65536 abc "" " 49000 " "+49000" "0x10"; do run_case "Rtc__MediaPort=$value"; done
run_case Giphy__ApiKey=abc123
run_case "Giphy__ApiKey= padded "
run_case Giphy__ApiKey=
echo "$cases cases, $differences differ"
[ "$differences" = 0 ]
