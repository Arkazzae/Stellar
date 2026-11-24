#!/bin/bash
# The direct conversations of this server and of the first one, on copies of
# one database file.
#
# This server founds an instance, makes seven accounts and arranges uploads
# and settings (the person who takes messages from friends only, a disabled
# one, one that stands in for somebody). The file goes to the first server,
# which turns encryption on in two conversations through its own routes
# (`dms-oracle.mjs prepare`). The file that comes back is copied to each
# server, and each is asked the same story with four hub connections open
# (`dms-oracle.mjs story`): the answers, the events every connection hears in
# order and the state of each person are compared, then the rows the story
# left. Last, the file the first server ended with goes to both, history is
# paged to its end at several sizes on each, and each server's cursors are
# asked of the other.
#
# Needs docker, sqlite3, curl, Node 26, and the first server's container from
# `oracle start <name> <port> Production`, which must be fresh.
#
#   ORACLE=dms ORACLE_PORT=5225 STELLARR_API=path/to/stellarr-api dms-oracle.sh
#
# Prints the differences, which are none when the servers agree on everything.
# The work folder is removed when it ends, whatever the way; KEEP=1 keeps it.
set -euo pipefail

name=${ORACLE:-dms}
port=${ORACLE_PORT:-5225}
binary=${STELLARR_API:?the path of the stellarr-api binary}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
oracle=http://127.0.0.1:$port
work=${WORK:-$(mktemp -d)}
password='Corona!2026-long'
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
  env ASPNETCORE_ENVIRONMENT="$1" Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    "$binary" > "$work/rust/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 50); do grep -q 'Now listening on' "$work/rust/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
  wait_ready "$rust"
}
give_oracle() { docker stop -t 10 "$container" >/dev/null; docker cp "$1" "$container:/tmp/data/stellarr.db"; docker start "$container" >/dev/null; wait_ready "$oracle"; }
take_oracle() { docker stop -t 10 "$container" >/dev/null; docker cp "$container:/tmp/data/stellarr.db" "$1"; docker start "$container" >/dev/null; wait_ready "$oracle"; }
node_run() { node "$here/dms-oracle.mjs" "$@" & node_pid=$!; wait "$node_pid"; node_pid=; }

register() {
  code -X POST "$rust/api/auth/register" "${headers[@]}" \
    -d "{\"username\":\"$1\",\"displayName\":\"$1\",\"email\":\"$1@example.test\",\"password\":\"$password\",\"inviteCode\":\"sharedinvite\"}"
}

# The accounts and what the story needs of them, made on this server in the
# Testing environment, which lifts the limits seven registrations would meet.
start_rust Testing ""
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/rust/log.txt")
code -X POST "$rust/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"ola\",\"displayName\":\"ola\",\"email\":\"ola@example.test\",\"password\":\"$password\",\"setupCode\":\"$setup\"}" >/dev/null
db=$work/rust/stellarr.db
sqlite3 "$db" "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000001',(select Id from Servers),(select Id from Users),NULL,'sharedinvite',0,NULL,NULL,'2026-11-24 01:00:00')"
for person in ben cat dan eve fay gus; do register $person >/dev/null; done
id() { sqlite3 "$db" "select Id from Users where Username='$1'"; }
now='2026-11-24 02:00:00'
ola=$(id ola); ben=$(id ben); dan=$(id dan)
{
  # dan takes direct messages from friends only; eve stands in for somebody;
  # fay is disabled.
  echo "insert into UserPreferences (UserId,DesktopNotifications,MentionPopups,CallRingtone,EmailNotifications,PushNotifications,MentionsOnly,MessageSound,JoinLeaveSound,AllowDirectMessages,AllowFriendRequests,ShowPresence,DeveloperMode,ReducedMotion,VadSensitivity) values('$dan',1,1,1,0,0,0,1,1,0,1,1,0,0,50);"
  echo "update Users set Origin='discord' where Username='eve';"
  echo "update Users set IsDisabled=1 where Username='fay';"
  # ben wants pushes, so that a message to him queues a delivery.
  echo "insert into UserPreferences (UserId,DesktopNotifications,MentionPopups,CallRingtone,EmailNotifications,PushNotifications,MentionsOnly,MessageSound,JoinLeaveSound,AllowDirectMessages,AllowFriendRequests,ShowPresence,DeveloperMode,ReducedMotion,VadSensitivity) values('$ben',1,1,1,0,1,0,1,1,1,1,1,0,0,50);"
  bensession=$(sqlite3 "$db" "select Id from UserSessions where UserId='$ben' limit 1")
  echo "insert into PushSubscriptions (Id,UserId,SessionId,Endpoint,P256dh,Auth,CreatedAt) values('E0000000-0000-0000-0000-000000000001','$ben','$bensession','https://push.example/ben','key','auth','$now');"
  # The uploads: ten and more plain ones and twenty-two sealed ones of ola's,
  # one of ben's, a voice message.
  upload() { # id owner encrypted voice poster
    echo "insert into ImageAttachments (Id,UploadedById,ObjectKey,FileName,ContentType,SizeBytes,Width,Height,DurationSeconds,IsVoiceMessage,PosterObjectKey,IsEncrypted,CreatedAt) values('$1','$2','key-$1','file-$1.png','image/png',12,3,4,NULL,$4,$5,$3,'$now');"
  }
  for n in $(seq 1 16); do upload "A1000000-0000-0000-0000-$(printf %012d "$n")" "$ola" 0 0 NULL; done
  echo "update ImageAttachments set PosterObjectKey='poster-16', PreviewObjectKey='preview-16' where Id='A1000000-0000-0000-0000-000000000016';"
  for n in $(seq 1 22); do upload "A2000000-0000-0000-0000-$(printf %012d "$n")" "$ola" 1 0 NULL; done
  upload A3000000-0000-0000-0000-000000000001 "$ola" 0 1 NULL
  upload B1000000-0000-0000-0000-000000000001 "$ben" 0 0 NULL
} | sqlite3 "$db"
eve=$(id eve); fay=$(id fay)
sqlite3 "$db" "pragma wal_checkpoint(truncate)" >/dev/null
stop_rust
cp "$db" "$work/shared0.db"
rm -rf "$work/rust"

