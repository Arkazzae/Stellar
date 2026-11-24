#!/bin/bash
# The operator panel (/api/cockpit) against the first server on one database file.
#
# The first server is started fresh and seeded through its own routes
# (cockpit-oracle.mjs seed): accounts, roles of the instance, servers of other
# people, content the overview counts, sessions and a disabled account. It is
# stopped and its database copied; what no route makes without an outside
# provider (identity links, second-step methods, trusted devices, recovery
# codes) is written into the copy (prepare). The first server runs on one copy
# and this server on another, both are asked the same questions
# (cockpit-oracle.mjs probe) while two fake mail servers keep what each sends,
# and the two transcripts are compared, then the mail, then the two databases
# table by table (cockpit-tables.py), with ids, secrets and moments replaced.
#
# The second step is asked on fresh copies of the seeded file: sessions made
# first, the file edited so that the owner, Maya and Joe have an authenticator
# and one owner session passed the step now, one ten minutes ago and one never,
# then both servers started again and asked (stale-login, stale-edit,
# stale-ask), the policies that ask the panel itself for the step included.
#
# Needs docker, node 26 and python3. The first server is a container made by
# `oracle start <name> <port> Production`, which the script starts and stops
# itself. Every server and helper it starts is stopped, and its work folder
# removed, however it ends (KEEP=1 keeps the folder). Not part of the ordinary
# run. It exits 1 when a transcript differs in a way that is not on the list of
# known differences (see KNOWN in cockpit-oracle.mjs: each is there by decision
# or accepted, with its reason).
#
#   STELLARR_API=path/to/stellarr-api ORACLE_SCRIPT=path/to/oracle \
#     [ORACLE=cockpit ORACLE_PORT=5233 MAIL_PORTS=5291,5292,5293,5294 KEEP=1 PROVIDERS=1] \
#     cockpit-shared-database.sh
set -euo pipefail

name=${ORACLE:-cockpit}
port=${ORACLE_PORT:-5233}
binary=${STELLARR_API:?the path of the stellarr-api binary}
oracle_script=${ORACLE_SCRIPT:?the path of the oracle script}
IFS=, read -r mail_first mail_this mail_closed mail_refusing <<< "${MAIL_PORTS:-5291,5292,5293,5294}"
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
oracle=http://127.0.0.1:$port
public=http://127.0.0.1:$port
work=$(mktemp -d "${TMPDIR:-/tmp}/cockpit-oracle.XXXXXX")
pid=
helpers=
unknown=0

