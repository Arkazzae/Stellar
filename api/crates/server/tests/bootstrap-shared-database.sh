#!/bin/bash
# The bootstrap of this server and of the first one, on copies of one database.
#
# This server founds an instance and registers nine people; the file is then
# arranged to hold what the questions need: a home server of two sections and
# eight channels that not everybody sees (a section hidden from the base role,
# a channel hidden from one member, the first channel hidden from another), three
# thousand messages in the first channel with moments that tie, a second server
# that one person belongs to first, one person in no server, a disabled member
# and a member that stands in for somebody, a hidden status, a friendship and a
# block. The file goes to both servers, bootstrap-oracle.mjs asks each the same
# questions with six people connected in four statuses, and the answers, then
# the markers each left, are compared.
#
# Needs docker, sqlite3, curl, Node 26, and the first server's container from
# `oracle start <name> <port> Production` started with
# `Rtc__TurnSharedSecret=oracle-turn-secret-oracle-turn-secret-0123` (what
# `oracle` sets), which must be fresh.
#
#   ORACLE=bootstrap ORACLE_PORT=5230 STELLARR_API=path/to/stellarr-api bootstrap-shared-database.sh
#
# Prints the difference between the two, which is empty when the servers agree.
# The work folder is removed when it ends, whatever the way; KEEP=1 keeps it.
set -euo pipefail
# The platform tools' sqlite3 has no full-text search, which the messages table needs.
PATH=/usr/bin:$PATH

name=${ORACLE:-bootstrap}
port=${ORACLE_PORT:-5230}
binary=${STELLARR_API:?the path of the stellarr-api binary}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
oracle=http://127.0.0.1:$port
work=${WORK:-$(mktemp -d)}
password='Corona!2026-long'
secret=oracle-turn-secret-oracle-turn-secret-0123
headers=(-H 'X-Stellarr-Client: web' -H 'X-Stellarr-Client-Version: 2.0.0' -H 'Content-Type: application/json')
pid=
node_pid=

cleanup() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  [ -n "$node_pid" ] && kill "$node_pid" 2>/dev/null || true
  [ -n "${KEEP:-}" ] || rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
