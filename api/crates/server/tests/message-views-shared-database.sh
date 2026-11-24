#!/bin/bash
# Reactions, pins, threads, search and the channel index, against the first
# server on copies of one database file.
#
# A fresh first server (`oracle start <name> <port> Production`, with the
# rate limits of `RateLimiting__AuthWrite__PermitLimit` and `...Login...`
# raised, see the brief) is filled through its own routes (people, channels,
# overrides, messages, uploads, reactions, pins), stopped, and its database
# copied out and filled further with thousands of messages in several
# languages (equal moments, a pinned list over its limit, attachments out of
# id order, embeds, reactions). Both servers then run on copies of the file
# and are asked the same things:
#
#   - the pin, thread and channel reads, for every person and every channel;
#   - the index walked to its end for every kind, text and limit, every page and
#     cursor compared, and each server's cursors fed to the other;
#   - some hundreds of searches, among them every character that means
#     something to the search syntax, in several languages;
#   - reactions (every awkward emoji) and pins, request by request, and the rows
#     they leave;
#   - the events each change tells the hub, as each person on it hears them;
#   - the first server's own code (a small program in dn/) and this one's on the
#     same inputs: the search expression, the number of a limit, the cursor,
#     a quote of a message, ids, case and white space.
#
# The server it starts is stopped and the folder it makes removed however it
# ends; KEEP_WORK=1 keeps the folder, WORK=<folder> uses (and keeps) one.
#
# Needs docker, python3, node (26), the .NET SDK and the first server's
# container. Not part of the ordinary run. Prints every difference, and
# "differences 0" lines when there are none.
#
#   ORACLE=msgviews ORACLE_PORT=5215 RUST_PORT=5316 DOTNET=~/.dotnet/dotnet \
#     STELLARR_API=path/to/stellarr-api ACARGO=path/to/acargo message-views-shared-database.sh
set -euo pipefail
here=$(cd "$(dirname "$0")/message-views" && pwd)
api=$(cd "$here/../../../.." && pwd)
made=
if [ -z "${WORK:-}" ]; then WORK=$(mktemp -d); made=1; fi
work=$WORK
dotnet=${DOTNET:-dotnet}
acargo=${ACARGO:?the path of acargo}
mkdir -p "$work/work" && cd "$work"
export PYTHONPATH=$here ORACLE=${ORACLE:-msgviews} ORACLE_PORT=${ORACLE_PORT:-5215} RUST_PORT=${RUST_PORT:-5316}
export STELLARR_API=${STELLARR_API:?the path of the stellarr-api binary}
container=stellarr-port-oracle-$ORACLE

# However this ends, stop the server it started and remove the folder it made
# (KEEP_WORK=1 keeps it). The first server's container is the caller's.
cleanup() {
  for pid in $(lsof -tiTCP:"$RUST_PORT" -sTCP:LISTEN 2>/dev/null); do kill "$pid" 2>/dev/null || true; done
  if [ -n "$made" ] && [ -z "${KEEP_WORK:-}" ]; then rm -rf "$work"; elif [ -n "${KEEP_WORK:-}" ]; then echo "kept $work"; fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

python3 "$here/populate_a.py"
docker stop -t 15 "$container" >/dev/null
docker cp "$container:/tmp/data/stellarr.db" work/base.db
python3 "$here/populate_b.py" work/base.db work/shared.db

reset() { python3 -c "import harness; harness.reset_oracle('work/shared.db')" && python3 "$here/restart_rust.py" work/shared.db; }

# The code compared on the same inputs.
(cd "$here/dn" && "$dotnet" build -c Release -o "$work/dn-out" >/dev/null)
export DN_DLL=$work/dn-out/dn.dll
python3 "$here/differential.py" 5
(cd "$api" && MESSAGE_VIEWS_CASES=$work/work/cases.txt MESSAGE_VIEWS_ANSWERS=$work/work/rust-answers.txt \
  "$acargo" test -p stellarr-api --lib answers_for_the_cases -- --ignored)
python3 "$here/compare_differential.py"

reset
python3 "$here/cmp_read1.py"
python3 "$here/cmp_walk.py"
python3 "$here/cmp_search.py"

reset
python3 "$here/cmp_write.py"
python3 "$here/cmp_rows.py"

reset
node "$here/events.mjs" --base "http://127.0.0.1:$ORACLE_PORT" --state state.json --messages work/ids.json --out work/events-oracle.txt
reset
node "$here/events.mjs" --base "http://127.0.0.1:$RUST_PORT" --state state.json --messages work/ids.json --out work/events-rust.txt
diff work/events-oracle.txt work/events-rust.txt && echo "events: same"
