#!/bin/bash
# The routes of roles, overrides and moderation, this server against the first
# one, on copies of one database file.
#
# People are registered on the first server, which is then stopped and its
# database copied out. Servers of random shape (roles with tied positions,
# held roles, sections, channels, overrides of roles and of members, bans,
# strangers) are written into the copy, with a plan of requests for each: every
# route of the group, by people who may and people who may not. The copy goes
# to both servers, each gets the plan, and the script compares every answer,
# the lists of roles and members after every change, and in the end every row
# the changes left (roles by position, who holds what, overrides, bans, audit).
# Then the hub is listened to on both while the plan runs, and a second stage
# gives some accounts a second step their sessions never passed, edits both
# files while the servers are stopped, and asks the fourteen routes that need
# a fresh one, on the instance's own server and on another.
#
# Needs docker, sqlite3 (Python's), curl and python3, and the first server's
# container from `oracle start <name> <port> Production` with the rate limits
# raised (ORACLE_ENV_FILE), which must be fresh: the script founds the instance.
# Not part of the ordinary run.
#
#   ORACLE=roles ORACLE_PORT=5213 ACARGO=path/to/acargo STELLARR_API=path/to/stellarr-api roles-shared-database.sh
#   (WORLDS=100 STEPS=8 PEOPLE=8 SEED=7 to change the shapes; ROLES_PAIRS=1 for the pairs of
#   requests that may and may not, one world each, instead of random worlds;
#   EVENT_WORLDS=100 for the hub stage, 0 to skip it; STAGES=main,events,stale to choose;
#   KEEP=1 keeps the work folder, KEEP_EVENTS=file keeps the transcript)
set -euo pipefail

name=${ORACLE:-roles}
port=${ORACLE_PORT:-5213}
acargo=${ACARGO:?the path of acargo}
binary=${STELLARR_API:?the path of the stellarr-api binary}
people=${PEOPLE:-8}
worlds=${WORLDS:-100}
steps=${STEPS:-8}
seed=${SEED:-12345}
stages=${STAGES:-main,events,stale}
container=stellarr-port-oracle-$name
oracle=http://127.0.0.1:$port
here=$(cd "$(dirname "$0")" && pwd)
api=$(cd "$here/../../.." && pwd)
work=$(mktemp -d)
pid=

