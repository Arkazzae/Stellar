#!/bin/bash
# The panel's backup routes against the first server, on copies of one
# database file.
#
# The first server is started fresh and seeded through its own routes (an
# owner, two members and a second server's owner). Its database is copied, and
# for each phase (see backups-oracle.mjs) both servers are started on a fresh
# copy, the people sign in, the servers stop, the copies are edited the same
# way (a second step added, a session made stale, the policy changed) and both
# are started again and asked the same questions. The same small files are put
# in both servers' backup folders. The transcripts are compared, then the
# audit lines and the settings row each leaves.
#
# Needs docker, node 26, python3 and sqlite3. Not part of the ordinary run.
#
#   ORACLE=backups ORACLE_PORT=5226 STELLARR_API=path/to/stellarr-api \
#     ORACLE_SCRIPT=path/to/oracle backups-shared-database.sh
#
# The work folder is removed on every way out unless KEEP_WORK=1. The container
# is stopped and removed on every way out.
set -euo pipefail

name=${ORACLE:-backups}
port=${ORACLE_PORT:-5226}
binary=${STELLARR_API:?the path of the stellarr-api binary}
oracle_script=${ORACLE_SCRIPT:?the path of the oracle script}
here=$(cd "$(dirname "$0")" && pwd)
container=stellarr-port-oracle-$name
work=${WORK:-$(mktemp -d)}
oracle=http://127.0.0.1:$port
pid=
ACCESS=AKIAORACLEACCESS1234
SECRET=ORACLESECRETVALUE9876xyz
cleanup() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  "$oracle_script" stop "$name" >/dev/null 2>&1 || true
  docker rm -f "$container" >/dev/null 2>&1 || true
  if [ "${KEEP_WORK:-}" = 1 ]; then echo "work kept: $work" >&2; else rm -rf "$work"; fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

settings=$work/oracle.env
cat > "$settings" <<EOT
RateLimiting__AuthWrite__PermitLimit=500
RateLimiting__Login__PermitLimit=500
RateLimiting__InvitePreview__PermitLimit=500
Backups__InitialDelayMinutes=1440
Maintenance__Enabled=false
Cloudflare__R2__AccessKey=$ACCESS
Cloudflare__R2__SecretKey=$SECRET
EOT
ready() { for _ in $(seq 1 90); do [ "$(curl -s -o /dev/null -w '%{http_code}' "$1/health/ready")" = 200 ] && return; sleep 1; done; echo "$1 is not ready" >&2; exit 1; }

# Small files for both servers' backup folders, old enough to tell from new ones.
python3 - "$work/fixtures" <<'EOT'
import os, sqlite3, sys, zipfile, json
out = sys.argv[1]; os.makedirs(out)
P = "backup-20200101-000000-"
def name(c, ext): return os.path.join(out, P + c * 32 + ext)
def database(path):
    c = sqlite3.connect(path); c.execute("create table t (a)"); c.executemany("insert into t values (?)", [(i,) for i in range(50)]); c.commit(); c.close()
def manifest(objects=1, keys=1, instance=1, parts=("database", "objects", "keys", "instance")):
    return json.dumps({"format": "stellarr-backup", "version": 1, "createdAt": "2020-01-01T00:00:00.0000000Z", "instanceName": "Fixture", "parts": list(parts),
        "databaseBytes": 1, "objectCount": objects, "objectBytes": 5, "missingObjects": [], "keyFiles": keys, "environmentSecrets": [], "instanceFiles": instance}, indent=2)
def archive(path, db_bytes=None, man=None, objects=True, include_db=True):
    database(path + ".tmp")
    data = db_bytes if db_bytes is not None else open(path + ".tmp", "rb").read()
    os.remove(path + ".tmp")
    with zipfile.ZipFile(path, "w") as z:
        if include_db: z.writestr("database/stellarr.db", data, zipfile.ZIP_DEFLATED)
        if objects: z.writestr("objects/a/b.txt", b"hello", zipfile.ZIP_STORED)
        z.writestr("keys/master.key", b"k" * 32, zipfile.ZIP_DEFLATED)
        z.writestr("instance/secrets/s", b"s", zipfile.ZIP_DEFLATED)
        z.writestr("manifest.json", man or manifest(objects=1 if objects else 0), zipfile.ZIP_DEFLATED)
