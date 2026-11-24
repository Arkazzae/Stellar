#!/bin/bash
# What a client sees from this server and from the first one, on copies of one
# database, when it administers channels and sections: the answers, the events
# the connected people are sent, and the rows (channels, sections, the audit
# log, read markers) after every step.
#
# It founds an instance on this server, makes five accounts, arranges a server
# of eight channels in two sections where nobody but the owner sees them all
# (one account manages part of the channels, one only a section, one sees
# nothing of a section, one manages nothing), copies the file to the first
# server's container, and runs channels-oracle.mjs against each, the first
# server first to learn what each account sees. Needs docker, sqlite3, curl,
# Node 26, and the first server's container from `oracle start <name> <port>
# Development`, which must be fresh.
#
#   ORACLE=channels ORACLE_PORT=5211 STELLARR_API=path/to/stellarr-api channels-shared-database.sh
#
# Prints the difference between the two transcripts, which is empty when the
# servers agree on everything the script observes. They are left in $WORK.
set -euo pipefail

name=${ORACLE:-channels}
port=${ORACLE_PORT:-5211}
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

start_rust() { # a database to start on (copied) or nothing
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null || true
  rm -rf "$work/rust"; mkdir -p "$work/rust"
  [ -n "${1:-}" ] && cp "$1" "$work/rust/stellarr.db"
  env ASPNETCORE_ENVIRONMENT=Testing Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    "$binary" > "$work/rust/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 50); do grep -q 'listening on' "$work/rust/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
  wait_ready "$rust"
}

register() { # name
  code -X POST "$rust/api/auth/register" "${headers[@]}" \
    -d "{\"username\":\"$1\",\"displayName\":\"$1\",\"email\":\"$1@example.test\",\"password\":\"$password\",\"inviteCode\":\"sharedinvite\"}"
}

start_rust ""
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/rust/log.txt")
code -X POST "$rust/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"ola\",\"displayName\":\"ola\",\"email\":\"ola@example.test\",\"password\":\"$password\",\"setupCode\":\"$setup\"}" >/dev/null
db=$work/rust/stellarr.db
sqlite3 "$db" "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000001',(select Id from Servers),(select Id from Users),NULL,'sharedinvite',0,NULL,NULL,'2026-11-24 01:00:00')"
for person in ben cat dan eve; do register $person >/dev/null; done

id() { sqlite3 "$db" "select Id from Users where Username='$1'"; }
server=$(sqlite3 "$db" "select Id from Servers")
base=$(sqlite3 "$db" "select Id from ServerRoles where IsSystem=1 and IsOwner=0")
# Permission bits: manageChannels 2, sendMessages 512, viewChannel 16384.
mod=11111111-AAAA-4AAA-8AAA-000000000001
secb=11111111-AAAA-4AAA-8AAA-000000000002
sec_a=22222222-AAAA-4AAA-8AAA-00000000000A
sec_b=22222222-AAAA-4AAA-8AAA-00000000000B
ch() { echo "33333333-AAAA-4AAA-8AAA-0000000000$1"; }
now='2026-11-24 02:00:00'
sqlite3 "$db" "
update ServerRoles set Position=9 where Id='$base';
insert into ServerRoles values('$mod','$server','Mod','#3366ff',16898,2,0,0);
insert into ServerRoles values('$secb','$server','Section B','#33cc66',512,3,0,0);
update ServerMembers set RoleId='$mod' where UserId in ('$(id ben)','$(id eve)');
update ServerMembers set RoleId='$secb' where UserId='$(id dan)';
insert into Sections values('$sec_a','$server','Alpha',0);
insert into Sections values('$sec_b','$server','Bravo',1);
update Channels set Position=0 where Name='general';
update Channels set Position=1 where Name='voice';
insert into Channels values('$(ch 11)','$server','news',0,2,'',0,0,0,NULL);
insert into Channels values('$(ch 12)','$server','dev',0,3,'',0,0,0,'$sec_a');
insert into Channels values('$(ch 13)','$server','design',0,4,'',0,0,0,'$sec_a');
insert into Channels values('$(ch 14)','$server','lounge',1,5,'',0,0,0,'$sec_a');
insert into Channels values('$(ch 15)','$server','secret',0,6,'',0,0,0,'$sec_b');
insert into Channels values('$(ch 16)','$server','ops',1,7,'',0,0,0,'$sec_b');
-- Bravo is hidden from everybody but those who are let in: the section's
-- own managers and the moderators (eve is turned away by a rule of her own).
insert into PermissionOverrides values('44444444-AAAA-4AAA-8AAA-000000000001','$server',1,'$sec_b','$base',0,16384);
insert into PermissionOverrides values('44444444-AAAA-4AAA-8AAA-000000000002','$server',1,'$sec_b','$secb',16386,0);
insert into PermissionOverrides values('44444444-AAAA-4AAA-8AAA-000000000003','$server',1,'$sec_b','$mod',16384,0);
insert into MemberPermissionOverrides values('55555555-AAAA-4AAA-8AAA-000000000001','$server',1,'$sec_b','$(id eve)',0,16384);
-- ben may not see two channels of Alpha.
insert into MemberPermissionOverrides values('55555555-AAAA-4AAA-8AAA-000000000002','$server',0,'$(ch 12)','$(id ben)',0,16384);
insert into MemberPermissionOverrides values('55555555-AAAA-4AAA-8AAA-000000000003','$server',0,'$(ch 14)','$(id ben)',0,16384);
-- cat may not read news, which dan may not write in.
insert into MemberPermissionOverrides values('55555555-AAAA-4AAA-8AAA-000000000004','$server',0,'$(ch 11)','$(id cat)',0,16384);
insert into MemberPermissionOverrides values('55555555-AAAA-4AAA-8AAA-000000000005','$server',0,'$(ch 11)','$(id dan)',0,512);
"
sqlite3 "$db" "pragma wal_checkpoint(truncate)" >/dev/null
kill "$pid"; wait "$pid" 2>/dev/null || true; pid=
cp "$db" "$work/shared.db"

docker stop -t 10 "$container" >/dev/null
docker cp "$work/shared.db" "$container:/tmp/data/stellarr.db"
docker start "$container" >/dev/null
wait_ready "$oracle"
start_rust "$work/shared.db"

common=(--server "$server" --password "$password" --visible "$work/visible.json")
node "$here/channels-oracle.mjs" --base "$oracle" "${common[@]}" --discover
for target in oracle rust; do
  address=${!target}
  extra=()
  [ "$target" = oracle ] && extra=(--container "$container") || extra=(--database "$work/rust/stellarr.db")
  node "$here/channels-oracle.mjs" --base "$address" "${common[@]}" "${extra[@]}" --out "$work/$target.txt"
done
echo "transcripts: $work/oracle.txt (first server), $work/rust.txt (this one)"
diff "$work/oracle.txt" "$work/rust.txt" && echo "the two servers agree on everything observed"
