#!/bin/bash
# The desktop browser sign-in of this server against the first server, on one
# database file.
#
# The first server is started fresh with a single sign-on provider configured
# (the start of a request needs one), and seeded through its own routes. It is
# stopped and its database copied twice. The first server runs on one copy and
# this server on the other, and both are asked the same questions
# (desktop-login-oracle.mjs ask): the page for a signed-in person, for nobody,
# for another person, approval with every kind of form and origin, every way a
# request can be refused. The two transcripts are compared. Then the database
# of each is changed the same way (a request that ran out, a session that
# ended, an account disabled, a password changed, a second step passed ten
# minutes and a moment ago), both are asked again (`after`), and the
# transcripts and the rows each left (desktop-login-tables.py) are compared.
#
# The exchange, the last step, is not part of it: this server does not serve it.
#
# Needs docker, node 26 and python3. Not part of the ordinary run.
# Whatever the way out (the end, an error, Ctrl-C), the servers it started are
# stopped and its work folder is removed; KEEP=1 keeps the folder.
#
#   STELLARR_API=path/to/stellarr-api ORACLE_SCRIPT=path/to/oracle \
#     [ORACLE=desktop ORACLE_PORT=5229 LANGUAGE=pl] desktop-login-shared-database.sh
#
# LANGUAGE=pl runs both servers in Polish, to compare the pages' texts and the
# way their letters are written.
set -euo pipefail

name=${ORACLE:-desktop}
port=${ORACLE_PORT:-5229}
binary=${STELLARR_API:?the path of the stellarr-api binary}
oracle_script=${ORACLE_SCRIPT:?the path of the oracle script}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
work=$(mktemp -d "${TMPDIR:-/tmp}/desktop-login.XXXXXX")
oracle=http://127.0.0.1:$port
differ=0
language=${LANGUAGE:-en}
pid=

cleanup() {
  status=$?
  trap - EXIT INT TERM
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null || true
  "$oracle_script" stop "$name" >/dev/null 2>&1 || true
  if [ "${KEEP:-}" = 1 ]; then echo "kept: $work"; else rm -rf "$work"; fi
  exit $status
}
trap cleanup EXIT
trap 'exit 130' INT TERM

ready() { for _ in $(seq 1 90); do [ "$(curl -s -o /dev/null -w '%{http_code}' "$1/health/ready")" = 200 ] && return; sleep 1; done; echo "$1 is not ready" >&2; exit 1; }

provider='Oidc__Enabled=true
Oidc__Authority=https://identity.test
Oidc__ClientId=stellarr-tests
Oidc__ClientSecret=test-client-secret
Stellarr__Language='"$language"
settings=$work/oracle.env
printf 'RateLimiting__AuthWrite__PermitLimit=500\nRateLimiting__Login__PermitLimit=500\nRateLimiting__LoginPoll__PermitLimit=5000\n%s\n' "$provider" > "$settings"

start_rust() {
  [ -n "$pid" ] && { kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null || true; pid=; }
  : > "$work/rust/log.txt"
  env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    RateLimiting__AuthWrite__PermitLimit=500 RateLimiting__Login__PermitLimit=500 RateLimiting__LoginPoll__PermitLimit=5000 \
    "Stellarr__Language=$language" Oidc__Enabled=true Oidc__Authority=https://identity.test Oidc__ClientId=stellarr-tests Oidc__ClientSecret=test-client-secret \
    "Operations__PublicUrl=http://127.0.0.1:$port" "$binary" > "$work/rust/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 50); do grep -q 'Now listening on' "$work/rust/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt" | tail -1)
  ready "$rust"
}
stop_oracle() { docker stop -t 20 "$container" >/dev/null; }
start_oracle() { docker start "$container" >/dev/null; ready "$oracle"; }

