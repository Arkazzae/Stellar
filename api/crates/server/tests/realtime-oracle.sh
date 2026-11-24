#!/bin/bash
# What a client of the hub sees from this server and from the first one, on
# copies of one database: the records after connecting, the answers to each
# method, the events other connections of the same account and of other
# accounts receive, and the close on sign-out.
#
# It founds an instance on this server, makes six accounts and their
# relationships in the database, copies the file to the first server's
# container, and runs realtime-oracle.mjs against each. Needs docker, sqlite3,
# curl, Node 26, and the first server's container from
# `oracle start <name> <port> Development`.
#
#   ORACLE=realtime ORACLE_PORT=5202 STELLARR_API=path/to/stellarr-api realtime-oracle.sh
#
# Prints the difference between the two transcripts, which is empty when the
# servers agree on everything the script observes. The transcripts are left in
# $WORK (default a new folder under the temporary directory).
set -euo pipefail

name=${ORACLE:-realtime}
port=${ORACLE_PORT:-5202}
binary=${STELLARR_API:?the path of the stellarr-api binary}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
oracle=http://127.0.0.1:$port
work=${WORK:-$(mktemp -d)}
password='Corona!2026-long'
headers=(-H 'X-Stellarr-Client: web' -H 'X-Stellarr-Client-Version: 2.0.0' -H 'Content-Type: application/json')
pid=

cleanup() { [ -n "$pid" ] && kill "$pid" 2>/dev/null || true; }
trap cleanup EXIT

code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
wait_ready() {
  for _ in $(seq 1 60); do [ "$(code "$1/health/ready")" = 200 ] && return; sleep 1; done
  echo "$1 did not become ready" >&2; exit 1
}

start_rust() { # an environment, and a database to start on (copied) or nothing
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null || true
  rm -rf "$work/rust"; mkdir -p "$work/rust"
  [ -n "${2:-}" ] && cp "$2" "$work/rust/stellarr.db"
  env ASPNETCORE_ENVIRONMENT="$1" Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    "$binary" > "$work/rust/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 50); do grep -q 'Now listening on' "$work/rust/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
  wait_ready "$rust"
}

register() { # name
  code -X POST "$rust/api/auth/register" "${headers[@]}" \
    -d "{\"username\":\"$1\",\"displayName\":\"$1\",\"email\":\"$1@example.test\",\"password\":\"$password\",\"inviteCode\":\"sharedinvite\"}"
}

# The Testing environment lifts the limits that six registrations would meet.
start_rust Testing ""
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/rust/log.txt")
code -X POST "$rust/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"ola\",\"displayName\":\"ola\",\"email\":\"ola@example.test\",\"password\":\"$password\",\"setupCode\":\"$setup\"}" >/dev/null
db=$work/rust/stellarr.db
sqlite3 "$db" "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000001',(select Id from Servers),(select Id from Users),NULL,'sharedinvite',0,NULL,NULL,'2026-11-24 01:00:00')"
for person in ben cat dan eve fay; do register $person >/dev/null; done

id() { sqlite3 "$db" "select Id from Users where Username='$1'"; }
first=$(sqlite3 "$db" "select Id from Servers")
third=11111111-AAAA-4AAA-8AAA-333333333333
role=$(sqlite3 "$db" "select RoleId from ServerMembers limit 1")
now='2026-11-24 02:00:00'
sqlite3 "$db" "
insert into Friendships values('F0000000-0000-0000-0000-000000000001','$(id ola)','$(id ben)','$now');
insert into Friendships values('F0000000-0000-0000-0000-000000000002','$(id ola)','$(id dan)','$now');
insert into DirectConversations (Id,UserAId,UserBId,CreatedAt,EncryptionState) values('C0000000-0000-0000-0000-000000000001','$(id ola)','$(id cat)','$now','Off');
insert into UserBlocks values('$(id dan)','$(id ola)','$now');
delete from ServerMembers where UserId='$(id cat)';
insert into ServerMembers values('$third','$(id ola)','$role',NULL,'$now');
insert into ServerMembers values('$third','$(id eve)','$role',NULL,'$now');
insert into ServerMembers values('$third','$(id fay)','$role',NULL,'$now');
insert into UserPreferences (UserId,DesktopNotifications,MentionPopups,CallRingtone,EmailNotifications,PushNotifications,MentionsOnly,MessageSound,JoinLeaveSound,AllowDirectMessages,AllowFriendRequests,ShowPresence,DeveloperMode,ReducedMotion,VadSensitivity) values('$(id eve)',1,1,1,0,0,0,1,1,1,1,0,0,0,50);
"
sqlite3 "$db" "pragma wal_checkpoint(truncate)" >/dev/null
kill "$pid"; wait "$pid" 2>/dev/null || true; pid=
cp "$db" "$work/shared.db"

docker stop -t 10 "$container" >/dev/null
docker cp "$work/shared.db" "$container:/tmp/data/stellarr.db"
docker start "$container" >/dev/null
wait_ready "$oracle"
start_rust Development "$work/shared.db"

for target in oracle rust; do
  address=${!target}
  node "$here/realtime-oracle.mjs" --base "$address" --server1 "$first" --server3 "$third" --password "$password" --out "$work/$target.txt"
done
echo "transcripts: $work/oracle.txt (first server), $work/rust.txt (this one)"
diff "$work/oracle.txt" "$work/rust.txt" && echo "the two servers agree on everything observed"
