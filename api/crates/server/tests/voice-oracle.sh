#!/bin/bash
# What clients of the hub and of the moderation routes see of voice from this
# server and from the first one, each with a fake media server behind it that
# records every request: every reply, every event each client receives (in
# order, per client), every request made to the control API, and the rows the
# routes leave.
#
# It founds an instance on this server, makes six accounts, their rooms, roles,
# calls and sounds in the database, starts the first server with the `oracle`
# script (and removes it again), copies the database into it, starts a fake
# media server for each (one bound to every address so that the container can
# reach it), and runs voice-oracle.mjs against each. Needs docker, sqlite3,
# curl and Node 26. Ports 5231 and 5232 are the fakes'.
#
#   ORACLE=voice ORACLE_PORT=5217 STELLARR_API=path/to/stellarr-api \
#   FAKE_MEDIA=path/to/examples/fake_media ORACLE_SCRIPT=path/to/oracle \
#   voice-oracle.sh [section,section]
#
# LIFECYCLE=1 also runs voice-lifecycle.sh on the same database. KEEP_WORK=1
# leaves the work folder, which holds both transcripts; without it the folder
# is removed, with the transcripts printed first. Prints the difference between
# the two transcripts, which is empty when the servers agree on everything the
# script observes. Every server and fake it started is stopped on every way out.
set -euo pipefail

name=${ORACLE:-voice}
port=${ORACLE_PORT:-5217}
binary=${STELLARR_API:?the path of the stellarr-api binary}
fake=${FAKE_MEDIA:?the path of the fake_media example}
only=${1:-}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
oracle=http://127.0.0.1:$port
oracle_script=${ORACLE_SCRIPT:-oracle}
work=${WORK:-$(mktemp -d)}
password='Corona!2026-long'
headers=(-H 'X-Stellarr-Client: web' -H 'X-Stellarr-Client-Version: 2.0.0' -H 'Content-Type: application/json')
pids=()

cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  "$oracle_script" stop "$name" >/dev/null 2>&1 || true
  [ -n "${KEEP_WORK:-}" ] || rm -rf "$work"
}
trap cleanup EXIT INT TERM
printf 'Rtc__MediaControlUrl=http://host.docker.internal:5231\nRtc__MediaControlSecret=media-test-secret\nRtc__MediaPort=5236\nRateLimiting__AuthWrite__PermitLimit=500\nRateLimiting__Login__PermitLimit=500\n' > "$work/oracle.env"
ORACLE_ENV_FILE="$work/oracle.env" "$oracle_script" start "$name" "$port" Development

code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
wait_ready() {
  for _ in $(seq 1 90); do [ "$(code "$1/health/ready")" = 200 ] && return; sleep 1; done
  echo "$1 did not become ready" >&2; exit 1
}

start_rust() { # an environment, a database to start on (copied) or nothing, the address of its fake media server
  rm -rf "$work/rust"; mkdir -p "$work/rust"
  [ -n "${2:-}" ] && cp "$2" "$work/rust/stellarr.db"
  env ASPNETCORE_ENVIRONMENT="$1" Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    Rtc__MediaControlUrl="${3:-http://127.0.0.1:5232}" Rtc__MediaControlSecret=media-test-secret \
    "$binary" > "$work/rust/log.txt" 2>&1 &
  rust_pid=$!
  pids+=("$rust_pid")
  for _ in $(seq 1 50); do grep -q 'Now listening on' "$work/rust/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
  wait_ready "$rust"
}

register() { # name
  code -X POST "$rust/api/auth/register" "${headers[@]}" \
    -d "{\"username\":\"$1\",\"displayName\":\"$1\",\"email\":\"$1@example.test\",\"password\":\"$password\",\"inviteCode\":\"sharedinvite\"}"
}

# Two fakes: the first server's behind every address, this one's on the loopback.
"$fake" 0.0.0.0:5231 > "$work/fake-oracle.txt" 2>&1 & pids+=($!)
"$fake" 127.0.0.1:5232 > "$work/fake-rust.txt" 2>&1 & pids+=($!)
sleep 1

# The Testing environment lifts the limits that six registrations would meet.
start_rust Testing "" http://127.0.0.1:5232
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/rust/log.txt")
code -X POST "$rust/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"ola\",\"displayName\":\"ola\",\"email\":\"ola@example.test\",\"password\":\"$password\",\"setupCode\":\"$setup\"}" >/dev/null
db=$work/rust/stellarr.db
sqlite3 "$db" "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000001',(select Id from Servers),(select Id from Users),NULL,'sharedinvite',0,NULL,NULL,'2026-11-24 01:00:00')"
for person in ben cat dan eve fay; do register $person >/dev/null; done

