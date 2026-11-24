#!/bin/bash
# What the encryption routes do on this server and on the first one, on copies
# of one database.
#
# It makes four accounts on this server (maja, ada, ilya and zed), the
# conversations between some of them, a block, and a hundred less two places of
# zed's devices, and copies the file to the first server's container. Then
# encryption-oracle.mjs tells the same story to each, with a hub connection open
# for three of the people, and the transcripts and the rows each server left
# are compared:
#
#   story   every answer, and every event each connection hears, in order
#   tables  what each route accepts at each size and in each encoding
#   races   requests that meet: the answers that came, and what was left
#
# Needs docker, sqlite3, curl, Node 26, and the first server's container from
# `oracle start <name> <port> Production` with the limits raised:
#
#   RateLimiting__AuthWrite__PermitLimit=500   RateLimiting__Login__PermitLimit=500
#   RateLimiting__SecurityWrite__PermitLimit=500   RateLimiting__PinUnlock__PermitLimit=500
#
# Prints the difference of each pair of files. The servers agree on everything
# but two lines of `tables`, which are known and meant: a body of 1 MiB and a
# little more (the first server read up to 128 MiB and answered the validation
# problem of what it held; this one refuses it with a 413), and a `null` among
# the envelopes of an epoch (a 500 of the first server's code, a 400 here).
# Who wins a race is not compared, only what was answered. The files are left in
# $WORK.
#
#   ORACLE=encryption ORACLE_PORT=5218 STELLARR_API=path/to/stellarr-api encryption-oracle.sh
set -euo pipefail

name=${ORACLE:-encryption}
port=${ORACLE_PORT:-5218}
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

# This server on a copy of $2 (or on nothing), in $work/rust, as environment $1.
start_rust() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null || true
  rm -rf "$work/rust"; mkdir -p "$work/rust"
  [ -n "${2:-}" ] && cp "$2" "$work/rust/stellarr.db"
  env ASPNETCORE_ENVIRONMENT="$1" Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    RateLimiting__AuthWrite__PermitLimit=500 RateLimiting__Login__PermitLimit=500 \
    RateLimiting__SecurityWrite__PermitLimit=500 RateLimiting__PinUnlock__PermitLimit=500 \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    "$binary" > "$work/rust/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 50); do grep -q 'Now listening on' "$work/rust/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
  wait_ready "$rust"
}
# This server on the folder it has now (the sealing key of its sessions is in it).
start_rust_in_place() {
  env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    RateLimiting__AuthWrite__PermitLimit=500 RateLimiting__Login__PermitLimit=500 \
    RateLimiting__SecurityWrite__PermitLimit=500 RateLimiting__PinUnlock__PermitLimit=500 \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    "$binary" > "$work/rust/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 50); do grep -q 'Now listening on' "$work/rust/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
  wait_ready "$rust"
}
stop_rust() { kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null || true; pid=; }

give_oracle() { # a database file
  docker stop -t 10 "$container" >/dev/null
  docker cp "$1" "$container:/tmp/data/stellarr.db"
  docker start "$container" >/dev/null
  wait_ready "$oracle"
}
take_oracle() { docker stop -t 10 "$container" >/dev/null; docker cp "$container:/tmp/data/stellarr.db" "$1"; docker start "$container" >/dev/null; wait_ready "$oracle"; }

register() { # name
  code -X POST "$rust/api/auth/register" "${headers[@]}" \
    -d "{\"username\":\"$1\",\"displayName\":\"$1\",\"email\":\"$1@example.test\",\"password\":\"$password\",\"inviteCode\":\"sharedinvite\"}" >/dev/null
}

# ---- the database both start from ----

start_rust Testing ""
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/rust/log.txt")
code -X POST "$rust/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"maja\",\"displayName\":\"maja\",\"email\":\"maja@example.test\",\"password\":\"$password\",\"setupCode\":\"$setup\"}" >/dev/null
db=$work/rust/stellarr.db
sqlite3 "$db" "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000001',(select Id from Servers),(select Id from Users),NULL,'sharedinvite',0,NULL,NULL,'2026-11-24 01:00:00')"
for person in ada ilya zed; do register $person; done
stop_rust

id() { sqlite3 "$db" "select Id from Users where Username='$1'"; }
A=AAAA0000-0000-4000-8000-00000000000A; B=BBBB0000-0000-4000-8000-00000000000B
C=CCCC0000-0000-4000-8000-00000000000C; E=EEEE0000-0000-4000-8000-00000000000E
now='2026-11-24 02:00:00'
conversation() { # id first second
  sqlite3 "$db" "insert into DirectConversations (Id,UserAId,UserBId,CreatedAt,EncryptionState) values('$1', (select min('$2','$3')), (select max('$2','$3')), '$now', 'Off')"
}
conversation $A "$(id maja)" "$(id ada)"
conversation $B "$(id maja)" "$(id ilya)"
conversation $C "$(id ada)" "$(id ilya)"
conversation $E "$(id ilya)" "$(id zed)"
sqlite3 "$db" "insert into UserBlocks values('$(id zed)','$(id ilya)','$now')"
zed=$(id zed)
# zed's devices: ninety-eight browsers that signed in a day ago and stay signed in.
sqlite3 "$db" "
with recursive n(i) as (select 1 union all select i + 1 from n where i < 98)
insert into UserSessions (Id,UserId,Device,IpAddress,CreatedAt,LastSeenAt,ExpiresAt,AbsoluteExpiresAt,PrimarySource,AuthenticatedAt,SecurityVersion,ShortLived)
select 'DDDD0000-0000-4000-8000-' || printf('%012d', i), '$zed', 'Seeded', '203.0.113.7', '2026-11-24 02:00:00', '2026-11-24 02:00:00',
       '2036-11-24 02:00:00', '2036-11-24 02:00:00', 'password', '2026-11-24 02:00:00', (select SessionVersion from Users where Id='$zed'), 0 from n;