wait_ready() {
  for _ in $(seq 1 60); do [ "$(code "$1/health/ready")" = 200 ] && return; sleep 1; done
  echo "$1 did not become ready" >&2; exit 1
}
stop_rust() { [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null || true; pid=; }
start_rust() { # an environment, and a database to start on (copied) or nothing
  stop_rust
  rm -rf "$work/rust"; mkdir -p "$work/rust"
  [ -n "${2:-}" ] && cp "$2" "$work/rust/stellarr.db"
  env ASPNETCORE_ENVIRONMENT="$1" Rtc__TurnSharedSecret=$secret STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    "$binary" > "$work/rust/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 50); do grep -q 'listening on' "$work/rust/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
  wait_ready "$rust"
}
register() {
  code -X POST "$rust/api/auth/register" "${headers[@]}" \
    -d "{\"username\":\"$1\",\"displayName\":\"$2\",\"email\":\"$1@example.test\",\"password\":\"$password\",\"inviteCode\":\"sharedinvite\"}"
}

start_rust Testing ""
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/rust/log.txt")
code -X POST "$rust/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"ola\",\"displayName\":\"Ola\",\"email\":\"ola@example.test\",\"password\":\"$password\",\"setupCode\":\"$setup\"}" >/dev/null
db=$work/rust/stellarr.db
sqlite3 "$db" "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000001',(select Id from Servers),(select Id from Users),NULL,'sharedinvite',0,NULL,NULL,'2026-11-24 01:00:00')"
# Display names that tie, and two that sort by the way the letters are written.
register mia "Mia" >/dev/null
register noor "Noor" >/dev/null
register omar "Omar" >/dev/null
register pat "Mia" >/dev/null
register uma "Uma" >/dev/null
register tess "tess" >/dev/null
register sam "Sam" >/dev/null
register quinn "Ōzer" >/dev/null
register rex "Rex" >/dev/null

id() { sqlite3 "$db" "select Id from Users where Username='$1'"; }
home=$(sqlite3 "$db" "select Id from Servers")
base=$(sqlite3 "$db" "select Id from ServerRoles where IsSystem=1 and IsOwner=0")
beta=22222222-BBBB-4BBB-8BBB-000000000001
mod=11111111-AAAA-4AAA-8AAA-000000000001
sec_a=22222222-AAAA-4AAA-8AAA-00000000000A
sec_b=22222222-AAAA-4AAA-8AAA-00000000000B
ch() { echo "33333333-AAAA-4AAA-8AAA-0000000000$1"; }
general=$(sqlite3 "$db" "select Id from Channels where Name='general'")
now='2026-11-24 02:00:00'
sqlite3 "$db" "
update ServerRoles set Position=9 where Id='$base';
insert into ServerRoles values('$mod','$home','Mod','#3366ff',16898,2,0,0);
update ServerMembers set RoleId='$mod' where UserId='$(id omar)';
insert into Sections values('$sec_a','$home','Alpha',0);
insert into Sections values('$sec_b','$home','Bravo',1);
update Channels set Position=0 where Name='general';
update Channels set Position=1 where Name='voice';
insert into Channels values('$(ch 11)','$home','news',0,2,'',0,0,0,NULL);
insert into Channels values('$(ch 12)','$home','dev',0,3,'topic of dev',0,0,0,'$sec_a');
insert into Channels values('$(ch 13)','$home','design',0,4,'',30,0,0,'$sec_a');
insert into Channels values('$(ch 14)','$home','lounge',1,5,'',0,0,8,'$sec_a');
insert into Channels values('$(ch 15)','$home','secret',0,6,'',0,0,0,'$sec_b');
insert into Channels values('$(ch 16)','$home','ops',1,7,'',0,0,0,'$sec_b');
insert into PermissionOverrides values('44444444-AAAA-4AAA-8AAA-000000000001','$home',1,'$sec_b','$base',0,16384);
insert into PermissionOverrides values('44444444-AAAA-4AAA-8AAA-000000000003','$home',1,'$sec_b','$mod',16384,0);
insert into MemberPermissionOverrides values('55555555-AAAA-4AAA-8AAA-000000000002','$home',0,'$(ch 12)','$(id noor)',0,16384);
insert into MemberPermissionOverrides values('55555555-AAAA-4AAA-8AAA-000000000003','$home',0,'$general','$(id uma)',0,16384);

-- Three thousand messages in the first channel, three to a second, replies,
-- pins and edits among them; fewer in the others, and some where nobody but a
-- moderator looks.
create temp table n(i integer primary key);
with recursive c(i) as (select 1 union all select i+1 from c where i<3000) insert into n select i from c;
insert into Messages (Id,ChannelId,AuthorId,IsSystem,Content,CreatedAt,EditedAt,PinnedAt,ReplyToMessageId)
  select printf('B0000000-0000-0000-0000-%012d', i), '$general',
    case i % 4 when 0 then '$(id ola)' when 1 then '$(id mia)' when 2 then '$(id noor)' else '$(id omar)' end,
    0, 'message ' || i || case when i % 13 = 0 then ' with **bold** and @mia' else '' end,
    datetime('2026-03-01 00:00:00', printf('+%d seconds', i / 3)),
    case when i % 11 = 0 then datetime('2026-03-02 00:00:00') end,
    case when i % 500 = 0 then datetime('2026-03-02 01:00:00') end,
    case when i % 7 = 0 and i > 7 then printf('B0000000-0000-0000-0000-%012d', i - 3) end
  from n;
insert into Messages (Id,ChannelId,AuthorId,IsSystem,Content,CreatedAt,EditedAt,PinnedAt,ReplyToMessageId)
  select printf('B1000000-0000-0000-0000-%012d', i), '$(ch 11)', '$(id mia)', 0, 'news ' || i, datetime('2026-03-05 00:00:00', printf('+%d minutes', i)), NULL, NULL, NULL from n where i <= 30;
insert into Messages (Id,ChannelId,AuthorId,IsSystem,Content,CreatedAt,EditedAt,PinnedAt,ReplyToMessageId)
  select printf('B2000000-0000-0000-0000-%012d', i), '$(ch 15)', '$(id omar)', 0, 'secret ' || i, datetime('2026-03-06 00:00:00', printf('+%d minutes', i)), NULL, NULL, NULL from n where i <= 40;
insert into Messages (Id,ChannelId,AuthorId,IsSystem,Content,CreatedAt,EditedAt,PinnedAt,ReplyToMessageId)
  select printf('B3000000-0000-0000-0000-%012d', i), '$(ch 12)', '$(id noor)', 0, 'dev ' || i, datetime('2026-03-07 00:00:00', printf('+%d minutes', i)), NULL, NULL, NULL from n where i <= 10;

-- Read markers: mia has some, noor one for a channel she does not see.
insert into ChannelReads values('$general','$(id mia)','2026-03-01 00:10:00');
insert into ChannelReads values('$(ch 11)','$(id mia)','2026-03-05 00:10:00.5');
insert into ChannelReads values('$(ch 15)','$(id noor)','2026-03-06 00:10:00');

-- People: nobody's server, a disabled member, a stand-in, a hidden status, a
-- friendship and a block.
delete from ServerMembers where UserId='$(id pat)';
update Users set IsDisabled=1 where Username='quinn';
update Users set Origin='discord' where Username='rex';
insert into UserPreferences (UserId,DesktopNotifications,MentionPopups,CallRingtone,EmailNotifications,PushNotifications,MentionsOnly,MessageSound,JoinLeaveSound,AllowDirectMessages,AllowFriendRequests,ShowPresence,DeveloperMode,ReducedMotion,VadSensitivity) values('$(id sam)',1,1,1,0,0,0,1,1,1,1,0,0,0,50);
insert into Friendships values('F0000000-0000-0000-0000-000000000001','$(id mia)','$(id noor)','$now');
insert into UserBlocks values('$(id mia)','$(id tess)','$now');

-- A second server, which omar belongs to first.
insert into Servers (Id,Name,Description,IsPublic,IconUrl,CreatedAt,SoundboardEveryoneCanPlay,SoundboardRateLimited) values('$beta','Beta','The second',0,NULL,'2026-02-01 00:00:00',1,0);
insert into ServerRoles select 'BBBBBBBB'||substr(Id,9), '$beta', Name, Color, Permissions, Position, IsSystem, IsOwner from ServerRoles where ServerId='$home' and IsSystem=1;
insert into ServerMembers values('$beta','$(id omar)',(select Id from ServerRoles where ServerId='$beta' and IsOwner=1),1,'2026-01-15 00:00:00');
insert into Channels values('66666666-BBBB-4BBB-8BBB-000000000001','$beta','town-square',0,0,'',0,0,0,NULL);
insert into Channels values('66666666-BBBB-4BBB-8BBB-000000000002','$beta','hall',1,1,'',0,0,0,NULL);
insert into Messages (Id,ChannelId,AuthorId,IsSystem,Content,CreatedAt,EditedAt,PinnedAt,ReplyToMessageId)
  select printf('B4000000-0000-0000-0000-%012d', i), '66666666-BBBB-4BBB-8BBB-000000000001', '$(id omar)', 0, 'beta ' || i, datetime('2026-03-08 00:00:00', printf('+%d minutes', i)), NULL, NULL, NULL from n where i <= 5;
"
sqlite3 "$db" "pragma wal_checkpoint(truncate)" >/dev/null
stop_rust
cp "$db" "$work/shared.db"
rm -rf "$work/rust"

docker stop -t 10 "$container" >/dev/null
docker cp "$work/shared.db" "$container:/tmp/data/stellarr.db"
docker start "$container" >/dev/null
wait_ready "$oracle"
start_rust Development "$work/shared.db"

# The markers each left, by person and channel name.
markers() {
  sqlite3 "$1" "select (select Username from Users where Id=r.UserId), (select Name from Channels where Id=r.ChannelId), r.LastReadAt is not null,
                  r.LastReadAt >= '2026-11-24' from ChannelReads r order by 1, 2"
}
for target in oracle rust; do
  address=${!target}
  node "$here/bootstrap-oracle.mjs" --base "$address" --home "$home" --beta "$beta" --secret "$secret" --password "$password" --out "$work/$target.txt" &
  node_pid=$!; wait "$node_pid"; node_pid=
done
docker stop -t 10 "$container" >/dev/null
docker cp "$container:/tmp/data/stellarr.db" "$work/oracle-final.db"
stop_rust
markers "$work/oracle-final.db" > "$work/oracle-markers.txt"
markers "$work/rust/stellarr.db" > "$work/rust-markers.txt"
status=0
diff "$work/oracle.txt" "$work/rust.txt" && echo "the two servers agree on every answer" || status=1
diff "$work/oracle-markers.txt" "$work/rust-markers.txt" && echo "the two servers left the same markers" || status=1
exit $status