id() { sqlite3 "$db" "select Id from Users where Username='$1'"; }
first=$(sqlite3 "$db" "select Id from Servers")
v1=$(sqlite3 "$db" "select Id from Channels where Kind=1 order by Position limit 1")
t1=$(sqlite3 "$db" "select Id from Channels where Kind=0 order by Position limit 1")
base=$(sqlite3 "$db" "select Id from ServerRoles where IsSystem=1 and IsOwner=0")
v2=20000000-0000-4000-8000-000000000002; v3=20000000-0000-4000-8000-000000000003
v4=20000000-0000-4000-8000-000000000004; v5=20000000-0000-4000-8000-000000000005
v6=20000000-0000-4000-8000-000000000006; far=20000000-0000-4000-8000-000000000007
third=11111111-AAAA-4AAA-8AAA-333333333333; fourth=11111111-AAAA-4AAA-8AAA-444444444444
role_mod=60000000-0000-4000-8000-000000000001; role_senior=60000000-0000-4000-8000-000000000002
role3=60000000-0000-4000-8000-000000000003; role3staff=60000000-0000-4000-8000-000000000004; role4=60000000-0000-4000-8000-000000000005
sound1=70000000-0000-4000-8000-000000000001; sound3=70000000-0000-4000-8000-000000000003
now='2026-11-24 02:00:00'
conv() { # id first second: stored with the lower id first, as conversations are made
  if [[ "$2" < "$3" ]]; then echo "insert into DirectConversations (Id,UserAId,UserBId,CreatedAt,EncryptionState) values('$1','$2','$3','$now','Off');"; else echo "insert into DirectConversations (Id,UserAId,UserBId,CreatedAt,EncryptionState) values('$1','$3','$2','$now','Off');"; fi
}
sqlite3 "$db" "$(conv C0000000-0000-4000-8000-000000000001 "$(id ola)" "$(id cat)")
$(conv C0000000-0000-4000-8000-000000000002 "$(id ola)" "$(id dan)")
$(conv C0000000-0000-4000-8000-000000000003 "$(id ben)" "$(id eve)")
$(conv C0000000-0000-4000-8000-000000000004 "$(id ben)" "$(id fay)")"
sqlite3 "$db" "
update ServerRoles set Position=100 where Id='$base';
insert into ServerRoles values('$role_mod','$first','Mod','#123456',32,5,0,0);
insert into ServerRoles values('$role_senior','$first','Senior','#654321',32,3,0,0);
insert into ServerMemberRoles values('$first','$(id ben)','$role_mod');
insert into ServerMemberRoles values('$first','$(id cat)','$role_senior');
update ServerMembers set RoleId='$role_mod' where UserId='$(id ben)';
update ServerMembers set RoleId='$role_senior' where UserId='$(id cat)';
insert into Channels values('$v2','$first','night-shift',1,10,'',0,0,0,NULL);
insert into Channels values('$v3','$first','secret',1,11,'',0,0,0,NULL);
insert into Channels values('$v4','$first','tiny',1,12,'',0,0,1,NULL);
insert into Channels values('$v5','$first','quiet',1,13,'',0,0,0,NULL);
insert into MemberPermissionOverrides values('F0000000-0000-4000-8000-000000000001','$first',0,'$v2','$(id eve)',0,4096);
insert into MemberPermissionOverrides values('F0000000-0000-4000-8000-000000000002','$first',0,'$v2','$(id fay)',0,16384);
insert into PermissionOverrides values('F0000000-0000-4000-8000-000000000003','$first',0,'$v3','$base',0,16384);
insert into PermissionOverrides values('F0000000-0000-4000-8000-000000000004','$first',0,'$v5','$base',0,4096);
insert into Servers values('$third','Third','',0,NULL,'$now',0,0);
insert into ServerRoles values('$role3','$third','Member','#777777',24064,1,1,0);
insert into ServerRoles values('$role3staff','$third','Staff','#777778',32,0,0,0);
insert into ServerMembers values('$third','$(id ola)','$role3',NULL,'2026-11-24 03:00:00');
insert into ServerMembers values('$third','$(id eve)','$role3',NULL,'2026-11-24 03:00:00');
insert into ServerMembers values('$third','$(id fay)','$role3staff',NULL,'2026-11-24 03:00:00');
insert into ServerMemberRoles values('$third','$(id fay)','$role3staff');
insert into Channels values('$v6','$third','third-voice',1,0,'',0,0,0,NULL);
insert into Servers values('$fourth','Fourth','',0,NULL,'$now',1,1);
insert into ServerRoles values('$role4','$fourth','Member','#777777',24064,1,1,0);
insert into Channels values('$far','$fourth','far-voice',1,0,'',0,0,0,NULL);
insert into ImageAttachments (Id,UploadedById,ObjectKey,FileName,ContentType,SizeBytes,Width,Height,DurationSeconds,IsVoiceMessage,IsEncrypted,CreatedAt) values('80000000-0000-4000-8000-000000000001','$(id ola)','k1','a.wav','audio/wav',16,NULL,NULL,3.0,0,0,'$now');
insert into ImageAttachments (Id,UploadedById,ObjectKey,FileName,ContentType,SizeBytes,Width,Height,DurationSeconds,IsVoiceMessage,IsEncrypted,CreatedAt) values('80000000-0000-4000-8000-000000000003','$(id ola)','k3','b.wav','audio/wav',16,NULL,NULL,NULL,0,0,'$now');
insert into SoundboardSounds values('$sound1','$first','80000000-0000-4000-8000-000000000001','$(id ola)','Ping','x',0.5,12.0,0,'$now');
insert into SoundboardSounds values('$sound3','$third','80000000-0000-4000-8000-000000000003','$(id ola)','Third','y',0,2,0,'$now');
insert into UserPreferences (UserId,DesktopNotifications,MentionPopups,CallRingtone,EmailNotifications,PushNotifications,MentionsOnly,MessageSound,JoinLeaveSound,AllowDirectMessages,AllowFriendRequests,ShowPresence,DeveloperMode,ReducedMotion,VadSensitivity,JoinSoundId) values('$(id ola)',1,1,1,0,0,0,1,1,1,1,1,0,0,50,'$sound1');
insert into UserPreferences (UserId,DesktopNotifications,MentionPopups,CallRingtone,EmailNotifications,PushNotifications,MentionsOnly,MessageSound,JoinLeaveSound,AllowDirectMessages,AllowFriendRequests,ShowPresence,DeveloperMode,ReducedMotion,VadSensitivity) values('$(id eve)',1,1,1,0,0,0,1,1,0,1,1,0,0,50);
insert into UserBlocks values('$(id dan)','$(id ola)','$now');
"
ben=$(id ben); fay=$(id fay)
if [[ "$ben" < "$fay" ]]; then a=$ben; b=$fay; else a=$fay; b=$ben; fi
sqlite3 "$db" "insert into Friendships values('F0000000-0000-4000-8000-000000000009','$a','$b','$now');"
sqlite3 "$db" "pragma wal_checkpoint(truncate)" >/dev/null
kill "$rust_pid"; wait "$rust_pid" 2>/dev/null || true
cp "$db" "$work/shared.db"
cat > "$work/ids.json" <<JSON
{"server1":"$first","server3":"$third","v1":"$v1","v2":"$v2","v3":"$v3","v4":"$v4","v5":"$v5","v6":"$v6","far":"$far","t1":"$t1",
 "conv_oc":"C0000000-0000-4000-8000-000000000001","conv_od":"C0000000-0000-4000-8000-000000000002","conv_be":"C0000000-0000-4000-8000-000000000003","conv_bf":"C0000000-0000-4000-8000-000000000004",
 "sound1":"$sound1","sound3":"$sound3","ola":"$(id ola)","ben":"$ben","cat":"$(id cat)","dan":"$(id dan)","eve":"$(id eve)","fay":"$fay"}
