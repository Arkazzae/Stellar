#!/bin/bash
# What clients see of voice while the media server comes and goes, answers
# slowly, answers 500 or answers what is not JSON, and while this server is
# restarted with people seated: the same story told to this server and to the
# first one, each with a fake media server behind it.
#
# It needs the database and the ids a run of voice-oracle.sh left in its work
# folder, and the same tools:
#
#   SHARED=<the work folder of voice-oracle.sh> ORACLE=voice ORACLE_PORT=5217 \
#   STELLARR_API=path/to/stellarr-api FAKE_MEDIA=path/to/examples/fake_media voice-lifecycle.sh
set -euo pipefail

name=${ORACLE:-voice}
port=${ORACLE_PORT:-5217}
binary=${STELLARR_API:?the path of the stellarr-api binary}
fake=${FAKE_MEDIA:?the path of the fake_media example}
shared=${SHARED:?the work folder of a run of voice-oracle.sh}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
work=${WORK:-$(mktemp -d)}
password='Corona!2026-long'
pids=()
rust_pid=
cleanup() { for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done; }
trap cleanup EXIT

code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
wait_ready() {
  for _ in $(seq 1 90); do [ "$(code "$1/health/ready")" = 200 ] && return; sleep 1; done
  echo "$1 did not become ready" >&2; exit 1
}
start_rust() {
  if [ "$1" != keep ]; then
    rm -rf "$work/rust"; mkdir -p "$work/rust"
    cp "$1" "$work/rust/stellarr.db"
  fi
  : > "$work/rust-log.txt"
  env ASPNETCORE_ENVIRONMENT=Development Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    Rtc__MediaControlUrl=http://127.0.0.1:5232 Rtc__MediaControlSecret=media-test-secret \
    "$binary" >> "$work/rust-log.txt" 2>&1 &
  rust_pid=$!
  pids+=("$rust_pid")
  for _ in $(seq 1 50); do grep -q 'Now listening on' "$work/rust-log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust-log.txt" | tail -1)
  wait_ready "$rust"
}

for target in oracle rust; do
  : > "$work/$target.txt"
  if [ "$target" = oracle ]; then media=http://127.0.0.1:5231; bind=0.0.0.0:5231; else media=http://127.0.0.1:5232; bind=127.0.0.1:5232; fi
  "$fake" "$bind" > "$work/fake-$target.txt" 2>&1 & fake_pid=$!; pids+=("$fake_pid")
  sleep 1
  curl -s "$media/_fake/down?on=1" >/dev/null
  if [ "$target" = oracle ]; then
    docker stop -t 10 "$container" >/dev/null
    docker cp "$shared/shared.db" "$container:/tmp/data/stellarr.db"
    docker start "$container" >/dev/null
    address=http://127.0.0.1:$port
    wait_ready "$address"
  else
    : > "$work/rust-log.txt"
    start_rust "$shared/shared.db"
    address=$rust
  fi
  run() { node "$here/voice-lifecycle.mjs" --phase "$1" --base "$address" --fake "$media" --ids "$shared/ids.json" --out "$work/$target.txt" --password "$password"; sleep 3; }
  run start-down
  run outage
  run restart-media
  run slow
  run nonsense
  run seated
  if [ "$target" = oracle ]; then
    docker restart -t 10 "$container" >/dev/null
    wait_ready "$address"
  else
    kill "$rust_pid"; wait "$rust_pid" 2>/dev/null || true
    start_rust keep
    address=$rust
  fi
  run after-restart
  kill "$fake_pid" 2>/dev/null || true
  [ -n "$rust_pid" ] && kill "$rust_pid" 2>/dev/null || true
  sleep 1
done
echo "transcripts: $work/oracle.txt (first server), $work/rust.txt (this one)"
diff "$work/oracle.txt" "$work/rust.txt" && echo "the two servers agree on everything observed"
