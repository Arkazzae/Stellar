#!/bin/bash
# What the second step's rows are, as the first server wrote them and as this
# server reads them, and the other way round, on copies of one database file.
#
#   1. An account enrols an authenticator app on the first server, through its
#      own routes, with recovery codes and a remembered device, then a code
#      sent by mail (the first server sends through a small mail server this
#      script starts), and a sign-in with one recovery code.
#   2. This server's modules read the copy of the file: the recovery codes and
#      the remembered device, which are digests, work; the authenticator app's
#      secret and the mailed code, which the first server sealed with its own
#      keys, do not, and the mailed code is charged an attempt.
#   3. This server's modules write a set of recovery codes and a remembered
#      device into the file; the first server accepts both.
#   4. This server answers the sign-in of the account on the first server's
#      file, with the first server's remembered device, as the first server does.
#
# Needs docker, sqlite3, curl and python3, the first server's container from
# `oracle start <name> <port>` (Development), and a built `stellarr-api`.
#
#   ORACLE=security ORACLE_PORT=5203 STELLARR_API=path/to/stellarr-api \
#     STELLARR_TEST_CARGO=cargo oracle-security-shared-database.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
name=${ORACLE:-security}
port=${ORACLE_PORT:-5203}
binary=${STELLARR_API:?the path of the stellarr-api binary}
cargo=${STELLARR_TEST_CARGO:-cargo}
container=stellarr-port-oracle-$name
oracle=http://127.0.0.1:$port
work=$(mktemp -d)
password='Corona!2026-long'
headers=(-H 'X-Stellarr-Client: web' -H 'X-Stellarr-Client-Version: 2.0.0' -H 'Content-Type: application/json')
pid=; sink=

cleanup() {
  for process in $pid $sink; do kill "$process" 2>/dev/null && wait "$process" 2>/dev/null || true; done
  rm -rf "$work"
}
trap cleanup EXIT

wait_ready() {
  for _ in $(seq 1 60); do [ "$(curl -s -o /dev/null -w '%{http_code}' "$1/health/ready")" = 200 ] && return; sleep 1; done
  echo "$1 did not become ready" >&2; exit 1
}
take_oracle_db() { docker stop -t 10 "$container" >/dev/null; docker cp "$container:/tmp/data/stellarr.db" "$1"; docker start "$container" >/dev/null; wait_ready "$oracle"; }
give_oracle_db() { docker stop -t 10 "$container" >/dev/null; docker cp "$1" "$container:/tmp/data/stellarr.db"; docker start "$container" >/dev/null; wait_ready "$oracle"; }
json() { sed -n "s/.*\"$1\":\"\\([^\"]*\\)\".*/\\1/p"; }
post() { # jar, path, body
  curl -s -b "$1" -c "$1" -X POST "$oracle$2" "${headers[@]}" -d "$3"
}
status() { # jar, path, body, extra curl args
  local jar=$1 path=$2 body=$3; shift 3
  curl -s -o /dev/null -w '%{http_code}' -b "$jar" -c "$jar" "$@" -X POST "$oracle$path" "${headers[@]}" -d "$body"
}
# Prints what a request answered next to what it should have.
check() { # label, expected, then the command
  local label=$1 expected=$2 got; shift 2
  got=$("$@")
  echo "$label: $got ($expected expected)"
}
challenge_body() { printf '{"challengeId":"%s","code":"%s"}' "$1" "$2"; }
device_body() { printf '{"challengeId":"%s","useTrustedDevice":true}' "$1"; }
rust_test() { # the name of an ignored test of the library
  (cd "$here/../../.." && "$cargo" test -p stellarr-api --lib "$1" -- --ignored --nocapture 2>&1 | grep -E "^RESULT|panicked|test result: FAILED" || true)
}

echo "== mail goes to a server of ours"
python3 "$here/smtp-sink.py" "$work/mail" 2525 &
sink=$!
sleep 1

