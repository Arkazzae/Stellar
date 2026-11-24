#!/bin/sh
# Runs the offsite upload of backups against a real S3-compatible server, the
# Versity gateway (posix backend), from the image already on the machine:
# a small and a multipart-sized archive, the same ones again, the bytes and
# the checksum read back, wrong credentials, and a server that has stopped.
#
#   CARGO=/path/to/cargo ./backups-bucket.sh
#
# The container is called stellarr-port-s3-backups and listens on 127.0.0.1
# only (port 5272 unless S3_BACKUPS_PORT says otherwise). It is removed, with
# the temporary folder, on every way out: the end, an error, Ctrl-C. Docker
# and curl are needed.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
api=$(cd "$here/../../.." && pwd)
cargo=${CARGO:-cargo}
image=${VERSITYGW_IMAGE:-versity/versitygw:v1.11.0}
port=${S3_BACKUPS_PORT:-5272}
name=stellarr-port-s3-backups
access=stellarr-test
secret=stellarr-test-secret-0123456789

work=$(mktemp -d "${TMPDIR:-/tmp}/stellarr-backups-bucket.XXXXXX")
cleanup() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT INT TERM

docker image inspect "$image" >/dev/null 2>&1 || { echo "backups-bucket.sh: $image is not on this machine" >&2; exit 1; }
docker rm -f "$name" >/dev/null 2>&1 || true
docker run -d --name "$name" -p "127.0.0.1:$port:7070" "$image" \
  --access "$access" --secret "$secret" posix /tmp >/dev/null
i=0
until curl -sS -o /dev/null "http://127.0.0.1:$port/" 2>/dev/null; do
  i=$((i + 1))
  [ "$i" -gt 60 ] && { echo "backups-bucket.sh: the gateway did not answer" >&2; exit 1; }
  sleep 1
done

export STELLARR_TEST_S3_ENDPOINT=http://127.0.0.1:$port
export STELLARR_TEST_S3_ACCESS_KEY=$access
export STELLARR_TEST_S3_SECRET_KEY=$secret
export STELLARR_TEST_S3_STOPPED_FLAG=$work/stopped

cd "$api"
# A backup of an instance whose objects are in the bucket, first, while the
# server is up.
"$cargo" test -p stellarr-api --lib -- --ignored --nocapture \
  backups::archive_tests::a_backup_of_a_bucket

"$cargo" test -p stellarr-api --lib -- --ignored --nocapture \
  backups::offsite::tests::a_real_bucket > "$work/out.txt" 2>&1 &
runner=$!
# When the test says it is ready, the server is stopped under it.
i=0
until grep -q READY_TO_STOP "$work/out.txt" 2>/dev/null; do
  kill -0 "$runner" 2>/dev/null || break
  i=$((i + 1))
  [ "$i" -gt 1200 ] && break
  sleep 1
done
docker rm -f "$name" >/dev/null 2>&1 || true
: > "$work/stopped"
status=0
wait "$runner" || status=$?
cat "$work/out.txt"
exit "$status"