with recursive n(i) as (select 1 union all select i + 1 from n where i < 98)
insert into EncryptionDevices (Id,UserId,SessionId,Algorithm,PublicKey,Label,CreatedAt,LastSeenAt)
select 'EEEE1111-0000-4000-8000-' || printf('%012d', i), '$zed', 'DDDD0000-0000-4000-8000-' || printf('%012d', i), 'x25519', randomblob(32), 'Seeded',
       datetime('2026-11-24 02:00:00', '+' || i || ' seconds'), '2026-11-24 02:00:00' from n;
pragma wal_checkpoint(truncate);" >/dev/null
cp "$db" "$work/shared.db"
ids="{\"A\":\"$A\",\"B\":\"$B\",\"C\":\"$C\",\"E\":\"$E\"}"

# ---- what each server leaves in the file ----

rows() { # a database, the file to write
  sqlite3 "$1" "
select 'key', UserId, Algorithm, hex(PublicKey), CreatedAt from AccountKeys order by UserId;
select 'backup', UserId, hex(Salt), Iterations, hex(Verifier), hex(Iv), hex(Ciphertext), FailedAttempts, UpdatedAt from AccountKeyBackups order by UserId;
select 'device', d.UserId, d.Algorithm, hex(d.PublicKey), d.Label, d.CreatedAt, d.LastSeenAt, coalesce(d.RevokedAt,'-'), s.Device from EncryptionDevices d join UserSessions s on s.Id = d.SessionId where d.Label <> 'Seeded' order by d.UserId, d.CreatedAt;
select 'seeded', count(*), sum(RevokedAt is not null) from EncryptionDevices where Label = 'Seeded';
select 'conversation', Id, EncryptionState, coalesce(EncryptionRequestedById,'-'), coalesce(EncryptionRequestedAt,'-'), coalesce(EncryptionEnabledAt,'-'), coalesce(CurrentEpochId,'-') from DirectConversations order by Id;
select 'epoch', ConversationId, Sequence, coalesce(PreviousEpochId,'-'), CreatedById, CreatedAt from EncryptionEpochs order by ConversationId, Sequence;
select 'envelope', e.ConversationId, e.Sequence, v.RecipientUserId, v.RecipientKind, hex(v.EphemeralPublicKey), hex(v.Iv), hex(v.Ciphertext) from EncryptionEnvelopes v join EncryptionEpochs e on e.Id = v.EpochId order by e.ConversationId, e.Sequence, v.RecipientUserId, v.RecipientKind, hex(v.EphemeralPublicKey), hex(v.Iv), hex(v.Ciphertext);
select 'line', ConversationId, SystemKind, AuthorId, CreatedAt, Content from DirectMessages order by ConversationId, CreatedAt, SystemKind;
select 'security', UserId, Kind, Detail, IpAddress from SecurityEvents where Kind = 'session-revoked' order by CreatedAt;
select 'notifications', count(*) from SecurityNotifications;
select 'sessions revoked', UserId, count(*) from UserSessions where RevokedAt is not null group by UserId order by UserId;
" | perl -pe 's/[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9:.]+/<time>/g; s/\|(172\.17\.0\.1|127\.0\.0\.1)$/|<address>/' > "$2"
}

# The same ids by name and by order of appearance, so that two files compare.
tidy() {
  perl -e '
    my %seen; my $n = 0;
    while (<>) { s/\b([0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12})\b/ $seen{$1} \/\/= "<id" . ++$n . ">" /ge; print }
  ' "$1" > "$1.tidy"
}

differ=0
compare() { # label, first server's file, this server's
  if diff "$2" "$3" > "$work/$1.diff"; then echo "same        $1"; else echo "DIFFERENT   $1 ($work/$1.diff)"; head -60 "$work/$1.diff"; differ=1; fi
}