echo "== 1. the first server: an owner, an authenticator app, recovery codes, a remembered device, a mailed code"
setup=$(docker logs "$container" 2>&1 | sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' | tail -1)
owner=$work/owner.jar
curl -s -c "$owner" -X POST "$oracle/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"maya\",\"displayName\":\"Maya\",\"email\":\"maya@example.test\",\"password\":\"$password\",\"setupCode\":\"$setup\"}" > "$work/user.json"
user=$(json id < "$work/user.json")
# The first server sends mail through the sink: a row of settings in its file.
take_oracle_db "$work/base.db"
server=$(sqlite3 "$work/base.db" "select Id from Servers")
sqlite3 "$work/base.db" "insert into OperationsSettings (ServerId,PublicUrl,TimeZone,MailingEnabled,MailFromAddress,InviteMailEnabled,RecoveryMailEnabled,AlertMailEnabled,BackupRetentionDays,LogRetentionDays,OrphanObjectRetentionDays,AutomaticCleanupEnabled,UpdatedAt,MailSmtpHost,MailSmtpPort,MailSmtpSecurity,MailResendEnabled) values ('$server','','UTC',1,'noreply@example.test',0,0,0,7,7,7,0,'2026-11-24 00:00:00','host.docker.internal',2525,'none',0);"
give_oracle_db "$work/base.db"

challenge=$(post "$owner" /api/me/security/mfa/enroll/start '{}' | json challengeId)
post "$owner" /api/auth/mfa/enroll/totp/start "{\"challengeId\":\"$challenge\",\"name\":\"Phone\"}" > "$work/totp.json"
secret=$(json secret < "$work/totp.json"); method=$(json methodId < "$work/totp.json")
curl -s -i -b "$owner" -c "$owner" -X POST "$oracle/api/auth/mfa/enroll/totp/confirm" "${headers[@]}" \
  -d "{\"challengeId\":\"$challenge\",\"methodId\":\"$method\",\"code\":\"$(python3 "$here/totp-independent.py" now "$secret")\",\"name\":\"Phone\",\"rememberDevice\":true}" > "$work/confirm.txt"
trust=$(sed -n 's/^Set-Cookie: stellarr-dev-trust=\([^;]*\);.*/\1/p' "$work/confirm.txt" | tr -d '\r')
codes=($(grep -o '"recoveryCodes":\[[^]]*\]' "$work/confirm.txt" | grep -o '[A-Z0-9]\{7\}-[A-Z0-9]\{7\}-[A-Z0-9]\{7\}-[A-Z0-9]\{7\}'))
echo "authenticator app $method, ${#codes[@]} recovery codes, remembered device cookie of ${#trust} characters"

challenge=$(post "$owner" /api/me/security/mfa/enroll/start '{}' | json challengeId)
post "$owner" /api/auth/mfa/enroll/email/start "{\"challengeId\":\"$challenge\"}" > /dev/null
sleep 1
mailed=$(tr -d '\r' < "$work/mail/0001.eml" | grep -A1 'valid for' | grep -E '^[0-9]{6}$' | head -1)
post "$owner" /api/auth/mfa/enroll/email/confirm "{\"challengeId\":\"$challenge\",\"code\":\"$mailed\",\"name\":\"Mail\"}" > /dev/null
take_oracle_db "$work/a.db"
email_method=$(sqlite3 "$work/a.db" "select Id from MfaMethods where Type='email'" | tr A-Z a-z)

# A sign-in that is answered by mail and finished with a recovery code.
login() { curl -s -c "$1" -b "${2:-/dev/null}" -X POST "$oracle/api/auth/login" "${headers[@]}" -d "{\"username\":\"maya\",\"password\":\"$password\"}"; }
signin=$work/signin.jar
challenge=$(login "$signin" "stellarr-dev-trust=$trust" | json challengeId)
post "$signin" /api/auth/mfa/email/send "$(printf '{"challengeId":"%s","methodId":"%s"}' "$challenge" "$email_method")" > /dev/null
sleep 1
login_mail=$(tr -d '\r' < "$work/mail/0002.eml" | grep -A1 'valid for' | grep -E '^[0-9]{6}$' | head -1)
typed=$(echo "${codes[0]}" | tr A-Z a-z | tr - ' ')
check "first server, the recovery code typed in lower case with spaces" 200 status "$signin" /api/auth/mfa/verify "$(challenge_body "$challenge" "$typed")"
take_oracle_db "$work/first.db"

echo "== 2. this server reads what the first server wrote"
cp "$work/first.db" "$work/read.db"
export STELLARR_SHARED_DB=$work/read.db STELLARR_SHARED_USER=$user STELLARR_SHARED_SPENT_CODE=${codes[0]} STELLARR_SHARED_UNUSED_CODE=${codes[1]} \
  STELLARR_SHARED_TRUST_TOKEN=$trust STELLARR_SHARED_TOTP_METHOD=$method STELLARR_SHARED_TOTP_CODE=$(python3 "$here/totp-independent.py" now "$secret") \
  STELLARR_SHARED_EMAIL_CHALLENGE=$challenge STELLARR_SHARED_EMAIL_METHOD=$email_method STELLARR_SHARED_EMAIL_PURPOSE=login STELLARR_SHARED_EMAIL_CODE=$login_mail
rust_test read_what_the_first_server_wrote

echo "== 3. the first server reads what this server wrote"
cp "$work/first.db" "$work/write.db"
export STELLARR_SHARED_DB=$work/write.db STELLARR_SHARED_OUT=$work/written.txt
rust_test write_what_the_first_server_reads
give_oracle_db "$work/write.db"
mine=$(sed -n 's/^TRUST //p' "$work/written.txt"); new=($(sed -n 's/^CODE //p' "$work/written.txt"))
r=$work/j1.jar; challenge=$(login "$r" "stellarr-dev-trust=$mine" | json challengeId)
listed=$(login "$work/j0.jar" "stellarr-dev-trust=$mine" | sed -n 's/.*"trustedDeviceAvailable":\([a-z]*\).*/\1/p')
echo "a device this server wrote, in the first server's list: $listed (true expected)"
check "verify with that device" 200 status "$r" /api/auth/mfa/verify "$(device_body "$challenge")" -b "stellarr-dev-trust=$mine"
r=$work/j2.jar; challenge=$(login "$r" | json challengeId)
check "a code of the first server's, retired by the new set" 401 status "$r" /api/auth/mfa/verify "$(challenge_body "$challenge" "${codes[1]}")"
check "a new code, lower case" 200 status "$r" /api/auth/mfa/verify "$(challenge_body "$challenge" "$(echo "${new[0]}" | tr A-Z a-z)")"
r=$work/j3.jar; challenge=$(login "$r" | json challengeId)
check "the same code again" 401 status "$r" /api/auth/mfa/verify "$(challenge_body "$challenge" "${new[0]}")"

echo "== 4. this server answers a sign-in on the first server's file as the first server does"
take_oracle_db "$work/last.db"
rm -rf "$work/rust"; mkdir "$work/rust"; cp "$work/last.db" "$work/rust/stellarr.db"
env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" \
  ASPNETCORE_URLS=http://127.0.0.1:0 "$binary" > "$work/rust/log.txt" 2>&1 &
pid=$!
sleep 2
rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
scrub() {
  tr -d '\r' | perl -pe 's/^([A-Za-z-]+):/\L$1:/; s/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/<id>/g; s/[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z?/<time>/g; s/oklch\(0\.58 0\.14 [0-9]+\)/oklch(<hue>)/g; s/=[^;=]{20,};/=<value>;/; s/expires=[A-Za-z]+, [0-9]+ [A-Za-z]+ [0-9]+ [0-9:]+ GMT/expires=<in five minutes>/; s/stellarr-dev-preauth/preauth/; s/__Host-stellarr-preauth/preauth/' | grep -Eiv '^(date|server|transfer-encoding|content-length):' | sort
}
for cookie in "stellarr-dev-trust=$trust" "stellarr-dev-trust=$mine" "stellarr-dev-trust=nothing"; do
  a=$(curl -si -b "$cookie" -X POST "$oracle/api/auth/login" "${headers[@]}" -d "{\"username\":\"maya\",\"password\":\"$password\"}" | scrub)
  b=$(curl -si -b "$cookie" -X POST "$rust/api/auth/login" "${headers[@]}" -d "{\"username\":\"maya\",\"password\":\"$password\"}" | scrub)
  label=${cookie%%=*}=${cookie#*=}; label=${label:0:36}
  if [ "$a" = "$b" ]; then echo "same      sign-in with $label"; else echo "DIFFERENT sign-in with $label"; diff <(echo "$a") <(echo "$b") || true; fi
done
