#!/bin/bash
# What a signed-in person does to the security of their own account, put to
# the first server and to this one on copies of one database file, with a hub
# connection open for each browser and a mail server that catches what is sent.
#
#   1. Both servers start fresh. `acctsec-oracle.mjs prepare` founds each
#      instance and signs a roster of accounts in, in more than one browser.
#   2. Both are stopped and their files edited the same way
#      (`acctsec-edit.py`): second steps, passwords and none, passkeys, linked
#      identities, remembered browsers, recovery codes, sessions that passed
#      the second step lately and sessions that did not or are too old, the
#      instance's requirement for administrators, and the mail settings.
#   3. Both are started again and `acctsec-oracle.mjs ask` puts every route to
#      every kind of account: status, the headers a browser cares about, the
#      body, which hub connections the server closed, and the mail it sent.
#   4. The mail server is moved to a port nobody listens on and the address
#      route is asked again (`mail-down`).
#   5. The two transcripts are compared, and then the rows each server left
#      (`acctsec-tables.py`).
#   6. A fresh pair of servers with the real limits each refuse the request
#      after the limit of their route.
#
# Needs docker, node 26, python3 and sqlite3. Starts a container for each
# server (`oracle start`) and one for a mail server (Mailpit, which is already
# an image on the machine); every exit stops and removes them and the work
# folder. KEEP=1 keeps the folder.
#
#   ORACLE=acctsec ORACLE_PORT=5223 MAIL_SMTP=5261 MAIL_HTTP=5262 \
#     STELLARR_API=path/to/stellarr-api ORACLE_SCRIPT=path/to/oracle \
#     acctsec-shared-database.sh
set -uo pipefail

name=${ORACLE:-acctsec}
port=${ORACLE_PORT:-5223}
smtp=${MAIL_SMTP:-5261}
mail_http=${MAIL_HTTP:-5262}
binary=${STELLARR_API:?the path of the stellarr-api binary}
oracle_script=${ORACLE_SCRIPT:?the path of the oracle script}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
mailer=stellarr-port-mail-$name
work=$(mktemp -d "${TMPDIR:-/tmp}/acctsec.XXXXXX")
oracle=http://127.0.0.1:$port
pid=
status=0

