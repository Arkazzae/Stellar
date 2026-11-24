#!/bin/bash
# The hourly housekeeping of this server against the first server's, on copies
# of one database file.
#
# One database is seeded (housekeeping-seed.py) with rows a few minutes inside
# and outside every retention: the message journal at its cutoff and at its
# clamps, the short-lived records of signing in, uploads that are old enough
# and used or not, an object deletion queue, the settings of two servers (the
# one written last decides), and files of the backups and logs folders with
# ages on either side of their retention. Both servers run the housekeeping
# once on a copy, and the tables, the files and the log line are compared.
#
# Ages are relative to a moment the seed is told (about a minute ahead), and a
# row is two minutes either side of its cutoff, so the minute or two between
# seeding and the two runs does not move a row over its line: both servers
# must run within about a minute of the seed.
#
# Five runs: cleanup on, cleanup off, a batch of 3 (more orphans and queued
# objects than that), and the journal's days at its two clamps (1 -> 7,
# 1000 -> 365).
#
# Needs docker, python3 and curl. The first server runs in a container made by
# the integrator's `oracle` script; everything is removed on every way out
# (KEEP_WORK=1 keeps the work folder).
#
#   ORACLE=backups ORACLE_PORT=5226 STELLARR_API=path/to/stellarr-api \
#     ORACLE_SCRIPT=path/to/oracle housekeeping-shared-database.sh
set -euo pipefail

name=${ORACLE:-backups}
port=${ORACLE_PORT:-5226}
binary=${STELLARR_API:?the path of the stellarr-api binary}
oracle_script=${ORACLE_SCRIPT:?the path of the oracle script}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
work=$(mktemp -d)
pid=
cleanup() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  "$oracle_script" stop "$name" >/dev/null 2>&1 || true
  docker rm -f "$container" >/dev/null 2>&1 || true
  if [ -n "${KEEP_WORK:-}" ]; then echo "kept: $work" >&2; else rm -rf "$work"; fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

seed() { python3 -I "$here/housekeeping-seed.py" "$@"; }
fresh_container() {
  "$oracle_script" stop "$name" >/dev/null 2>&1 || true
  docker rm -f "$container" >/dev/null 2>&1 || true
  ORACLE_ENV_FILE=$1 "$oracle_script" start "$name" "$port" Production > "$work/start.log"
}

# The base: a database the first server made and an owner registered, which
# gives the schema and the instance server.
printf 'Maintenance__InitialDelayMinutes=600\n' > "$work/base.env"
fresh_container "$work/base.env"
code=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/start.log" | tail -1)
curl -fs -o /dev/null -H 'Content-Type: application/json' -H 'X-Stellarr-Client: web' -H 'X-Stellarr-Client-Version: 2.0.0' \
  -d "{\"username\":\"owner\",\"displayName\":\"Owner\",\"email\":\"owner@example.test\",\"password\":\"Corona!2026-long\",\"setupCode\":\"$code\"}" \
  "http://127.0.0.1:$port/api/auth/register"
docker stop -t 20 "$container" >/dev/null
docker cp "$container:/tmp/data/stellarr.db" "$work/base.db"

scenario() { # label cleanup batch journal-days
  local label=$1 cleanup_flag=$2 batch=$3 days=$4
  local dir=$work/$label
  mkdir -p "$dir"
  cp "$work/base.db" "$dir/seeded.db"
  local now=$(( $(date +%s) + 60 ))
  HOUSEKEEPING_CLEANUP=$cleanup_flag seed seed "$dir/seeded.db" "$dir" "$now"

  # The first server.
  printf 'Maintenance__InitialDelayMinutes=0\nMaintenance__BatchSize=%s\n' "$batch" > "$dir/oracle.env"
  [ -z "$days" ] || printf 'Maintenance__MessageChangeRetentionDays=%s\n' "$days" >> "$dir/oracle.env"
  fresh_container "$dir/oracle.env"
  docker stop -t 20 "$container" >/dev/null
  docker cp "$dir/seeded.db" "$container:/tmp/data/stellarr.db"
  docker cp "$dir/root/." "$container:/tmp/data/"
  lines_before=$("$oracle_script" logs "$name" 2>&1 | wc -l)
  docker start "$container" >/dev/null
  # The log holds the first start too, which ran on the base database: wait
  # for the second start's own listening line, and its run.
  for _ in $(seq 1 60); do
    [ "$("$oracle_script" logs "$name" 2>&1 | grep -c 'Now listening')" -ge 2 ] && break
    sleep 1
  done
  if [ "$cleanup_flag" = 1 ]; then
    for _ in $(seq 1 30); do
      [ "$("$oracle_script" logs "$name" 2>&1 | grep -c 'Maintenance removed')" -ge 2 ] && break
      sleep 1
    done
  else
    sleep 8
  fi
  docker stop -t 20 "$container" >/dev/null
  mkdir -p "$dir/first"
  docker cp "$container:/tmp/data/." "$dir/first/"
  "$oracle_script" logs "$name" > "$dir/first.log" 2>&1 || true
  # Only what the second start said: the first ran on the base database.
  tail -n +$((lines_before + 1)) "$dir/first.log" | grep 'Maintenance removed' | tail -1 | sed 's/.*Maintenance removed/Maintenance removed/' > "$dir/first.line" || true

  # This server.
  mkdir -p "$dir/rust"
  cp -Rp "$dir/root/." "$dir/rust/"
  cp "$dir/seeded.db" "$dir/rust/stellarr.db"
  local extra=("Maintenance__BatchSize=$batch")
  [ -z "$days" ] || extra+=("Maintenance__MessageChangeRetentionDays=$days")
  env ASPNETCORE_ENVIRONMENT=Testing Testing__Workers=housekeeping Maintenance__InitialDelayMinutes=0 \
    Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$dir/rust" "ConnectionStrings__Database=Data Source=$dir/rust/stellarr.db" \
    ASPNETCORE_URLS=http://127.0.0.1:0 "${extra[@]}" "$binary" > "$dir/rust.log" 2>&1 &
  pid=$!
  for _ in $(seq 1 40); do grep -q 'Maintenance removed' "$dir/rust.log" && break; sleep 1; done
  [ "$cleanup_flag" = 1 ] || sleep 5
  kill -TERM "$pid"; wait "$pid" 2>/dev/null || true; pid=
  grep 'Maintenance removed' "$dir/rust.log" | sed 's/.*Maintenance removed/Maintenance removed/' > "$dir/rust.line" || true

  seed dump "$dir/first/stellarr.db" "$dir/first" "$dir" > "$dir/first.txt"
  seed dump "$dir/rust/stellarr.db" "$dir/rust" "$dir" > "$dir/rust.txt"
  cat "$dir/first.line" >> "$dir/first.txt" 2>/dev/null || true
  cat "$dir/rust.line" >> "$dir/rust.txt" 2>/dev/null || true
  if diff -u "$dir/first.txt" "$dir/rust.txt" > "$dir/diff.txt"; then
    echo "$label: identical"
  else
    echo "$label: DIFFERENT"; cat "$dir/diff.txt"; failed=1
  fi
  [ -z "${SHOW:-}" ] || cat "$dir/first.txt"
}

failed=0
want() { [ -z "${ONLY:-}" ] || [ "$ONLY" = "$1" ]; }
want enabled && scenario enabled 1 1000 ""
want disabled && scenario disabled 0 1000 ""
want batch3 && scenario batch3 1 3 ""
want clamp-low && scenario clamp-low 1 1000 1
want clamp-high && scenario clamp-high 1 1000 1000
exit "$failed"