phase() { # story or tables
  give_oracle "$work/shared.db"
  start_rust Production "$work/shared.db"
  node "$here/encryption-oracle.mjs" --base "$oracle" --phase "$1" --ids "$ids" --password "$password" --out "$work/$1.oracle.txt"
  node "$here/encryption-oracle.mjs" --base "$rust" --phase "$1" --ids "$ids" --password "$password" --out "$work/$1.rust.txt"
  take_oracle "$work/$1.oracle.db"
  stop_rust
  cp "$work/rust/stellarr.db" "$work/$1.rust.db"
  rows "$work/$1.oracle.db" "$work/$1.oracle.rows"
  rows "$work/$1.rust.db" "$work/$1.rust.rows"
  for side in oracle rust; do tidy "$work/$1.$side.rows"; done
  compare "$1 transcript" "$work/$1.oracle.txt" "$work/$1.rust.txt"
  # Which of two requests that met won is not the same every time.
  [ "$1" = races ] || compare "$1 rows" "$work/$1.oracle.rows.tidy" "$work/$1.rust.rows.tidy"
}

# What the guarded routes say to sessions that did and did not pass a second
# step lately, and to an account that has no password: sessions are made, the
# file is changed while the server is stopped, and the same sessions ask.
fresh() {
  give_oracle "$work/shared.db"
  start_rust Production "$work/shared.db"
  node "$here/encryption-oracle.mjs" --base "$oracle" --phase fresh-login --cookies "$work/fresh.oracle.json" --ids "$ids" --password "$password" --out /dev/null
  node "$here/encryption-oracle.mjs" --base "$rust" --phase fresh-login --cookies "$work/fresh.rust.json" --ids "$ids" --password "$password" --out /dev/null
  take_oracle "$work/fresh.oracle.db"
  stop_rust
  cp "$work/rust/stellarr.db" "$work/fresh.rust.db"
  for side in oracle rust; do
    file=$work/fresh.$side.db
    sqlite3 "$file" "
      insert into MfaMethods (Id,UserId,Type,Name,CreatedAt,ConfirmedAt) select 'CCCC' || substr(Id,5), Id, 'totp', 'Seeded', '2026-11-20 10:00:00', '2026-11-20 10:00:00' from Users where Username in ('maja','ilya');
      update UserSessions set MfaCompletedAt = strftime('%Y-%m-%d %H:%M:%f', 'now', '-15 minutes') where UserId in (select Id from Users where Username in ('maja','ilya'));
      update UserSessions set MfaCompletedAt = strftime('%Y-%m-%d %H:%M:%f', 'now') where Id in (select Id from UserSessions s where UserId in (select Id from Users where Username in ('maja','ilya')) and CreatedAt = (select max(CreatedAt) from UserSessions where UserId = s.UserId));
      update Users set PasswordHash = NULL where Username = 'ada';
      update UserSessions set AuthenticatedAt = strftime('%Y-%m-%d %H:%M:%f', 'now', '-1 hour'), CreatedAt = strftime('%Y-%m-%d %H:%M:%f', 'now', '-1 hour') where UserId = (select Id from Users where Username = 'ada') and CreatedAt = (select CreatedAt from UserSessions where UserId = (select Id from Users where Username = 'ada') order by CreatedAt limit 1 offset 1);
      pragma wal_checkpoint(truncate);" > /dev/null
  done
  give_oracle "$work/fresh.oracle.db"
  # This server goes on from the folder it has, with the file changed.
  rm -f "$work/rust"/stellarr.db-*; cp "$work/fresh.rust.db" "$work/rust/stellarr.db"
  start_rust_in_place
  node "$here/encryption-oracle.mjs" --base "$oracle" --phase fresh --cookies "$work/fresh.oracle.json" --ids "$ids" --password "$password" --out "$work/fresh.oracle.txt"
  node "$here/encryption-oracle.mjs" --base "$rust" --phase fresh --cookies "$work/fresh.rust.json" --ids "$ids" --password "$password" --out "$work/fresh.rust.txt"
  stop_rust
  compare "fresh second step" "$work/fresh.oracle.txt" "$work/fresh.rust.txt"
}

# What ends a device on the first server only: signing out, a session revoked
# from another, a changed password. The rows of the devices are read from the
# first server's file after each, and nothing is written on them. (This server
# has no route for the last two yet; its own tests of them are in lifecycle.rs.)
lifecycle() {
  give_oracle "$work/shared.db"
  cat > "$work/dump.sh" <<DUMP
rm -f $work/dump.db*
for suffix in "" -wal -shm; do docker cp $container:/tmp/data/stellarr.db\$suffix $work/dump.db\$suffix > /dev/null 2>&1 || true; done
sqlite3 $work/dump.db "select d.Label || '|' || d.SessionId || '|' || d.CreatedAt || '|' || d.LastSeenAt || '|' || coalesce(d.RevokedAt, '-') from EncryptionDevices d join Users u on u.Id = d.UserId where u.Username = 'maja' order by d.CreatedAt"
DUMP
  node "$here/encryption-oracle.mjs" --base "$oracle" --phase lifecycle --dump "$work/dump.sh" --ids "$ids" --password "$password" --out "$work/lifecycle.oracle.txt"
  echo "first server, devices before and after (kept in $work/lifecycle.oracle.txt):"; cat "$work/lifecycle.oracle.txt"
}

phase story
phase tables
phase races
fresh
lifecycle
echo "files are in $work"
exit $differ
