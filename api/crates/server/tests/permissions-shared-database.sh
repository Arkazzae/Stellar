#!/bin/bash
# The permission rules and the first server on one database file.
#
# People are registered on the first server, which is then stopped and its
# database copied out. Servers of random shape (roles, a base role that may
# lack view, sections, text and voice channels, overrides of roles and of
# members at both levels, administrators, held roles) are written into the
# copy, along with what these rules say each person holds and sees there. The
# copy goes back, the first server starts on it, and each person is asked what
# the first server says: the bootstrap (server-wide permissions, the channels
# and sections they see) and a message sent to every text channel (404 for a
# channel they do not see, 403 naming `sendMessages` where they lack it, 201).
#
# Needs docker, curl and python3, and the first server's container from
# `oracle start <name> <port> Production`, which must be fresh: the script
# founds the instance. Not part of the ordinary run.
#
#   ORACLE=permissions ORACLE_PORT=5201 ACARGO=path/to/acargo permissions-shared-database.sh
#   (SERVERS=40 PEOPLE=8 SEED=7 to change the shapes)
set -euo pipefail

name=${ORACLE:-permissions}
port=${ORACLE_PORT:-5201}
acargo=${ACARGO:?the path of acargo}
people=${PEOPLE:-8}
servers=${SERVERS:-40}
seed=${SEED:-12345}
container=stellarr-port-oracle-$name
oracle=http://127.0.0.1:$port
here=$(cd "$(dirname "$0")" && pwd)
api=$(cd "$here/../../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

wait_ready() {
  for _ in $(seq 1 60); do
    [ "$(curl -s -o /dev/null -w '%{http_code}' "$oracle/health/ready")" = 200 ] && return
    sleep 1
  done
  echo "the first server did not become ready" >&2; exit 1
}

setup=$(docker logs "$container" 2>&1 | sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' | tail -1)
python3 "$here/permissions-probe.py" register "$oracle" "$setup" "$work" "$people"

docker stop -t 10 "$container" >/dev/null
docker cp "$container:/tmp/data/stellarr.db" "$work/stellarr.db"
(cd "$api" && PERMISSIONS_DB="$work/stellarr.db" PERMISSIONS_USERS="$work/users.txt" \
  PERMISSIONS_EXPECTED="$work/expected.jsonl" PERMISSIONS_SERVERS="$servers" PERMISSIONS_SEED="$seed" \
  "$acargo" test -p stellarr-api --test api arrange_servers_in_a_shared_database -- --ignored --nocapture)
docker cp "$work/stellarr.db" "$container:/tmp/data/stellarr.db"
docker start "$container" >/dev/null
wait_ready

python3 "$here/permissions-probe.py" observe "$oracle" "$work" "$work/expected.jsonl"
python3 "$here/permissions-probe.py" compare "$work/expected.jsonl" "$work/observed.jsonl"