"$oracle_script" stop "$name" >/dev/null 2>&1 || true
ORACLE_ENV_FILE=$settings "$oracle_script" start "$name" "$port" Production > "$work/start.log"
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/start.log" | tail -1)
node "$here/desktop-login-oracle.mjs" seed --base "$oracle" --setup "$setup" --state "$work/state.json"
stop_oracle
docker cp "$container:/tmp/data/stellarr.db" "$work/seeded.db"
cp "$work/seeded.db" "$work/oracle.db"
docker cp "$work/oracle.db" "$container:/tmp/data/stellarr.db"
start_oracle
mkdir -p "$work/rust"
cp "$work/seeded.db" "$work/rust/stellarr.db"
start_rust

echo "== the questions"
node "$here/desktop-login-oracle.mjs" ask --base "$oracle" --state "$work/state.json" --out "$work/first.txt" --ids "$work/first-ids.json" &
asked_first=$!
node "$here/desktop-login-oracle.mjs" ask --base "$rust" --state "$work/state.json" --out "$work/this.txt" --ids "$work/this-ids.json" &
asked_this=$!
wait $asked_first $asked_this
diff "$work/first.txt" "$work/this.txt" && echo "the transcripts are the same" || differ=1

# The same changes to both databases, for the requests each server made.
changes() { # database ids-file
  python3 - "$1" "$2" <<'PY'
import json, sqlite3, sys
db = sqlite3.connect(sys.argv[1])
requests = json.load(open(sys.argv[2]))["requests"]
now = "strftime('%Y-%m-%d %H:%M:%f', 'now'"
def row(name): return requests[name]["id"].upper()
by_request = '(SELECT {0} FROM "DesktopBrowserLogins" WHERE "Id" = ?)'
db.execute('UPDATE "DesktopBrowserLogins" SET "ExpiresAt" = \'2000-01-01 00:00:00\' WHERE "Id" = ?', (row("ranOut"),))
db.execute(f'UPDATE "DesktopBrowserLogins" SET "RedeemedAt" = {now}) WHERE "Id" = ?', (row("used"),))
db.execute(f'UPDATE "UserSessions" SET "RevokedAt" = {now}) WHERE "Id" = ' + by_request.format('"ApprovalSessionId"'), (row("endedSession"),))
db.execute('UPDATE "Users" SET "IsDisabled" = 1 WHERE "Id" = ' + by_request.format('"ApprovalUserId"'), (row("disabledAccount"),))
db.execute('UPDATE "Users" SET "SessionVersion" = "SessionVersion" + 1 WHERE "Id" = ' + by_request.format('"ApprovalUserId"'), (row("passwordChanged"),))
for name, seconds in (("staleSecondStep", 600), ("freshSecondStep", 30)):
    db.execute(
        f'UPDATE "UserSessions" SET "MfaCompletedAt" = strftime(\'%Y-%m-%d %H:%M:%f\', \'now\', \'-{seconds} seconds\'), "MfaEvidence" = \'totp\' WHERE "Id" = '
        + by_request.format('"ApprovalSessionId"'),
        (row(name),),
    )
db.commit()
PY
}

echo "== the database is changed"
kill "$pid"; wait "$pid" 2>/dev/null || true; pid=
stop_oracle
docker cp "$container:/tmp/data/stellarr.db" "$work/oracle.db"
changes "$work/oracle.db" "$work/first-ids.json"
changes "$work/rust/stellarr.db" "$work/this-ids.json"
docker cp "$work/oracle.db" "$container:/tmp/data/stellarr.db"
start_oracle
start_rust
node "$here/desktop-login-oracle.mjs" after --base "$oracle" --ids "$work/first-ids.json" --out "$work/after-first.txt"
node "$here/desktop-login-oracle.mjs" after --base "$rust" --ids "$work/this-ids.json" --out "$work/after-this.txt"
diff "$work/after-first.txt" "$work/after-this.txt" && echo "the transcripts after the changes are the same" || differ=1

echo "== the rows"
kill "$pid"; wait "$pid" 2>/dev/null || true; pid=
stop_oracle
docker cp "$container:/tmp/data/stellarr.db" "$work/after-first.db"
python3 "$here/desktop-login-tables.py" "$work/after-first.db" "$work/rust/stellarr.db" || differ=1
exit $differ
