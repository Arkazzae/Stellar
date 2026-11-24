#!/bin/bash
# Custom emoji, the soundboard, the own profile and the client settings against
# the first server.
#
# Two runs, each on a fresh first server (started by `oracle start <name> <port>
# Production`) and a fresh instance of this one on an empty data folder:
#
#   1. one corpus of requests to each (custom-probe.py): the transcripts are
#      compared, then what the two left in their data folders
#      (custom-data-compare.py);
#   2. the same changes made while three people are connected to the hub
#      (custom-events.mjs, needs Node 26): what each of them is told.
#
# The exit status is 0 when the only differences are the ones listed in
# custom-known-differences.txt (a label, a tab, a word on why; the label is the
# text before the first " | " of a transcript line). It is 1 on any other
# difference, or on a listed line that no longer differs. To change the list,
# run with KEEP_WORK=1, read $work/diff.txt, and add or remove the labels.
#
# Everything started is stopped and every folder removed on every way out;
# KEEP_WORK=1 keeps the folder.
#
#   STELLARR_API=path/to/stellarr-api ORACLE_SCRIPT=path/to/oracle \
#     [ORACLE=custom ORACLE_PORT=5224 CUSTOM_PROBE_ONLY=emoji,soundboard RUN=probe|events] custom-compare.sh
set -uo pipefail

name=${ORACLE:-custom}
port=${ORACLE_PORT:-5224}
binary=${STELLARR_API:?the path of the stellarr-api binary}
oracle_script=${ORACLE_SCRIPT:?the path of the oracle script}
run=${RUN:-probe,events}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
work=$(mktemp -d "${TMPDIR:-/tmp}/stellarr-custom-compare.XXXXXX")
pid=
cleanup() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null
  "$oracle_script" stop "$name" >/dev/null 2>&1
  if [ -n "${KEEP_WORK:-}" ]; then echo "work: $work"; else rm -rf "$work"; fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

ready() { for _ in $(seq 1 90); do [ "$(curl -s -o /dev/null -w '%{http_code}' "$1/health/ready")" = 200 ] && return 0; sleep 1; done; echo "$1 is not ready" >&2; return 1; }

# Starts both servers on nothing. Sets $setup, $rust, $rust_setup and $rust_data.
start_pair() {
  rust_data=$work/rust-$1
  printf 'RateLimiting__AuthWrite__PermitLimit=500\nRateLimiting__Login__PermitLimit=500\nRateLimiting__Uploads__PermitLimit=5000\nUploads__MinimumFreeDataRootBytes=0\n' > "$work/oracle.env"
  "$oracle_script" stop "$name" >/dev/null 2>&1
  ORACLE_ENV_FILE=$work/oracle.env "$oracle_script" start "$name" "$port" Production > "$work/start.log" || return 1
  setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/start.log" | tail -1)
  mkdir -p "$rust_data"
  env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$rust_data" \
    "ConnectionStrings__Database=Data Source=$rust_data/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    RateLimiting__AuthWrite__PermitLimit=500 RateLimiting__Login__PermitLimit=500 RateLimiting__Uploads__PermitLimit=5000 \
    Uploads__MinimumFreeDataRootBytes=0 "Operations__PublicUrl=http://127.0.0.1:$port" \
    "$binary" > "$rust_data/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 50); do grep -q 'Now listening on' "$rust_data/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$rust_data/log.txt")
  ready "$rust" || return 1
  rust_setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$rust_data/log.txt" | tail -1)
}

stop_pair() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null
  pid=
  "$oracle_script" stop "$name" >/dev/null 2>&1
}

status=0
case ",$run," in *,probe,*)
  start_pair probe || exit 1
  python3 "$here/custom-probe.py" "http://127.0.0.1:$port" "$setup" > "$work/first.txt" 2> "$work/first.err" &
  first=$!
  python3 "$here/custom-probe.py" "$rust" "$rust_setup" "$rust_data/stellarr.db" > "$work/this.txt" 2> "$work/this.err" &
  second=$!
  wait $first $second
  echo "== transcripts ($(wc -l < "$work/first.txt") and $(wc -l < "$work/this.txt") lines)"
  cat "$work/first.err" "$work/this.err"
  diff "$work/first.txt" "$work/this.txt" > "$work/diff.txt"
  python3 "$here/custom-transcripts.py" "$work/first.txt" "$work/this.txt" "$here/custom-known-differences.txt" || status=1
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; pid=
  docker stop -t 20 "$container" >/dev/null
  docker cp "$container:/tmp/data" "$work/first-data" >/dev/null
  echo "== data"
  python3 "$here/custom-data-compare.py" "$work/first-data" "$rust_data" || status=1
  stop_pair
esac
case ",$run," in *,events,*)
  start_pair events || exit 1
  node "$here/custom-events.mjs" --base "http://127.0.0.1:$port" --setup "$setup" --out "$work/events-first.txt" > "$work/events-first.err" 2>&1
  node "$here/custom-events.mjs" --base "$rust" --setup "$rust_setup" --database "$rust_data/stellarr.db" --out "$work/events-this.txt" > "$work/events-this.err" 2>&1
  cat "$work/events-first.err" "$work/events-this.err"
  echo "== events ($(wc -l < "$work/events-first.txt") and $(wc -l < "$work/events-this.txt") lines)"
  diff "$work/events-first.txt" "$work/events-this.txt" && echo "the events are the same" || status=1
  stop_pair
esac
exit $status