good = name("a", ".zip"); archive(good)
database(name("b", ".db"))
corrupt = name("c", ".zip"); archive(corrupt)
raw = bytearray(open(corrupt, "rb").read()); at = raw.index(b"hello"); raw[at] ^= 0xFF; open(corrupt, "wb").write(raw)
open(name("d", ".zip"), "wb").write(open(good, "rb").read()[:300])
archive(name("e", ".zip"), man=manifest(objects=5))
archive(name("f", ".zip"), db_bytes=b"not a database " * 40)
open(name("0", ".db"), "wb").write(b"just some text\n")
open(name("2", ".zip"), "wb").write(b"not a zip at all")
archive(name("3", ".zip"), man="{}")
archive(name("4", ".zip"), include_db=False)
archive(name("5", ".zip"), man=manifest(keys=3))
archive(name("6", ".zip"), man=manifest(instance=3))
open(good + ".verified", "w").write("2026-01-01T00:00:00.0000000Z")
open(os.path.join(out, ".backup-x.zip.partial"), "w").write("half")
open(os.path.join(out, "backup-note.txt"), "w").write("note")
os.makedirs(os.path.join(out, "backup-folder.zip"))
for f in os.listdir(out):
    p = os.path.join(out, f)
    if os.path.isfile(p): os.utime(p, (1577836800, 1577836800))
EOT

"$oracle_script" stop "$name" >/dev/null 2>&1 || true
docker rm -f "$container" >/dev/null 2>&1 || true
ORACLE_ENV_FILE=$settings "$oracle_script" start "$name" "$port" Production > "$work/start.log"
setup=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/start.log" | tail -1)
node "$here/backups-oracle.mjs" seed --base "$oracle" --setup "$setup" --state "$work/state.json"
docker stop -t 20 "$container" >/dev/null
docker cp "$container:/tmp/data/stellarr.db" "$work/seeded.db"

mkdir -p "$work/rust"
start_rust() {
  stop_rust
  : > "$work/rust/log.txt"
  env ASPNETCORE_ENVIRONMENT=Production Rtc__TurnSharedSecret=x STELLARR_DATA_ROOT="$work/rust" \
    "ConnectionStrings__Database=Data Source=$work/rust/stellarr.db" ASPNETCORE_URLS=http://127.0.0.1:0 \
    RateLimiting__AuthWrite__PermitLimit=500 RateLimiting__Login__PermitLimit=500 RateLimiting__InvitePreview__PermitLimit=500 \
    Backups__InitialDelayMinutes=1440 Maintenance__Enabled=false \
    "Cloudflare__R2__AccessKey=$ACCESS" "Cloudflare__R2__SecretKey=$SECRET" \
    "Operations__PublicUrl=http://127.0.0.1:$port" DataRoot="$work/rust" \
    "$binary" >> "$work/rust/log.txt" 2>&1 &
  pid=$!
  for _ in $(seq 1 50); do grep -q 'Now listening on' "$work/rust/log.txt" && break; sleep 0.2; done
  rust=http://127.0.0.1:$(sed -n 's/.*listening on: http:\/\/127.0.0.1:\([0-9]*\).*/\1/p' "$work/rust/log.txt")
  ready "$rust"
}
stop_rust() { if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; pid=; fi; }
start_oracle() { docker start "$container" >/dev/null; ready "$oracle"; }
stop_oracle() { docker stop -t 20 "$container" >/dev/null || true; }
put_oracle() { # a database file, and fresh fixtures in the backup folder
  docker cp "$1" "$container:/tmp/data/stellarr.db"; start_oracle
  docker exec "$container" sh -c 'rm -rf /tmp/data/backups; mkdir -p /tmp/data/backups' >/dev/null
  docker cp "$work/fixtures/." "$container:/tmp/data/backups/"
}
put_rust() { cp "$1" "$work/rust/stellarr.db"; rm -f "$work/rust/stellarr.db-wal" "$work/rust/stellarr.db-shm"; rm -rf "$work/rust/backups"; mkdir -p "$work/rust/backups"; cp -Rp "$work/fixtures/." "$work/rust/backups/"; }

NOW="strftime('%Y-%m-%d %H:%M:%f', 'now')"
OWNER="(SELECT Id FROM Users WHERE Username = 'owner')"
method() { echo "INSERT INTO MfaMethods (Id, UserId, Type, Name, CreatedAt, ConfirmedAt) SELECT upper(hex(randomblob(4))) || '-0000-4000-8000-' || upper(hex(randomblob(6))), Id, 'totp', 'Phone', datetime('now'), datetime('now') FROM Users WHERE Username IN ($1);"; }
policy() { echo "INSERT OR REPLACE INTO OperationsSettings (ServerId, PublicUrl, TimeZone, MailingEnabled, MailFromAddress, InviteMailEnabled, RecoveryMailEnabled, AlertMailEnabled, BackupRetentionDays, LogRetentionDays, OrphanObjectRetentionDays, AutomaticCleanupEnabled, UpdatedAt, MfaRequirement) VALUES ((SELECT Id FROM Servers ORDER BY CreatedAt LIMIT 1), '', 'UTC', 0, '', 0, 0, 0, 14, 30, 1, 1, $NOW, 'cockpit');"; }
edit_for() {
  case "$1" in
    stale) echo "$(method "'owner', 'kim'") UPDATE UserSessions SET MfaCompletedAt = NULL WHERE UserId IN (SELECT Id FROM Users WHERE Username IN ('owner', 'kim'));" ;;
    fresh) echo "$(method "'owner'") UPDATE UserSessions SET MfaCompletedAt = strftime('%Y-%m-%d %H:%M:%f', 'now', '-60 seconds') WHERE UserId = $OWNER;" ;;
    cockpit-open) echo "$(policy) UPDATE UserSessions SET MfaCompletedAt = NULL WHERE UserId = $OWNER;" ;;
    cockpit-passed) echo "$(policy) UPDATE UserSessions SET MfaCompletedAt = strftime('%Y-%m-%d %H:%M:%f', 'now', '-60 seconds') WHERE UserId = $OWNER;" ;;
    disabled) echo "UPDATE Users SET IsDisabled = 1 WHERE Username = 'owner';" ;;
    *) echo "" ;;
  esac
}

