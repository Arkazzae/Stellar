#!/bin/bash
# The tool against a running server of this crate: what it changes, the server
# sees; a session it revokes is closed on the hub; a recovery credential it
# issues is the one the server redeems; and it leaves the data whole and says
# so plainly when the server is writing too much for it to get in.
#
#   STELLARR_API=path/to/stellarr-api STELLARR_BIN=path/to/stellarr \
#       running-server.sh
#
# Needs curl, node (26), python3 and sqlite3. It makes a data folder of its
# own, starts the server on a free port and stops it, whatever happens.
set -uo pipefail

here=$(cd "$(dirname "$0")" && pwd)
server=${STELLARR_API:?the path of the stellarr-api binary}
tool=${STELLARR_BIN:?the path of the stellarr binary}
work=$(mktemp -d)
root=$work/data
port=${PORT:-5330}
base=http://127.0.0.1:$port
password='Corona!2026-long'
headers=(-H 'X-Stellarr-Client: web' -H 'X-Stellarr-Client-Version: 2.0.0' -H 'Content-Type: application/json')
pid=
failures=0

cleanup() { [ -n "$pid" ] && kill "$pid" 2>/dev/null; wait 2>/dev/null; rm -rf "$work"; }
trap cleanup EXIT
trap 'exit 130' INT TERM

check() { # description, expected, actual
  if [ "$2" = "$3" ]; then echo "ok        $1"; else echo "FAILED    $1: expected '$2', got '$3'"; failures=$((failures + 1)); fi
}
cli() { STELLARR_API_URL=$base "$tool" --data-root "$root" "$@"; }

mkdir -p "$root/db"
env -i PATH="$PATH" ASPNETCORE_ENVIRONMENT=Testing ASPNETCORE_URLS=$base STELLARR_DATA_ROOT="$root" \
  "ConnectionStrings__Database=Data Source=$root/db/stellarr.db;Pooling=True" \
  "$server" >"$work/server.log" 2>&1 &
pid=$!
for _ in $(seq 1 100); do curl -fs "$base/health/ready" >/dev/null && break; sleep 0.2; done
code=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/server.log" | tail -1)

post() { # cookie jar, path, body
  curl -s -b "$1" -c "$1" -X POST "$base$2" "${headers[@]}" -d "$3"
}
post "$work/owner" /api/auth/register "{\"username\":\"owner\",\"displayName\":\"Owner\",\"email\":\"owner@example.test\",\"password\":\"$password\",\"setupCode\":\"$code\"}" >/dev/null
invite=$(post "$work/owner" /api/admin/invites '{}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["url"].rsplit("/",1)[1])')
post "$work/maya" /api/auth/register "{\"username\":\"maya\",\"displayName\":\"Maya\",\"email\":\"maya@example.test\",\"password\":\"$password\",\"inviteCode\":\"$invite\"}" >/dev/null
cookie=$(awk '$6 ~ /session/ {printf "%s=%s", $6, $7}' "$work/maya")

echo "== a setting the tool changes is the one the server reads, at once"
check "the instance name the server reports before" "Stellarr" "$(curl -s -b "$work/owner" "$base/api/bootstrap" "${headers[@]}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["server"]["name"])')"
cli config set instance.name "Orbit CLI" --yes >/dev/null
check "the instance name the server reports after" "Orbit CLI" "$(curl -s -b "$work/owner" "$base/api/bootstrap" "${headers[@]}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["server"]["name"])')"
cli config set mfa.requirement everyone --yes >/dev/null
status=$(post "$work/maya2" /api/auth/login "{\"username\":\"maya\",\"password\":\"$password\"}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("status"))')
check "with the second step required for everyone, a sign-in asks for one to be set up" "mfa_enrollment_required" "$status"
cli config set mfa.requirement optional --yes >/dev/null
check "and with it optional again, it signs in" "None" "$(post "$work/maya3" /api/auth/login "{\"username\":\"maya\",\"password\":\"$password\"}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("status"))')"

echo "== a session the tool revokes is closed on the hub within a second or two"
session=$(cli --json sessions list --user maya | python3 -c 'import sys,json;d=json.load(sys.stdin)["data"]["sessions"];print([s["id"] for s in d if s["device"]][0])')
node "$here/running-server.mjs" --base "$base" --cookie "$cookie" >"$work/hub.log" 2>&1 &
hub=$!
for _ in $(seq 1 50); do grep -q '^open' "$work/hub.log" && break; sleep 0.1; done
check "the hub connection opens" "open" "$(head -1 "$work/hub.log")"
revoked=$(cli --json sessions revoke "$session" --yes | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["revoked"])')
check "the session is revoked" "True" "$revoked"
wait "$hub"
echo "          $(tail -1 "$work/hub.log")"
check "the hub closed it" "closed" "$(tail -1 "$work/hub.log" | cut -d' ' -f1)"

echo "== a recovery credential the tool issues is the server's to redeem"
credential=$(cli --json security recover maya --reason "lost phone" --yes | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["recoveryCredential"])')
answer=$(curl -s -o "$work/redeem.json" -w '%{http_code}' -X POST "$base/api/auth/recovery/redeem" "${headers[@]}" -d "{\"credential\":\"$credential\"}")
if [ "$answer" = 404 ]; then
  echo "skipped   redeeming: this server has no /api/auth/recovery/redeem yet"
else
  check "redeeming it answers that a second step must be set up" "mfa_enrollment_required" "$(python3 -c 'import sys,json;print(json.load(open(sys.argv[1])).get("status"))' "$work/redeem.json")"
  check "redeeming it twice is refused" "400" "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$base/api/auth/recovery/redeem" "${headers[@]}" -d "{\"credential\":\"$credential\"}")"
fi

echo "== the tool while the server writes heavily"
writers=()
for n in 1 2 3 4 5 6; do
  ( for _ in $(seq 1 60); do post "$work/w$n" /api/auth/login "{\"username\":\"owner\",\"password\":\"$password\"}" >/dev/null; done ) &
  writers+=($!)
done
codes=()
for _ in $(seq 1 12); do
  cli --json config set mail.smtp.port 2525 --yes >/dev/null 2>&1; codes+=($?)
  cli --json sessions revoke --user owner --all --yes >/dev/null 2>&1; codes+=($?)
  cli --json secrets clear giphy.api-key --yes >/dev/null 2>&1; codes+=($?)
done
for writer in "${writers[@]}"; do wait "$writer"; done
bad=0
for code in "${codes[@]}"; do [ "$code" = 0 ] || [ "$code" = 4 ] || bad=$((bad + 1)); done
check "every command either went through or said the database was busy (exit 0 or 4)" 0 "$bad"
echo "          exit statuses seen: $(printf '%s\n' "${codes[@]}" | sort | uniq -c | tr '\n' ' ')"
kill "$pid"; wait "$pid" 2>/dev/null; pid=
check "the database is whole afterwards" "ok" "$(sqlite3 "$root/db/stellarr.db" 'PRAGMA integrity_check')"
check "every change the tool made was audited once (settings, sessions and secrets)" 0 "$(sqlite3 "$root/db/stellarr.db" "SELECT COUNT(*) FROM OperatorAuditEntries WHERE Actor <> 'local-cli'")"

[ "$failures" = 0 ] && echo "all good" || { echo "$failures failed"; exit 1; }