# Encryption, through the first server's own routes.
give_oracle "$work/shared0.db"
node_run prepare --base "$oracle" --password "$password" --ids "$work/ids.json"
take_oracle "$work/shared1.db"

start_rust Development "$work/shared1.db"
for target in oracle rust; do
  address=${!target}
  node_run story --base "$address" --password "$password" --eve "$eve" --fay "$fay" --ids "$work/ids.json" --out "$work/$target.txt"
done
docker stop -t 10 "$container" >/dev/null
docker cp "$container:/tmp/data/stellarr.db" "$work/oracle-final.db"
stop_rust
cp "$work/rust/stellarr.db" "$work/rust-final.db"

# What the story left in the tables, with the ids told apart by name.
rows() { # a database file
  sqlite3 "$1" "
    create temp table names as select Id, Username as Name from Users;
    create temp table threads as select Id, (select Name from names where Id=UserAId) || '-' || (select Name from names where Id=UserBId) as Name from DirectConversations;
    select 'conversation', Name, c.EncryptionState, c.CurrentEpochId is not null, c.EncryptionRequestedById is not null from DirectConversations c join threads t on t.Id=c.Id order by Name;
    select 'message', t.Name, (select Name from names where Id=m.AuthorId), m.Content, m.EncryptedEpochId is not null, hex(m.EncryptedIv), hex(m.EncryptedCiphertext), m.SystemKind, m.CreatedAt is not null
      from DirectMessages m join threads t on t.Id=m.ConversationId order by m.CreatedAt, m.Id;
    select 'attachment', m.Content, l.AttachmentId from DirectMessageAttachments l join DirectMessages m on m.Id=l.DirectMessageId order by m.CreatedAt, l.AttachmentId;
    select 'read', t.Name, (select Name from names where Id=r.UserId), r.LastReadAt is not null from DirectConversationReads r join threads t on t.Id=r.ConversationId order by 2,3;
    select 'delivery', d.EventKind, d.Channel, (select Name from names where Id=d.RecipientId) from NotificationDeliveries d order by d.CreatedAt, d.Id;
    select 'counts', (select count(*) from UserNotifications), (select count(*) from MessageChanges), (select count(*) from MessageIndexEntries), (select count(*) from Friendships), (select count(*) from UserBlocks), (select count(*) from DirectMessageAttachments);
  "
}
rows "$work/oracle-final.db" > "$work/oracle-rows.txt"
rows "$work/rust-final.db" > "$work/rust-rows.txt"

# Cursors, on one set of rows.
give_oracle "$work/oracle-final.db"
start_rust Development "$work/oracle-final.db"
node_run pages --base "$oracle" --password "$password" --eve "$eve" --fay "$fay" --cursors "$work/oracle-cursors.json" --out "$work/oracle-pages.txt"
node_run pages --base "$rust" --password "$password" --eve "$eve" --fay "$fay" --cursors "$work/rust-cursors.json" --out "$work/rust-pages.txt"
node_run feed --base "$oracle" --password "$password" --eve "$eve" --fay "$fay" --cursors "$work/rust-cursors.json" --out "$work/oracle-feed.txt"
node_run feed --base "$rust" --password "$password" --eve "$eve" --fay "$fay" --cursors "$work/oracle-cursors.json" --out "$work/rust-feed.txt"

# A block ends a call in the conversation and sends the room's snapshot, which
# belongs to voice and is not ported here: those two lines are left out.
for target in oracle rust; do grep -v VoiceParticipantChanged "$work/$target.txt" > "$work/$target.cmp"; done
status=0
for part in "" -rows -pages -feed; do
  if [ "$part" = -rows ]; then a=$work/oracle-rows.txt; b=$work/rust-rows.txt; elif [ -z "$part" ]; then a=$work/oracle.cmp; b=$work/rust.cmp; else a=$work/oracle$part.txt; b=$work/rust$part.txt; fi
  if diff "$a" "$b"; then echo "the two servers agree${part:+ ($part)}"; else status=1; fi
done
exit $status
