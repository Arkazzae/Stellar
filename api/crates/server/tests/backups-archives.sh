#!/bin/bash
# Archives between this server and the first one, both ways.
#
# The first server is started fresh and seeded through its own routes
# (servers-oracle.mjs seed). Its data folder is taken out, every table is given
# a row and every folder of the stores some files (backups-archives.py fill),
# and both servers get a copy. Then:
#   - both make a full archive of the same data, and the entries, their order,
#     sizes, checksums, the manifest and the database are compared;
#   - each archive is checked by the other's verifier (the first server's own
#     code runs in tests/backups-witness) and restored by the other, and the
#     restored trees are compared with the data;
#   - a corpus of hostile archives is given to both checks.
#
# Needs docker, node 26, python3, the .NET SDK (~/.dotnet/dotnet) and a built
# `examples/backups`. Not part of the ordinary run.
#
#   STELLARR_API=path/to/stellarr-api   (optional: start it on each restored folder)
#   BACKUPS_EXAMPLE=path/to/examples/backups ORACLE_SCRIPT=path/to/oracle \
#   FIRST_SOURCE=path/to/server/Stellarr.Api backups-archives.sh
#
# Everything made is removed on every way out unless KEEP_WORK=1 (then the work
# folder is named on stderr); the container is stopped and removed.
set -euo pipefail
# For the last step, a server started on a restored folder, the binary is also
# needed: STELLARR_API=path/to/stellarr-api (the step is skipped without it).
server_pid=