# Every way out: the server this script started is stopped, the first server's
# container is stopped (the `oracle` script removes it), the folder is removed.
cleanup() {
  [ -z "$pid" ] || { kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; }
  docker stop -t 10 "$container" >/dev/null 2>&1 || true
  [ -z "${KEEP:-}" ] || return 0
  rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

wait_ready() {
  for _ in $(seq 1 60); do
    [ "$(curl -s -o /dev/null -w '%{http_code}' "$1/health/ready")" = 200 ] && return
    sleep 1
  done
  echo "$1 did not become ready" >&2; exit 1
}

setup=$(docker logs "$container" 2>&1 | sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' | tail -1)
python3 "$here/permissions-probe.py" register "$oracle" "$setup" "$work" "$people"

docker stop -t 10 "$container" >/dev/null
docker cp "$container:/tmp/data/stellarr.db" "$work/stellarr.db"
arranger=arrange_worlds_in_a_shared_database
[ -z "${ROLES_PAIRS:-}" ] || arranger=arrange_pairs_of_requests
(cd "$api" && ROLES_DB="$work/stellarr.db" ROLES_USERS="$work/users.txt" ROLES_PLAN="$work/plan.jsonl" \
  ROLES_WORLDS="$worlds" ROLES_STEPS="$steps" ROLES_SEED="$seed" \
  "$acargo" test -p stellarr-api --test api "$arranger" -- --ignored --nocapture)
(cd "$api" && ROLES_DB="$work/stellarr.db" ROLES_USERS="$work/users.txt" ROLES_STALE_PLAN="$work/stale-plan.jsonl" \
  "$acargo" test -p stellarr-api --test api arrange_a_second_step -- --ignored --nocapture)

stop_rust() { [ -z "$pid" ] || { kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; pid=; }; }

# Starts this server on the file $1, copied into its own folder; sets $rust.
start_rust() {
  stop_rust
  rm -rf "$work/rust"; mkdir "$work/rust"
  cp "${1:-$work/stellarr.db}" "$work/rust/stellarr.db"
  run_rust
}

# Starts this server on the file that is already in its folder.
run_rust() {
  env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    RateLimiting__Login__PermitLimit=500 RateLimiting__AuthWrite__PermitLimit=500 \
    "$binary" > "$work/rust/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 60); do grep -q 'listening on' "$work/rust/log.txt" && break; sleep 1; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
  wait_ready "$rust"
}

# The first server on the file $1.
start_oracle() {
  docker stop -t 10 "$container" >/dev/null 2>&1 || true
  docker cp "${1:-$work/stellarr.db}" "$container:/tmp/data/stellarr.db"
  docker start "$container" >/dev/null
  wait_ready "$oracle"
}

status=0
has() { case ",$stages," in *",$1,"*) return 0;; esac; return 1; }
probe() { python3 "$here/roles-probe.py" "$@"; }

if has main; then
  start_rust; start_oracle
  probe run "$rust" "$oracle" "$work" "$work/plan.jsonl" || status=1
  stop_rust
  docker stop -t 10 "$container" >/dev/null
  docker cp "$container:/tmp/data/stellarr.db" "$work/theirs.db"
  probe dump "$work/rust/stellarr.db" "$work/plan.jsonl" "$work/ours.json"
  probe dump "$work/theirs.db" "$work/plan.jsonl" "$work/theirs.json"
  probe diff "$work/ours.json" "$work/theirs.json" || status=1
fi

# What the people of each server hear while the plan runs: both servers again on
# the arranged file, a hub connection for every member, and the transcripts compared.
if has events && [ "${EVENT_WORLDS:-30}" != 0 ]; then
  start_rust; start_oracle
  node "$here/roles-events.mjs" --base "$rust" --plan "$work/plan.jsonl" --users "$work/users.txt" --worlds "${EVENT_WORLDS:-30}" --out "$work/ours-events.txt" || status=1
  node "$here/roles-events.mjs" --base "$oracle" --plan "$work/plan.jsonl" --users "$work/users.txt" --worlds "${EVENT_WORLDS:-30}" --out "$work/theirs-events.txt" || status=1
  [ -z "${KEEP_EVENTS:-}" ] || cp "$work/ours-events.txt" "$KEEP_EVENTS"
  if diff "$work/ours-events.txt" "$work/theirs-events.txt" > "$work/events.diff"; then
    echo "events: $(wc -l < "$work/ours-events.txt") lines of transcript, no differences"
  else
    head -60 "$work/events.diff"; echo "events: DIFFERENT"; status=1
  fi
fi

# The second step: the sessions are made first, the accounts get a step their
# sessions never passed while both servers are stopped, and then both are asked.
if has stale; then
  start_rust; start_oracle
  probe login "$rust" "$work" "$work/stale-plan.jsonl" "$work/cookies-ours.json"
  probe login "$oracle" "$work" "$work/stale-plan.jsonl" "$work/cookies-theirs.json"
  stop_rust
  docker stop -t 10 "$container" >/dev/null
  docker cp "$container:/tmp/data/stellarr.db" "$work/stale-theirs.db"
  probe edit-second-step "$work/rust/stellarr.db" "$work/users.txt"
  probe edit-second-step "$work/stale-theirs.db" "$work/users.txt"
  run_rust
  start_oracle "$work/stale-theirs.db"
  echo "second step:"
  probe run "$rust" "$oracle" "$work" "$work/stale-plan.jsonl" "$work/cookies-ours.json" "$work/cookies-theirs.json" || status=1
fi
exit $status