failed=0
for phase in plain stale fresh cockpit-open cockpit-passed disabled; do
  echo "== $phase"
  put_oracle "$work/seeded.db"; put_rust "$work/seeded.db"; start_rust
  node "$here/backups-oracle.mjs" login --base "$oracle" --state "$work/state.json" --out "$work/jar-first.json"
  node "$here/backups-oracle.mjs" login --base "$rust" --state "$work/state.json" --out "$work/jar-this.json"
  stop_rust; stop_oracle
  docker cp "$container:/tmp/data/stellarr.db" "$work/p-oracle.db"; cp "$work/rust/stellarr.db" "$work/p-rust.db"
  edit=$(edit_for "$phase")
  if [ -n "$edit" ]; then
    sqlite3 "$work/p-oracle.db" "$edit"; sqlite3 "$work/p-rust.db" "$edit"
  fi
  docker cp "$work/p-oracle.db" "$container:/tmp/data/stellarr.db"; start_oracle
  docker exec "$container" sh -c 'rm -rf /tmp/data/backups/*; true'; docker cp "$work/fixtures/." "$container:/tmp/data/backups/"
  cp "$work/p-rust.db" "$work/rust/stellarr.db"; start_rust
  node "$here/backups-oracle.mjs" ask --phase "$phase" --secrets "$SECRET" --base "$oracle" --state "$work/state.json" --cookies "$work/jar-first.json" --out "$work/$phase-first.txt"
  node "$here/backups-oracle.mjs" ask --phase "$phase" --secrets "$SECRET" --base "$rust" --state "$work/state.json" --cookies "$work/jar-this.json" --out "$work/$phase-this.txt"
  if python3 "$here/backups-diff.py" "$work/$phase-first.txt" "$work/$phase-this.txt" > "$work/$phase.diff"; then echo "the transcripts are the same"; else failed=1; cut -c1-${DIFF_WIDTH:-600} "$work/$phase.diff" | head -c ${DIFF_BYTES:-40000}; fi
  # Secrets must be in no answer and no log line.
  for value in "$ACCESS" "$SECRET"; do
    grep -qF "$value" "$work/rust/log.txt" && { echo "SECRET IN OUR LOG"; failed=1; }
    docker logs "$container" 2>&1 | grep -qF "$value" && { echo "SECRET IN THE FIRST SERVER'S LOG"; failed=1; }
    grep -qF "$value" "$work/$phase-this.txt" && { echo "SECRET IN OUR ANSWERS"; failed=1; }
  done
  if [ "$phase" = plain ]; then
    stop_rust; stop_oracle
    docker cp "$container:/tmp/data/stellarr.db" "$work/after-oracle.db"; cp "$work/rust/stellarr.db" "$work/after-rust.db"
    for side in oracle rust; do
      { sqlite3 "$work/after-$side.db" "SELECT Action, Summary FROM ServerAuditEntries ORDER BY CreatedAt" | sed -E 's/backup-[0-9]{8}-[0-9]{6}-[0-9a-f]{32}/backup-<id>/'
        sqlite3 "$work/after-$side.db" "SELECT PublicUrl, TimeZone, MailingEnabled, MailFromAddress, InviteMailEnabled, RecoveryMailEnabled, AlertMailEnabled, BackupRetentionDays, LogRetentionDays, OrphanObjectRetentionDays, AutomaticCleanupEnabled, MfaRequirement, TrustedDeviceDays, UpdatedAt > '2026' FROM OperationsSettings"
        sqlite3 "$work/after-$side.db" "SELECT Name FROM InstanceSecrets ORDER BY Name"; } > "$work/rows-$side.txt"
    done
    diff "$work/rows-oracle.txt" "$work/rows-rust.txt" && echo "the audit lines and the settings row are the same" || failed=1
  fi
  stop_rust; stop_oracle
done
exit $failed