JSON

docker stop -t 10 "$container" >/dev/null
docker cp "$work/shared.db" "$container:/tmp/data/stellarr.db"
docker start "$container" >/dev/null
wait_ready "$oracle"
start_rust Development "$work/shared.db" http://127.0.0.1:5232
sleep 3

for target in oracle rust; do
  if [ "$target" = oracle ]; then address=$oracle; media=http://127.0.0.1:5231; else address=$rust; media=http://127.0.0.1:5232; fi
  curl -s "$media/_fake/clear" >/dev/null
  node "$here/voice-oracle.mjs" --base "$address" --fake "$media" --ids "$work/ids.json" --out "$work/$target.txt" --only "$only" --password "$password"
done

# The rows the routes left, from both databases.
rows() { sqlite3 "$1" "select Action, Summary, TargetType, ChangesJson from ServerAuditEntries where Action like 'voice.%' order by CreatedAt, Action"; }
kill "$rust_pid"; wait "$rust_pid" 2>/dev/null || true
docker stop -t 10 "$container" >/dev/null
mkdir -p "$work/oracle-db"
docker cp "$container:/tmp/data/stellarr.db" "$work/oracle-db/stellarr.db"
docker cp "$container:/tmp/data/stellarr.db-wal" "$work/oracle-db/stellarr.db-wal" 2>/dev/null || true
docker cp "$container:/tmp/data/stellarr.db-shm" "$work/oracle-db/stellarr.db-shm" 2>/dev/null || true
{ echo "# audit rows"; rows "$work/oracle-db/stellarr.db"; } >> "$work/oracle.txt"
{ echo "# audit rows"; rows "$work/rust/stellarr.db"; } >> "$work/rust.txt"
docker start "$container" >/dev/null
docker stop -t 5 "$container" >/dev/null 2>&1 || true
echo "transcripts: $work/oracle.txt (first server), $work/rust.txt (this one)"
status=0
diff "$work/oracle.txt" "$work/rust.txt" && echo "the two servers agree on everything observed" || status=1
echo "lines: $(wc -l < "$work/oracle.txt") and $(wc -l < "$work/rust.txt")"
if [ -n "${LIFECYCLE:-}" ]; then
  ORACLE_EXTERNAL=1 SHARED="$work" bash "$here/voice-lifecycle.sh" || status=1
fi
exit $status
