#!/bin/bash
# What writing a large history costs: writes MESSAGES generated messages into one
# channel of a fresh database, in pages of a hundred as the reader gives them,
# while a second connection asks for the write lock every few milliseconds. Prints
# how long a call took, how long that second connection waited, and the peak
# memory of the process, which also holds the test's own copy of the Discord server.
#   MESSAGES  default 100000
#   PAUSE     0 writes without the pause between transactions (default 1)
#   ACARGO    the build wrapper (default: acargo on the PATH)
#   KEEP=1    leave the work folder
set -u
here="$(cd "$(dirname "$0")" && pwd)"
api="$(cd "$here/../../../.." && pwd)"
ACARGO="${ACARGO:-acargo}"
work="$(mktemp -d "${TMPDIR:-/tmp}/discordw-bulk.XXXXXX")"
cleanup() { if [ "${KEEP:-0}" = 1 ]; then echo "kept $work"; else rm -rf "$work"; fi; }
trap cleanup EXIT
trap 'exit 130' INT TERM

binary="$(cd "$api" && "$ACARGO" test -p stellarr-api --test api --no-run 2>&1 | sed -n 's/.*(\(.*\/api-[0-9a-f]*\))$/\1/p' | tail -1)"
[ -x "$binary" ] || { echo "no test binary: $binary"; exit 2; }
mkdir -p "$work/base"
DISCORDW_OUT="$work/base" "$binary" --ignored --exact discord_import_proof::proof_base >"$work/base.log" 2>&1 || { cat "$work/base.log"; exit 2; }
python3 "$here/gen_guild.py" "$work/guild.json" --messages 0 >/dev/null
server="$(python3 -c "import json;print(json.load(open('$work/base/base.json'))['server'])")"
user="$(python3 -c "import json;print(json.load(open('$work/base/base.json'))['user'])")"
cp "$work/base/base.db" "$work/bulk.db"
DISCORDW_DB="$work/bulk.db" DISCORDW_GUILD="$work/guild.json" DISCORDW_SERVER="$server" DISCORDW_USER="$user" \
  DISCORDW_MESSAGES="${MESSAGES:-100000}" DISCORDW_PAUSE="${PAUSE:-1}" \
  /usr/bin/time -l "$binary" --ignored --exact discord_import_proof::proof_bulk --nocapture 2>"$work/time.log" | grep -E "^(bulk|mean) "
grep -E "maximum resident|peak memory" "$work/time.log"
echo "database after: $(( $(stat -f %z "$work/bulk.db") / 1048576 )) MB"
