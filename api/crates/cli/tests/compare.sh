#!/bin/bash
# The first server's tool and this one, side by side, on copies of one data
# folder (see compare.py for what is compared and what is replaced first).
#
# Makes the data folder with the first server (a container of the first
# server's image: `oracle start cli 5232`), puts the rows of fixture.sql in it,
# builds the first server's tool with the .NET SDK, and runs compare.py. All of
# it is removed on every way out, the container included.
#
#   ORACLE=/path/to/oracle STELLARR_BIN=path/to/stellarr \
#       compare.sh [filter]
#
# Needs docker, sqlite3, curl and python3 and, for the first tool, the SDK
# (DOTNET=~/.dotnet/dotnet) and the sources of the first server (REPO, the
# root of the repository).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=${REPO:?the root of the repository, where server/ is}
oracle=${ORACLE:?the oracle script of the integrator}
export STELLARR_BIN=${STELLARR_BIN:?the path of the stellarr binary}
export DOTNET=${DOTNET:-dotnet}
name=${ORACLE_NAME:-cli}
port=${ORACLE_PORT:-5232}
container=stellarr-port-oracle-$name
work=$(mktemp -d)
headers=(-H 'X-Stellarr-Client: web' -H 'X-Stellarr-Client-Version: 2.0.0' -H 'Content-Type: application/json')
password='Corona!2026-long'

cleanup() { "$oracle" stop "$name" >/dev/null 2>&1 || true; rm -rf "$work"; }
trap cleanup EXIT
trap 'exit 130' INT TERM

# The first tool, built from a copy so that the build leaves nothing behind.
mkdir -p "$work/dn/server"
cp "$repo/Directory.Build.props" "$repo/global.json" "$work/dn/"
cp -R "$repo/server/Stellarr.Api" "$repo/server/Stellarr.Cli" "$work/dn/server/"
(cd "$work/dn/server/Stellarr.Cli" && "$DOTNET" build -c Release -o "$work/dn-out" -p:RestoreLockedMode=false >/dev/null)
export DN_DLL=$work/dn-out/Stellarr.Cli.dll

# The data folder, made by the first server: an owner, a second account, three sessions.
"$oracle" stop "$name" >/dev/null 2>&1 || true
code=$("$oracle" start "$name" "$port" Production | sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' | tail -1)
base=http://127.0.0.1:$port
curl -s -c "$work/owner" -X POST "$base/api/auth/register" "${headers[@]}" \
  -d "{\"username\":\"owner\",\"displayName\":\"The Owner\",\"email\":\"owner@example.test\",\"password\":\"$password\",\"setupCode\":\"$code\"}" >/dev/null
invite=$(curl -s -b "$work/owner" -X POST "$base/api/admin/invites" "${headers[@]}" -d '{}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["url"].rsplit("/",1)[1])')
curl -s -X POST "$base/api/auth/register" "${headers[@]}" -H 'X-Forwarded-For: 10.1.1.1' -H 'User-Agent: curl/8.7.1' \
  -d "{\"username\":\"maya\",\"displayName\":\"Maya\",\"email\":\"maya@example.test\",\"password\":\"$password\",\"inviteCode\":\"$invite\"}" >/dev/null
curl -s -X POST "$base/api/auth/login" "${headers[@]}" -H 'X-Forwarded-For: 10.1.1.2' -H 'User-Agent: Mozilla/5.0 (X11; Linux x86_64) Firefox/130' \
  -d "{\"username\":\"owner\",\"password\":\"$password\"}" >/dev/null
docker stop -t 10 "$container" >/dev/null
mkdir -p "$work/base/db"
docker cp "$container:/tmp/data/stellarr.db" "$work/base/db/stellarr.db"
docker cp "$container:/tmp/data/keys" "$work/base/keys"
"$oracle" stop "$name" >/dev/null

maya=$(sqlite3 "$work/base/db/stellarr.db" "SELECT Id FROM Users WHERE Username = 'maya'")
owner=$(sqlite3 "$work/base/db/stellarr.db" "SELECT Id FROM Users WHERE Username = 'owner'")
sed "s/@MAYA@/$maya/g; s/@OWNER@/$owner/g" "$here/fixture.sql" | sqlite3 "$work/base/db/stellarr.db"
export BASE=$work/base

python3 "$here/compare.py" "$@"