cleanup() {
  [ -n "$pid" ] && { kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; }
  "$oracle_script" stop "$name" >/dev/null 2>&1
  docker rm -f "$mailer" >/dev/null 2>&1
  if [ -n "${KEEP:-}" ]; then echo "kept: $work"; else rm -rf "$work"; fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

ready() { for _ in $(seq 1 90); do [ "$(curl -s -o /dev/null -w '%{http_code}' "$1/health/ready")" = 200 ] && return; sleep 1; done; echo "$1 is not ready" >&2; exit 1; }
start_rust() { # <folder> [extra environment as NAME=value ...]
  local folder=$1; shift
  mkdir -p "$folder"; : > "$folder/log.txt"
  env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$folder" \
    "ConnectionStrings__Database=Data Source=$folder/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    "Operations__PublicUrl=$oracle" "$@" "$binary" > "$folder/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 100); do grep -q 'Now listening on' "$folder/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$folder/log.txt")
  ready "$rust"
}
stop_rust() { [ -n "$pid" ] && { kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; pid=; }; }
setup_code() { sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$1" | tail -1; }
oracle_start() { # [env file]
  "$oracle_script" stop "$name" >/dev/null 2>&1
  ORACLE_ENV_FILE=${1:-} "$oracle_script" start "$name" "$port" Production > "$work/oracle-start.log" || { cat "$work/oracle-start.log" >&2; exit 1; }
}
restore_oracle() { docker stop -t 20 "$container" >/dev/null; docker cp "$1" "$container:/tmp/data/stellarr.db"; docker start "$container" >/dev/null; ready "$oracle"; }
take_oracle() { docker stop -t 20 "$container" >/dev/null; docker cp "$container:/tmp/data/stellarr.db" "$1"; }

high=$work/high.env
printf 'RateLimiting__AuthWrite__PermitLimit=500\nRateLimiting__Login__PermitLimit=500\nRateLimiting__SecurityWrite__PermitLimit=500\n' > "$high"
rust_high=(RateLimiting__AuthWrite__PermitLimit=500 RateLimiting__Login__PermitLimit=500 RateLimiting__SecurityWrite__PermitLimit=500)

docker rm -f "$mailer" >/dev/null 2>&1
docker run -d --name "$mailer" -p "127.0.0.1:$smtp:1025" -p "127.0.0.1:$mail_http:8025" axllent/mailpit:v1.34.0 >/dev/null || exit 1

echo "== 1. both servers, fresh, and the accounts"
oracle_start "$high"
start_rust "$work/rust" "${rust_high[@]}"
node "$here/acctsec-oracle.mjs" prepare --base "$oracle" --setup "$(setup_code "$work/oracle-start.log")" --out "$work/state-first.json" || exit 1
node "$here/acctsec-oracle.mjs" prepare --base "$rust" --setup "$(setup_code "$work/rust/log.txt")" --out "$work/state-this.json" || exit 1

echo "== 2. the files are edited"
stop_rust
take_oracle "$work/oracle.db"
python3 "$here/acctsec-edit.py" "$work/oracle.db" host.docker.internal "$smtp" || exit 1
python3 "$here/acctsec-edit.py" "$work/rust/stellarr.db" 127.0.0.1 "$smtp" || exit 1
cp "$work/oracle.db" "$work/before-oracle.db"; cp "$work/rust/stellarr.db" "$work/before-rust.db"
restore_oracle "$work/oracle.db"
start_rust "$work/rust" "${rust_high[@]}"

echo "== 3. every route to every kind of account"
node "$here/acctsec-oracle.mjs" ask --base "$oracle" --state "$work/state-first.json" --out "$work/first.txt" --public "$oracle" --mail "$mail_http" || status=1
node "$here/acctsec-oracle.mjs" ask --base "$rust" --state "$work/state-this.json" --out "$work/this.txt" --public "$oracle" --mail "$mail_http" || status=1

echo "== 4. the mail server is gone"
stop_rust
take_oracle "$work/oracle.db"
python3 "$here/acctsec-edit.py" "$work/oracle.db" --mail-port 59999
python3 "$here/acctsec-edit.py" "$work/rust/stellarr.db" --mail-port 59999
cp "$work/oracle.db" "$work/after-oracle.db"; cp "$work/rust/stellarr.db" "$work/after-rust.db"
restore_oracle "$work/oracle.db"
start_rust "$work/rust" "${rust_high[@]}"
node "$here/acctsec-oracle.mjs" mail-down --base "$oracle" --state "$work/state-first.json" --out "$work/down-first.txt" --public "$oracle" --mail "$mail_http" || status=1
node "$here/acctsec-oracle.mjs" mail-down --base "$rust" --state "$work/state-this.json" --out "$work/down-this.txt" --public "$oracle" --mail "$mail_http" || status=1

echo "== 5. the transcripts"
diff "$work/first.txt" "$work/this.txt" > "$work/transcript.diff" && echo "the transcripts are the same ($(grep -c '^## ' "$work/first.txt") requests)" || { status=1; cat "$work/transcript.diff"; }
diff "$work/down-first.txt" "$work/down-this.txt" && echo "the transcripts with the mail server gone are the same" || status=1
stop_rust
take_oracle "$work/final-oracle.db"
echo "== the rows"
python3 "$here/acctsec-tables.py" "$work/final-oracle.db" "$work/rust/stellarr.db" || status=1

echo "== 6. the limits"
for path in /api/me/email /api/me/password /api/me/security/mfa/enroll/start; do
  oracle_start
  start_rust "$work/limits-$(echo "$path" | tr / _)"
  node "$here/acctsec-oracle.mjs" limits --base "$oracle" --only "$path" --out "$work/limit-first.txt"
  node "$here/acctsec-oracle.mjs" limits --base "$rust" --only "$path" --out "$work/limit-this.txt"
  diff "$work/limit-first.txt" "$work/limit-this.txt" && echo "the limit of $path is the same: $(head -c 200 "$work/limit-first.txt" | head -1)" || status=1
  stop_rust
done
exit $status
