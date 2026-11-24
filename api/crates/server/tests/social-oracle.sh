#!/bin/bash
# What a client of the friends routes, the inbox and the hub sees from this
# server and from the first one, on copies of one database: the answers to a
# scripted story of every transition between two people with a third watching,
# the events each of three connections receives, the state and the inbox each
# person sees, and the rows left behind.
#
# It founds an instance on this server, makes five accounts, arranges the
# relationships and inbox rows in the database, copies the file to the first
# server's container, and runs social-oracle.mjs against each. Needs docker,
# sqlite3, curl, Node 26, and the first server's container from
# `oracle start <name> <port> Production`.
#
#   ORACLE=social ORACLE_PORT=5220 STELLARR_API=path/to/stellarr-api social-oracle.sh
#
# Prints the difference between the two transcripts and between the rows they
# left, which is empty when the servers agree on everything observed. The
# files are left in $WORK (default a new folder under the temporary directory).
set -euo pipefail

name=${ORACLE:-social}
port=${ORACLE_PORT:-5220}
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

register() {
  code -X POST "$rust/api/auth/register" "${headers[@]}" \
    -d "{\"username\":\"$1\",\"displayName\":\"$1\",\"email\":\"$1@example.test\",\"password\":\"$password\",\"inviteCode\":\"sharedinvite\"}"
}

# The accounts, made on this server in the Testing environment, which lifts
# the limits five registrations would meet.
start_rust Testing ""
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/rust/log.txt")
code -X POST "$rust/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"ola\",\"displayName\":\"ola\",\"email\":\"ola@example.test\",\"password\":\"$password\",\"setupCode\":\"$setup\"}" >/dev/null
db=$work/rust/stellarr.db
sqlite3 "$db" "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000001',(select Id from Servers),(select Id from Users),NULL,'sharedinvite',0,NULL,NULL,'2026-11-24 01:00:00')"
for person in ben cat dan eve; do register $person >/dev/null; done

id() { sqlite3 "$db" "select Id from Users where Username='$1'"; }
home=$(sqlite3 "$db" "select Id from Servers")
channel=$(sqlite3 "$db" "select Id from Channels limit 1")
other=EEEEEEEE-AAAA-4AAA-8AAA-000000000001
ben=$(id ben)
role=$(sqlite3 "$db" "select RoleId from ServerMembers limit 1")
now='2026-11-24 02:00:00'
note() { # id user server title source channel invite at
  echo "insert into UserNotifications (Id,ServerId,UserId,Source,SourcePackageId,SourcePackageName,SourceModuleId,SourceModuleName,Title,Body,Tone,TargetChannelId,TargetInviteCode,CreatedAt,ReadAt) values('$1','$3','$2','$5','pkg','Package','mod','Module','$4','Body of $4','info',$6,$7,'$8',NULL);"
}
{
  for n in $(seq 1 12); do
    note "B0000000-0000-0000-0000-0000000001$(printf %02d "$n")" "$ben" "$other" "channel-$n" system "'$channel'" NULL "2026-11-24 02:00:$(printf %02d "$n").123"
    note "B0000000-0000-0000-0000-0000000002$(printf %02d "$n")" "$ben" "$other" "plain-$n" sdk NULL NULL "2026-11-24 02:01:$(printf %02d "$n")"
  done
  note B0000000-0000-0000-0000-0000000000E0 "$ben" "$home" usable-invite system NULL "'usable'" "$now"
  note B0000000-0000-0000-0000-0000000000E1 "$ben" "$home" expired-invite system NULL "'expired'" "$now"
  note B0000000-0000-0000-0000-0000000000E2 "$ben" "$home" used-up-invite system NULL "'usedup'" "$now"
  note B0000000-0000-0000-0000-0000000000E3 "$ben" "$home" other-server-entry system NULL NULL "$now"
  note B0000000-0000-0000-0000-0000000000E4 "$ben" "$home" no-invite-row system NULL "'deleted'" "$now"
  note A0000000-0000-0000-0000-000000000001 "$(id ola)" "$home" olas sdk NULL NULL "$now"
  echo "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000011','$home','$(id ola)',NULL,'usable',0,5,'2999-01-01 00:00:00','$now');"
  echo "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000012','$home','$(id ola)',NULL,'expired',0,NULL,'2020-01-01 00:00:00','$now');"
  echo "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000013','$home','$(id ola)',NULL,'usedup',5,5,NULL,'$now');"
  # Ben follows a server of his own and cat none, so that only what the two
  # have to do with each other carries a status from one of them to the other.
  echo "delete from ServerMembers where UserId in ('$ben','$(id cat)');"
  echo "insert into ServerMembers values('$other','$ben','$role',NULL,'$now');"
  echo "insert into UserBlocks values('$(id eve)','$(id ola)','$now');"
  echo "insert into UserPreferences (UserId,DesktopNotifications,MentionPopups,CallRingtone,EmailNotifications,PushNotifications,MentionsOnly,MessageSound,JoinLeaveSound,AllowDirectMessages,AllowFriendRequests,ShowPresence,DeveloperMode,ReducedMotion,VadSensitivity) values('$(id dan)',1,1,1,0,0,0,1,1,1,0,1,0,0,50);"
} | sqlite3 "$db"
sqlite3 "$db" "pragma wal_checkpoint(truncate)" >/dev/null
kill "$pid"; wait "$pid" 2>/dev/null || true; pid=
cp "$db" "$work/shared.db"

docker stop -t 10 "$container" >/dev/null
docker cp "$work/shared.db" "$container:/tmp/data/stellarr.db"
docker start "$container" >/dev/null
wait_ready "$oracle"
start_rust Development "$work/shared.db"

# What the story left in the tables, with the ids told apart by name.
rows() { # a database file
  sqlite3 "$1" "
    create temp table names as select Id, Username as Name from Users;
    select 'friendship', (select Name from names where Id=UserAId), (select Name from names where Id=UserBId) from Friendships order by 2,3;
    select 'request', (select Name from names where Id=SenderId), (select Name from names where Id=ReceiverId), length(PairKey), substr(PairKey,1,32) < substr(PairKey,34), PairKey = lower(replace(SenderId,'-','')||':'||replace(ReceiverId,'-','')) from FriendRequests order by 2,3;
    select 'block', (select Name from names where Id=BlockerId), (select Name from names where Id=BlockedId) from UserBlocks order by 2,3;
    select 'notification', Sequence, Title, ReadAt is not null from UserNotifications order by Sequence;
  "
}

for target in oracle rust; do
  address=${!target}
  node "$here/social-oracle.mjs" --base "$address" --password "$password" --out "$work/$target.txt"
done
docker stop -t 10 "$container" >/dev/null
docker cp "$container:/tmp/data/stellarr.db" "$work/oracle.db"
docker start "$container" >/dev/null
kill "$pid"; wait "$pid" 2>/dev/null || true; pid=
rows "$work/oracle.db" > "$work/oracle-rows.txt"
rows "$work/rust/stellarr.db" > "$work/rust-rows.txt"
echo "transcripts and rows are in $work"
diff "$work/oracle.txt" "$work/rust.txt" && echo "the two servers agree on everything observed"
diff "$work/oracle-rows.txt" "$work/rust-rows.txt" && echo "the two servers left the same rows"
