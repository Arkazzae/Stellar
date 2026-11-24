#!/bin/bash
# Both servers on copies of one database file.
#
#   1. An account registered on the first server signs in on this one.
#   2. An account founded on this server, and one registered with an invite,
#      sign in on the first server, which also reads the instance (/api/bootstrap).
#   3. With the second step switched on for everyone (a row of settings in the
#      shared file), the answers to a sign-in of an account without a method and
#      of one with two, and to a registration, are the same on both.
#
# Needs docker, sqlite3 and curl, and the first server's container from
# `oracle start <name> <port> Production`. Not part of the ordinary run.
#
#   ORACLE=skeleton ORACLE_PORT=5191 STELLARR_API=path/to/stellarr-api oracle-shared-database.sh
set -euo pipefail

name=${ORACLE:-skeleton}
port=${ORACLE_PORT:-5191}
binary=${STELLARR_API:?the path of the stellarr-api binary}
container=stellarr-port-oracle-$name
oracle=http://127.0.0.1:$port
work=$(mktemp -d)
password='Corona!2026-long'
headers=(-H 'X-Stellarr-Client: web' -H 'X-Stellarr-Client-Version: 2.0.0' -H 'Content-Type: application/json')
pid=

cleanup() { [ -n "$pid" ] && kill "$pid" 2>/dev/null || true; rm -rf "$work"; }
trap cleanup EXIT

code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
login() { code -X POST "$1/api/auth/login" "${headers[@]}" -d "{\"username\":\"$2\",\"password\":\"$password\"}"; }

wait_ready() {
  for _ in $(seq 1 60); do [ "$(code "$1/health/ready")" = 200 ] && return; sleep 1; done
  echo "$1 did not become ready" >&2; exit 1
}

# This server on a copy of $1, in $work/rust; sets $rust to its address.
start_rust() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null || true
  rm -rf "$work/rust"; mkdir "$work/rust"
  [ -n "${1:-}" ] && cp "$1" "$work/rust/stellarr.db"
  env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    "$binary" > "$work/rust/log.txt" 2>&1 &
  pid=$!
  sleep 2
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
}

# The first server's database, copied out while it is stopped, and back in.
take_oracle_db() { docker stop -t 10 "$container" >/dev/null; docker cp "$container:/tmp/data/stellarr.db" "$1"; }
give_oracle_db() { docker cp "$1" "$container:/tmp/data/stellarr.db"; docker start "$container" >/dev/null; wait_ready "$oracle"; }

echo "== 1. registered on the first server, signing in on this one"
setup=$(docker logs "$container" 2>&1 | sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' | tail -1)
code -X POST "$oracle/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"fromcs\",\"displayName\":\"From CS\",\"email\":\"fromcs@example.test\",\"password\":\"$password\",\"setupCode\":\"$setup\"}" >/dev/null
take_oracle_db "$work/from-oracle.db"
start_rust "$work/from-oracle.db"
echo "fromcs on this server: $(login "$rust" fromcs) (200 expected)"

echo "== 2. founded and registered here, signing in on the first server"
start_rust ""
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/rust/log.txt")
code -X POST "$rust/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"fromrust\",\"displayName\":\"From Rust\",\"email\":\"fromrust@example.test\",\"password\":\"$password\",\"setupCode\":\"$setup\"}" >/dev/null
db=$work/rust/stellarr.db
sqlite3 "$db" "insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000001',(select Id from Servers),(select Id from Users),NULL,'sharedinvite',0,NULL,NULL,'2026-11-24 01:00:00')"
code -X POST "$rust/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"maya\",\"displayName\":\"Maya Lin\",\"email\":\"maya@example.test\",\"password\":\"$password\",\"inviteCode\":\"sharedinvite\"}" >/dev/null
kill "$pid"; wait "$pid" 2>/dev/null || true; pid=
cp "$db" "$work/from-rust.db"
give_oracle_db "$work/from-rust.db"
echo "fromrust on the first server: $(login "$oracle" fromrust) (200 expected)"
echo "maya on the first server: $(login "$oracle" maya) (200 expected)"

echo "== 3. second step for everyone, on both servers"
take_oracle_db "$work/shared.db"
server=$(sqlite3 "$work/shared.db" "select Id from Servers")
person=$(sqlite3 "$work/shared.db" "select Id from Users where Username='fromrust'")
owner=$(sqlite3 "$work/shared.db" "select Id from Users limit 1")
sqlite3 "$work/shared.db" "
insert into OperationsSettings (ServerId,PublicUrl,TimeZone,MailingEnabled,MailFromAddress,InviteMailEnabled,RecoveryMailEnabled,AlertMailEnabled,BackupRetentionDays,LogRetentionDays,OrphanObjectRetentionDays,AutomaticCleanupEnabled,UpdatedAt,MfaRequirement) values ('$server','','UTC',0,'',0,0,0,7,7,7,0,'2026-11-24 00:00:00','everyone');
insert into MfaMethods (Id,UserId,Type,Name,CreatedAt,ConfirmedAt,ProtectedSecret) values ('11111111-1111-1111-1111-111111111111','$person','totp','Phone','2026-11-20 10:00:00.5','2026-11-20 10:01:00','x');
insert into MfaMethods (Id,UserId,Type,Name,CreatedAt,ConfirmedAt,EmailAddress) values ('22222222-2222-2222-2222-222222222222','$person','email','Mail','2026-11-21 10:00:00','2026-11-21 10:01:00','FROMRUST@EXAMPLE.TEST');
insert into ServerInvites values('AAAAAAAA-0000-0000-0000-000000000002','$server','$owner',NULL,'regmfa',0,NULL,NULL,'2026-11-24 01:00:00');"
give_oracle_db "$work/shared.db"
start_rust "$work/shared.db"

# What differs by nature is replaced, header names are lower-cased (the two
# servers spell them differently) and the lines are sorted.
scrub() {
  tr -d '\r' | perl -pe 's/^([A-Za-z-]+):/\L$1:/; s/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/<id>/g; s/[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z?/<time>/g; s/oklch\(0\.58 0\.14 [0-9]+\)/oklch(<hue>)/g; s/00-[0-9a-f]{32}-[0-9a-f]{16}-00/<trace>/g; s/=[^;=]{20,};/=<value>;/; s/expires=[A-Za-z]+, [0-9]+ [A-Za-z]+ [0-9]+ [0-9:]+ GMT/expires=<in five minutes>/' | grep -Eiv '^(date|server|transfer-encoding|content-length):' | sort
}
compare() { # label, path, body
  local a b
  a=$(curl -si -X POST "$oracle$2" "${headers[@]}" -d "$3" | scrub)
  b=$(curl -si -X POST "$rust$2" "${headers[@]}" -d "$3" | scrub)
  if [ "$a" = "$b" ]; then echo "same      $1"; else echo "DIFFERENT $1"; diff <(echo "$a") <(echo "$b") || true; fi
}
compare "sign-in of an account with two methods" /api/auth/login "{\"username\":\"fromrust\",\"password\":\"$password\"}"
compare "sign-in of an account without a method" /api/auth/login "{\"username\":\"maya\",\"password\":\"$password\"}"
compare "registration that needs an enrolment" /api/auth/register \
  '{"username":"newbie","displayName":"New Bie","email":"newbie@example.test","password":"Corona!2026-long","inviteCode":"regmfa"}'