finish() {
  status=$?
  trap - EXIT INT TERM
  if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  for helper in $helpers; do kill "$helper" 2>/dev/null || true; wait "$helper" 2>/dev/null || true; done
  "$oracle_script" stop "$name" >/dev/null 2>&1 || true
  if [ "${KEEP:-0}" = 1 ]; then echo "work kept: $work"; else rm -rf "$work"; fi
  exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

settings=$work/oracle.env
printf '%s\n' RateLimiting__AuthWrite__PermitLimit=5000 RateLimiting__Login__PermitLimit=5000 \
  RateLimiting__InvitePreview__PermitLimit=5000 RateLimiting__SecurityWrite__PermitLimit=5000 > "$settings"
# PROVIDERS=1: both servers have identity providers set up (the original one and two named ones). The
# panel only reads their settings, so nothing listens at the authorities.
providers=()
if [ "${PROVIDERS:-0}" = 1 ]; then
  providers=(Oidc__Enabled=true Oidc__Authority=https://idp.example.test/default Oidc__ClientId=panel-tests
    Oidc__ClientSecret=test-client-secret "Oidc__DisplayName=Main IdP"
    Oidc__Providers__example__Authority=https://id.example.test Oidc__Providers__example__ClientId=panel-tests
    Oidc__Providers__example__ClientSecret=test-client-secret "Oidc__Providers__example__DisplayName=Example IdP"
    Oidc__Providers__google__Authority=https://accounts.example.test Oidc__Providers__google__ClientId=panel-tests
    Oidc__Providers__google__ClientSecret=test-client-secret)
  printf '%s\n' "${providers[@]}" >> "$settings"
fi
ready() { for _ in $(seq 1 90); do [ "$(curl -s -o /dev/null -w '%{http_code}' "$1/health/ready")" = 200 ] && return; sleep 1; done; echo "$1 is not ready" >&2; exit 1; }

# The first server's file goes in and out with the container stopped; the
# write-ahead log goes with it, and a stale one never meets a new file.
take_oracle_db() {
  docker stop -t 20 "$container" >/dev/null
  rm -f "$1" "$1-wal" "$1-shm"
  docker cp "$container:/tmp/data/stellarr.db" "$1"
  docker cp "$container:/tmp/data/stellarr.db-wal" "$1-wal" 2>/dev/null || true
}
give_oracle_db() {
  : > "$work/empty"
  docker cp "$1" "$container:/tmp/data/stellarr.db"
  docker cp "$work/empty" "$container:/tmp/data/stellarr.db-wal"
  docker cp "$work/empty" "$container:/tmp/data/stellarr.db-shm"
  docker start "$container" >/dev/null
  ready "$oracle"
}
# This server on a copy of a file, in a data folder that keeps its keys (the
# cookies of its sessions) from one start to the next.
start_rust() {
  stop_rust
  mkdir -p "$work/rust"
  rm -f "$work/rust/stellarr.db" "$work/rust/stellarr.db-wal" "$work/rust/stellarr.db-shm"
  cp "$1" "$work/rust/stellarr.db"
  env TZ=UTC ASPNETCORE_ENVIRONMENT=Production STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    Rtc__MediaControlUrl=http://127.0.0.1:5200 Rtc__MediaControlSecret=oracle-media-secret-oracle-media-secret \
    Rtc__TurnSharedSecret=oracle-turn-secret-oracle-turn-secret-0123 \
    RateLimiting__AuthWrite__PermitLimit=5000 RateLimiting__Login__PermitLimit=5000 \
    RateLimiting__InvitePreview__PermitLimit=5000 RateLimiting__SecurityWrite__PermitLimit=5000 \
    "Operations__PublicUrl=$public" "Desktop__CanonicalOrigin=$public" ${providers[@]+"${providers[@]}"} \
    "$binary" > "$work/rust.log" 2>&1 &
  pid=$!
  for _ in $(seq 1 100); do grep -q 'Now listening on' "$work/rust.log" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust.log" | head -1)
  ready "$rust"
}
stop_rust() {
  if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; pid=; fi
}

# The fake mail servers: one that keeps what each server sends, a port where
# nothing listens, and one that refuses everything.
python3 -I "$here/smtp-sink.py" "$work/mail-first" "$mail_first" > /dev/null 2>&1 &
helpers="$helpers $!"
python3 -I "$here/smtp-sink.py" "$work/mail-this" "$mail_this" > /dev/null 2>&1 &
helpers="$helpers $!"
node "$here/cockpit-oracle.mjs" smtp-refuse --port "$mail_refusing" > /dev/null 2>&1 &
helpers="$helpers $!"

"$oracle_script" stop "$name" >/dev/null 2>&1 || true
ORACLE_ENV_FILE=$settings "$oracle_script" start "$name" "$port" Production > "$work/start.log"
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/start.log" | tail -1)
node "$here/cockpit-oracle.mjs" seed --base "$oracle" --setup "$setup" --state "$work/state.json"
take_oracle_db "$work/seeded.db"
# Folds a file's write-ahead log into it, so that the file alone is the whole database.
sqlite3_checkpoint() { python3 -I -c 'import sqlite3, sys; c = sqlite3.connect(sys.argv[1]); c.execute("PRAGMA wal_checkpoint(TRUNCATE)"); c.close()' "$1"; }
sqlite3_checkpoint "$work/seeded.db"
node "$here/cockpit-oracle.mjs" prepare --db "$work/seeded.db" --state "$work/state.json"
sqlite3_checkpoint "$work/seeded.db"
rm -f "$work/seeded.db-wal" "$work/seeded.db-shm"

echo "== the probe"
cp "$work/seeded.db" "$work/oracle.db"
give_oracle_db "$work/oracle.db"
start_rust "$work/seeded.db"
# Run in the background only: exec makes the pid that is waited for and stopped node's own.
probe() { # base, smtp host, smtp port, transcript
  exec node "$here/cockpit-oracle.mjs" probe --base "$1" --state "$work/state.json" --seeded "$work/seeded.db" --out "$4" --public "$public" \
    --smtp-host "$2" --smtp-port "$3" --smtp-closed "$mail_closed" --smtp-refusing "$mail_refusing"
}
probe "$oracle" host.docker.internal "$mail_first" "$work/first.txt" &
first=$!
probe "$rust" 127.0.0.1 "$mail_this" "$work/this.txt" &
second=$!
helpers="$helpers $first $second"
wait $first
wait $second
echo "== transcripts"
grep -B2 '^!!' "$work/first.txt" | head -30 || true
node "$here/cockpit-oracle.mjs" compare --first "$work/first.txt" --this "$work/this.txt" || unknown=1
echo "== mail"
node "$here/cockpit-oracle.mjs" mails --first "$work/mail-first" --this "$work/mail-this"

stop_rust
take_oracle_db "$work/after-first.db"
echo "== databases"
python3 -I "$here/cockpit-tables.py" "$work/after-first.db" "$work/rust/stellarr.db" "$work/seeded.db"

echo "== second step"
cp "$work/seeded.db" "$work/stale-oracle.db"
give_oracle_db "$work/stale-oracle.db"
start_rust "$work/seeded.db"
node "$here/cockpit-oracle.mjs" stale-login --base "$oracle" --out "$work/jar-first.json"
node "$here/cockpit-oracle.mjs" stale-login --base "$rust" --out "$work/jar-this.json"
take_oracle_db "$work/stale-oracle.db"
stop_rust
cp "$work/rust/stellarr.db" "$work/stale-this.db"
if [ -f "$work/rust/stellarr.db-wal" ]; then cp "$work/rust/stellarr.db-wal" "$work/stale-this.db-wal"; fi
node "$here/cockpit-oracle.mjs" stale-edit --db "$work/stale-oracle.db"
node "$here/cockpit-oracle.mjs" stale-edit --db "$work/stale-this.db"
sqlite3_checkpoint "$work/stale-oracle.db"
sqlite3_checkpoint "$work/stale-this.db"
give_oracle_db "$work/stale-oracle.db"
start_rust "$work/stale-this.db"
stale() { # base, jar, transcript
  exec node "$here/cockpit-oracle.mjs" stale-ask --base "$1" --state "$work/state.json" --seeded "$work/seeded.db" --cookies "$2" --out "$3" --public "$public"
}
stale "$oracle" "$work/jar-first.json" "$work/stale-first.txt" &
first=$!
stale "$rust" "$work/jar-this.json" "$work/stale-this.txt" &
second=$!
helpers="$helpers $first $second"
wait $first
wait $second
grep -B2 '^!!' "$work/stale-first.txt" | head -30 || true
node "$here/cockpit-oracle.mjs" compare --first "$work/stale-first.txt" --this "$work/stale-this.txt" || unknown=1
stop_rust
take_oracle_db "$work/stale-after-first.db"
echo "== databases after the second step (a refused change must leave nothing behind)"
python3 -I "$here/cockpit-tables.py" "$work/stale-after-first.db" "$work/rust/stellarr.db" "$work/seeded.db"
echo "work: $work"
[ "$unknown" = 0 ] || { echo "there are differences that are not known"; exit 1; }
