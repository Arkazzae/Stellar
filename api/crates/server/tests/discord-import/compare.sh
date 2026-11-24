#!/bin/bash
# Imports one generated Discord server with the new writer and with the first
# server's, each into its own copy of the same database, and compares what they
# wrote; then interrupts each writer at every kind of place, continues it, and
# compares that with the uninterrupted run of the same writer.
#
# Needs docker with the development image, python3, and the first server
# published (STELLARR_FIRST_SERVER, a folder with Stellarr.Api.dll in it).
#   ACARGO          the build wrapper (default: acargo on the PATH)
#   MESSAGES        how many messages the generated server has (default 3000)
#   KEEP=1          leave the work folder
#   HOSTILE=0       leave the hostile messages out
set -u
here="$(cd "$(dirname "$0")" && pwd)"
api="$(cd "$here/../../../.." && pwd)"
ACARGO="${ACARGO:-acargo}"
first="${STELLARR_FIRST_SERVER:?set STELLARR_FIRST_SERVER}"
image="${IMAGE:-stellarr-api-dev:local}"
work="$(mktemp -d "${TMPDIR:-/tmp}/discordw.XXXXXX")"
name="discordw-net-$$"
pids=""

cleanup() {
  echo "ended with status $?"
  for p in $pids; do kill -9 "$p" 2>/dev/null; done
  docker kill "$name" >/dev/null 2>&1
  docker rm -f "$name" >/dev/null 2>&1
  if [ "${KEEP:-0}" = 1 ]; then echo "kept $work"; else rm -rf "$work"; fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

fail=0
note() { echo "$*"; }
bad() { echo "DIFFERENT: $*"; fail=1; }

# ---- the pieces
(cd "$api" && "$ACARGO" test -p stellarr-api --test api --no-run 2>&1 | sed -n 's/.*(\(.*\/api-[0-9a-f]*\))$/\1/p' | tail -1) > "$work/binary"
binary="$(cat "$work/binary")"
[ -x "$binary" ] || { echo "no test binary: $binary"; exit 2; }

mkdir -p "$work/harness"
cp "$here"/harness/Harness.csproj "$here"/harness/Program.cs "$work/harness/"
docker run --rm -v "$work/harness:/h" -v "$first:/first:ro" -e STELLARR_FIRST_SERVER=/first "$image" \
  sh -c 'cd /h && dotnet build -c Release -o /h/out 2>&1 | grep -E " error |Build FAILED"' && { echo "harness did not build"; exit 2; }

python3 "$here/gen_guild.py" "$work/guild.json" --messages "${MESSAGES:-3000}" $( [ "${HOSTILE:-1}" = 1 ] && echo --hostile )
mkdir -p "$work/base"
DISCORDW_OUT="$work/base" "$binary" --ignored --exact discord_import_proof::proof_base --nocapture >"$work/base.log" 2>&1 \
  || { cat "$work/base.log"; exit 2; }
server="$(python3 -c "import json;print(json.load(open('$work/base/base.json'))['server'])")"
user="$(python3 -c "import json;print(json.load(open('$work/base/base.json'))['user'])")"

# $1 database; $2 stop-after (0 for none); runs the new writer once
rust_run() {
  DISCORDW_DB="$1" DISCORDW_GUILD="$work/guild.json" DISCORDW_SERVER="$server" DISCORDW_USER="$user" \
    DISCORDW_STOP_AFTER="$2" "$binary" --ignored --exact discord_import_proof::proof_write --nocapture >"$1.log" 2>&1
}
net_run() {
  docker run --rm --name "$name" -e DOTNET_DbgEnableMiniDump=0 -v "$work:/w" -v "$work/harness/out:/h:ro" -v "$first:/first:ro" -e STOP_AFTER_TX="$2" \
    "$image" dotnet /h/Harness.dll "/w/$(basename "$1")" /w/guild.json "$server" "$user" >"$1.log" 2>&1
}
dump() { python3 "$here/dump.py" "$1" "$1.txt"; }
same() { # $1 db a, $2 db b, $3 what
  dump "$1"; dump "$2"
  if diff -q "$1.txt" "$2.txt" >/dev/null; then note "same: $3"; else bad "$3"; diff "$1.txt" "$2.txt" | head -${DIFF_LINES:-12}; fi
}

# ---- the two writers on the same database
cp "$work/base/base.db" "$work/rust.db"; cp "$work/base/base.db" "$work/net.db"
rust_run "$work/rust.db" 0 || { cat "$work/rust.db.log" | tail -20; exit 2; }
net_run "$work/net.db" 0 || { tail -20 "$work/net.db.log"; exit 2; }
same "$work/rust.db" "$work/net.db" "the new writer and the first server's wrote the same"
note "messages written: $(python3 -c "import sqlite3;print(sqlite3.connect('$work/rust.db').execute('select count(*) from DiscordImportMessages where Outcome = 0').fetchone()[0])")"

# ---- interrupted at a boundary between two calls (new) or two transactions (first), then continued
for n in ${STOPS:-1 2 3 5 8 13 21 34 55 89}; do
  cp "$work/base/base.db" "$work/rust-$n.db"
  rust_run "$work/rust-$n.db" "$n"; rust_run "$work/rust-$n.db" 0 || { tail -5 "$work/rust-$n.db.log"; bad "continue after $n calls failed"; continue; }
  same "$work/rust-$n.db" "$work/rust.db" "new writer, stopped after call $n and continued"
  cp "$work/base/base.db" "$work/net-$n.db"
  net_run "$work/net-$n.db" "$n"; net_run "$work/net-$n.db" 0 || { tail -5 "$work/net-$n.db.log"; bad "first server's, continue after $n failed"; continue; }
  same "$work/net-$n.db" "$work/net.db" "first server's writer, stopped before pause $n and continued"
  rm -f "$work/rust-$n.db"* "$work/net-$n.db"*
done

# ---- killed in the middle of whatever it was doing, at random times, then continued
for round in ${ROUNDS:-1 2 3 4 5 6}; do
  cp "$work/base/base.db" "$work/rust-k.db"
  for kill_at in 0.4 0.9 1.6 2.4; do
    DISCORDW_DB="$work/rust-k.db" DISCORDW_GUILD="$work/guild.json" DISCORDW_SERVER="$server" DISCORDW_USER="$user" \
      "$binary" --ignored --exact discord_import_proof::proof_write --nocapture >/dev/null 2>&1 &
    p=$!; pids="$pids $p"
    perl -e "select(undef,undef,undef,$kill_at + $round * 0.037)"; kill -9 "$p" 2>/dev/null; wait "$p" 2>/dev/null
  done
  rust_run "$work/rust-k.db" 0 || { tail -5 "$work/rust-k.db.log"; bad "new writer, continue after kills failed"; continue; }
  same "$work/rust-k.db" "$work/rust.db" "new writer, killed 4 times (round $round) and continued"
  rm -f "$work/rust-k.db"*

  cp "$work/base/base.db" "$work/net-k.db"
  for kill_at in 1.6 2.4 3.2 4.0; do
    docker run --rm --name "$name" -e DOTNET_DbgEnableMiniDump=0 -v "$work:/w" -v "$work/harness/out:/h:ro" -v "$first:/first:ro" \
      "$image" dotnet /h/Harness.dll /w/net-k.db /w/guild.json "$server" "$user" >/dev/null 2>&1 &
    p=$!; pids="$pids $p"
    perl -e "select(undef,undef,undef,$kill_at + $round * 0.11)"; docker kill "$name" >/dev/null 2>&1; wait "$p" 2>/dev/null
  done
  net_run "$work/net-k.db" 0 || { tail -5 "$work/net-k.db.log"; bad "first server's, continue after kills failed"; continue; }
  same "$work/net-k.db" "$work/net.db" "first server's writer, killed 4 times (round $round) and continued"
  rm -f "$work/net-k.db"*
done

[ "$fail" = 0 ] && echo "ALL SAME" || echo "SOME DIFFERENT"
exit "$fail"