example=${BACKUPS_EXAMPLE:?the path of the built examples/backups}
oracle_script=${ORACLE_SCRIPT:?the path of the oracle script}
source_folder=${FIRST_SOURCE:?the folder server/Stellarr.Api of the first server}
name=${ORACLE:-backups}
port=${ORACLE_PORT:-5226}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
work=${WORK:-$(mktemp -d)}
base=http://127.0.0.1:$port
cleanup() {
  [ -z "$server_pid" ] || kill "$server_pid" 2>/dev/null || true
  "$oracle_script" stop "$name" >/dev/null 2>&1 || true
  docker rm -f "$container" >/dev/null 2>&1 || true
  if [ "${KEEP_WORK:-}" = 1 ]; then echo "work kept: $work" >&2; else rm -rf "$work"; fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM
status=0
py() { python3 -I "$here/backups-archives.py" "$@"; }
ready() { for _ in $(seq 1 90); do [ "$(curl -s -o /dev/null -w '%{http_code}' "$base/health/ready")" = 200 ] && return; sleep 1; done; echo "not ready" >&2; exit 1; }

"$here/backups-witness/build.sh" "$source_folder" "$work/witness" >/dev/null
witness() { "$HOME/.dotnet/dotnet" "$work/witness/bin/Witness.dll" "$@"; }

printf 'RateLimiting__AuthWrite__PermitLimit=500\nRateLimiting__Login__PermitLimit=500\nRateLimiting__InvitePreview__PermitLimit=500\nBackups__InitialDelayMinutes=1440\nMaintenance__Enabled=false\n' > "$work/oracle.env"
"$oracle_script" stop "$name" >/dev/null 2>&1 || true
docker rm -f "$container" >/dev/null 2>&1 || true
ORACLE_ENV_FILE=$work/oracle.env "$oracle_script" start "$name" "$port" Production > "$work/start.log"
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/start.log" | tail -1)
node "$here/servers-oracle.mjs" seed --base "$base" --setup "$setup" --state "$work/state.json" >/dev/null
# What the groups merged since the seed add, through the first server's routes.
node "$here/backups-archives-fill.mjs" --base "$base" --state "$work/state.json" --out "$work/facts.json"
docker stop -t 20 "$container" >/dev/null
docker cp "$container:/tmp/data" "$work/seed"
py fill "$work/seed"
docker cp "$work/seed/." "$container:/tmp/data/"
docker start "$container" >/dev/null
ready
py layout "$work/seed" "$work/data"

echo "== both servers make a full archive of the same data"
node "$here/backups-archives-first.mjs" --base "$base" --out "$work/first.zip"
"$example" make "$work/data" all "$work/ours.zip"
ls -l "$work/first.zip" "$work/ours.zip" | awk '{print $5, $9}'
py compare "$work/first.zip" "$work/ours.zip" login || status=1

echo "== each archive checked by the other's verifier"
"$example" verify "$work/first.zip" || status=1
"$example" check "$work/first.zip" >/dev/null && echo "ours checks the first server's archive: ok" || status=1
witness verify "$work/ours.zip" "$work/w1" || status=1
witness check "$work/ours.zip" "$work/w2" || status=1
# And the same archives by the witness that made its own.
witness make "$work/data" all "$work/witness.zip" && witness verify "$work/witness.zip" "$work/w1" || status=1
"$example" check "$work/witness.zip" > /dev/null && echo "ours checks the witness's archive: ok" || status=1
py compare "$work/witness.zip" "$work/ours.zip" || status=1

echo "== restore: the first server's archive by ours, ours by the first server's code"
for who in ours-restores-first witness-restores-ours; do
  py layout "$work/seed" "$work/$who"
  # Something to replace and keep aside.
  echo old > "$work/$who/keys/old-key"
  rm -rf "$work/$who/restore"
done
"$example" apply "$work/first.zip" "$work/ours-restores-first" || status=1
witness apply "$work/ours.zip" "$work/witness-restores-ours" || status=1
py tree "$work/data" "$work/ours-restores-first" "$work/first.zip" || status=1
echo "-- the first server's own restore, for what it does with modes:"
py tree "$work/data" "$work/witness-restores-ours" || status=1

# This server, started on a folder each archive was restored into: the people
# sign in, the history reads, an attachment downloads, the hub negotiates.
if [ -n "${STELLARR_API:-}" ]; then
  echo "== this server on a folder restored from each archive"
  for from in first ours; do
    root=$work/started-on-$from
    mkdir -p "$root"
    "$example" apply "$work/$from.zip" "$root" >/dev/null || status=1
    env ASPNETCORE_ENVIRONMENT=Testing Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$root" \
      "ConnectionStrings__Database=Data Source=$root/db/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
      "$STELLARR_API" > "$root.log" 2>&1 &
    server_pid=$!
    for _ in $(seq 1 60); do grep -q 'Now listening' "$root.log" && break; sleep 0.5; done
    address=$(sed -n 's/.*listening on: \(http:[^ ]*\).*/\1/p' "$root.log" | tail -1)
    echo "-- restored from the $from archive: $address"
    if [ -z "$address" ]; then tail -5 "$root.log"; status=1; else
      node "$here/backups-archives-restored.mjs" --base "$address" --facts "$work/facts.json" --sealed 1 || status=1
    fi
    kill "$server_pid" 2>/dev/null || true; wait "$server_pid" 2>/dev/null || true; server_pid=
  done
fi

echo "== hostile archives given to both checks"
py hostile "$work/hostile" > /dev/null
printf '%-28s %-12s %-12s\n' case ours first
for f in "$work"/hostile/*.zip; do
  case_name=$(basename "$f" .zip)
  if "$example" check "$f" >/dev/null 2>"$work/ours.err"; then ours=ACCEPTS; else ours=refuses; fi
  if witness check "$f" "$work/hw" >/dev/null 2>"$work/first.err"; then first=ACCEPTS; else first=refuses; fi
  mark=; [ "$ours" = "$first" ] || mark="  <-- differs"
  printf '%-28s %-12s %-12s%s\n' "$case_name" "$ours" "$first" "$mark"
  if [ -n "$mark" ] || [ -n "${VERBOSE:-}" ]; then echo "    ours: $(head -c 220 "$work/ours.err")"; echo "    first: $(head -c 220 "$work/first.err")"; fi
done
exit $status
