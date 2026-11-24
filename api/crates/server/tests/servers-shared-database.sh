#!/bin/bash
# The servers routes against the first server on one database file.
#
# The first server is started fresh and seeded through its own routes
# (servers-oracle.mjs seed): accounts, servers, roles, a channel hidden from
# plain members, invites, bans, notifications, content and a long audit
# history. It is stopped and its database copied twice. The first server runs
# on one copy and this server on the other, both are asked the same questions
# (servers-oracle.mjs probe), and the two transcripts are compared, then the
# two databases, table by table, with ids and moments replaced.
#
# Needs docker, node 26, python3, and a fresh first server container named
# by `oracle start <name> <port> Production` (the script starts it itself when
# ORACLE_START=1). Not part of the ordinary run.
#
#   ORACLE=servers ORACLE_PORT=5212 STELLARR_API=path/to/stellarr-api \
#     ORACLE_ROOT=path/to/the/oracle/script/folder servers-shared-database.sh
set -euo pipefail

name=${ORACLE:-servers}
port=${ORACLE_PORT:-5212}
binary=${STELLARR_API:?the path of the stellarr-api binary}
oracle_script=${ORACLE_SCRIPT:?the path of the oracle script}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
work=${WORK:-$(mktemp -d)}
oracle=http://127.0.0.1:$port
pid=
trap '[ -n "$pid" ] && kill "$pid" 2>/dev/null || true' EXIT

settings=$work/oracle.env
printf 'RateLimiting__AuthWrite__PermitLimit=500\nRateLimiting__Login__PermitLimit=500\nRateLimiting__InvitePreview__PermitLimit=500\n' > "$settings"
ready() { for _ in $(seq 1 90); do [ "$(curl -s -o /dev/null -w '%{http_code}' "$1/health/ready")" = 200 ] && return; sleep 1; done; echo "$1 is not ready" >&2; exit 1; }

"$oracle_script" stop "$name" >/dev/null 2>&1 || true
docker rm -f "$container" >/dev/null 2>&1 || true
ORACLE_ENV_FILE=$settings "$oracle_script" start "$name" "$port" Production > "$work/start.log"
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/start.log" | tail -1)
node "$here/servers-oracle.mjs" seed --base "$oracle" --setup "$setup" --state "$work/state.json"

docker stop -t 20 "$container" >/dev/null
docker cp "$container:/tmp/data/stellarr.db" "$work/seeded.db"
cp "$work/seeded.db" "$work/oracle.db"
docker cp "$work/oracle.db" "$container:/tmp/data/stellarr.db"
docker start "$container" >/dev/null
ready "$oracle"

mkdir -p "$work/rust"
cp "$work/seeded.db" "$work/rust/stellarr.db"
env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
  "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
  RateLimiting__AuthWrite__PermitLimit=500 RateLimiting__Login__PermitLimit=500 RateLimiting__InvitePreview__PermitLimit=500 \
  "Operations__PublicUrl=http://127.0.0.1:$port" \
  "$binary" > "$work/rust/log.txt" 2>&1 &
pid=$!
for _ in $(seq 1 50); do grep -q 'Now listening on' "$work/rust/log.txt" && break; sleep 0.2; done
rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
ready "$rust"

node "$here/servers-oracle.mjs" probe --base "$oracle" --state "$work/state.json" --out "$work/first.txt" --public "http://127.0.0.1:$port" &
first=$!
node "$here/servers-oracle.mjs" probe --base "$rust" --state "$work/state.json" --out "$work/this.txt" --public "http://127.0.0.1:$port" &
second=$!
wait $first $second
echo "== transcripts"
diff "$work/first.txt" "$work/this.txt" && echo "the transcripts are the same"

kill "$pid"; wait "$pid" 2>/dev/null || true; pid=
docker stop -t 20 "$container" >/dev/null
docker cp "$container:/tmp/data/stellarr.db" "$work/after-first.db"
echo "== databases"
python3 "$here/servers-tables.py" "$work/after-first.db" "$work/rust/stellarr.db" "$work/seeded.db"

# The second step: accounts that have one, a session that passed it lately and
# sessions that did not, on fresh copies of the seeded file.
echo "== second step"
restore_oracle() { docker stop -t 20 "$container" >/dev/null; docker cp "$1" "$container:/tmp/data/stellarr.db"; docker start "$container" >/dev/null; ready "$oracle"; }
start_rust() {
  [ -n "$pid" ] && { kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null || true; }
  : > "$work/rust/log.txt"
  env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    RateLimiting__AuthWrite__PermitLimit=500 RateLimiting__Login__PermitLimit=500 \
    "Operations__PublicUrl=http://127.0.0.1:$port" "$binary" > "$work/rust/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 50); do grep -q 'Now listening on' "$work/rust/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
  ready "$rust"
}
edit() { # a database file: maya passed the second step a minute ago; kim, ilya and ada's sessions did not (ada has none)
  sqlite3 "$1" "
    INSERT INTO MfaMethods (Id, UserId, Type, Name, CreatedAt, ConfirmedAt)
      SELECT upper(hex(randomblob(4))) || '-0000-4000-8000-' || upper(hex(randomblob(6))), Id, 'totp', 'Phone', datetime('now'), datetime('now')
      FROM Users WHERE Username IN ('maya', 'kim', 'ilya');
    UPDATE UserSessions SET MfaCompletedAt = strftime('%Y-%m-%d %H:%M:%f', 'now', '-60 seconds') WHERE UserId = (SELECT Id FROM Users WHERE Username = 'maya');
    UPDATE UserSessions SET MfaCompletedAt = NULL WHERE UserId IN (SELECT Id FROM Users WHERE Username IN ('kim', 'ilya'));"
}
cp "$work/seeded.db" "$work/stale-oracle.db"; cp "$work/seeded.db" "$work/rust/stellarr.db"
restore_oracle "$work/stale-oracle.db"; start_rust
node "$here/servers-oracle.mjs" stale-login --base "$oracle" --out "$work/jar-first.json"
node "$here/servers-oracle.mjs" stale-login --base "$rust" --out "$work/jar-this.json"
docker stop -t 20 "$container" >/dev/null; docker cp "$container:/tmp/data/stellarr.db" "$work/stale-oracle.db"
kill "$pid"; wait "$pid" 2>/dev/null || true; pid=
edit "$work/stale-oracle.db"; edit "$work/rust/stellarr.db"
docker cp "$work/stale-oracle.db" "$container:/tmp/data/stellarr.db"; docker start "$container" >/dev/null; ready "$oracle"; start_rust
node "$here/servers-oracle.mjs" stale-ask --base "$oracle" --state "$work/state.json" --cookies "$work/jar-first.json" --out "$work/stale-first.txt" --public "http://127.0.0.1:$port"
node "$here/servers-oracle.mjs" stale-ask --base "$rust" --state "$work/state.json" --cookies "$work/jar-this.json" --out "$work/stale-this.txt" --public "http://127.0.0.1:$port"
diff "$work/stale-first.txt" "$work/stale-this.txt" && echo "the second-step transcripts are the same"
docker stop -t 20 "$container" >/dev/null
echo "work: $work"
