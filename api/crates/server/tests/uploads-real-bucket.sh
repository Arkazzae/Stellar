#!/bin/sh
# The uploads corpus against a real S3-compatible store, small files only.
#
#   STELLARR_API=/path/to/stellarr-api ./uploads-real-bucket.sh [garage|versitygw|all]
#
# The corpus is sent to this server on the folder store and then on the bucket
# store, and the two answers must be the same line for line. Garage and the
# Versity gateway run from images already on the machine (never pulled), in
# containers called stellarr-port-s3-<kind>, which are removed, with every
# folder made here and every server started, however the script ends.
set -u

here=$(cd "$(dirname "$0")" && pwd)
api=${STELLARR_API:?the path of the stellarr-api binary}
kinds=${1:-all}
port=${UPLOADS_PORT:-5227}
work=$(mktemp -d "${TMPDIR:-/tmp}/stellarr-bucket-run.XXXXXX")
pid=

stop_server() {
  if [ -n "$pid" ]; then kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; pid=; fi
}
cleanup() {
  stop_server
  docker rm -f stellarr-port-s3-garage stellarr-port-s3-versitygw >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT INT TERM

start_server() { # start_server NAME [VAR=value ...]
  name=$1; shift
  rm -rf "$work/$name"; mkdir -p "$work/$name"
  env -i STELLARR_ENVIRONMENT=Testing STELLARR_API_LISTEN="127.0.0.1:$port" \
    STELLARR_DATA_ROOT="$work/$name" \
    Uploads__MinimumFreeDataRootBytes=0 PATH="$PATH" "$@" "$api" > "$work/$name.log" 2>&1 &
  pid=$!
  i=0
  until curl -s -o /dev/null "http://127.0.0.1:$port/health/ready"; do
    i=$((i + 1)); [ "$i" -gt 120 ] && { echo "the server did not start"; cat "$work/$name.log"; return 1; }
    sleep 0.5
  done
}

probe() { # probe NAME -> $work/NAME.out
  code=$(sed -n 's/.*Setup code: \([A-Z0-9-]*\)\..*/\1/p' "$work/$1.log" | tail -1)
  UPLOADS_PROBE_SMALL=1 python3 "$here/uploads-probe.py" "http://127.0.0.1:$port" "$code" "$work/$1/stellarr.db" > "$work/$1.out"
}

status=0
start_server folder || exit 1
probe folder; stop_server
echo "folder store: $(wc -l < "$work/folder.out") answers"

run() { # run KIND ACCESS SECRET ENDPOINT BUCKET
  start_server "$1" Storage__Provider=S3 Storage__Endpoint="$4" Storage__AccessKey="$2" Storage__SecretKey="$3" Storage__Bucket="$5" || { status=1; return; }
  probe "$1"; stop_server
  if diff "$work/folder.out" "$work/$1.out" > "$work/$1.diff"; then
    echo "$1: the same $(wc -l < "$work/$1.out") answers as the folder store"
  else
    echo "$1: DIFFERENT"; head -20 "$work/$1.diff"; status=1
  fi
}

case $kinds in all|garage)
  name=stellarr-port-s3-garage
  mkdir -p "$work/garage/meta" "$work/garage/data"
  cat > "$work/garage/garage.toml" <<TOML
metadata_dir = "/meta"
data_dir = "/data"
db_engine = "sqlite"
replication_factor = 1
rpc_bind_addr = "127.0.0.1:3901"
rpc_public_addr = "127.0.0.1:3901"
rpc_secret = "$(openssl rand -hex 32)"

[s3_api]
s3_region = "us-east-1"
api_bind_addr = "0.0.0.0:3900"

[admin]
api_bind_addr = "127.0.0.1:3903"
TOML
  docker run -d --name "$name" -p "127.0.0.1:19000:3900" -v "$work/garage/garage.toml:/etc/garage.toml:ro" \
    -v "$work/garage/meta:/meta" -v "$work/garage/data:/data" dxflrs/garage:v2.5.0 >/dev/null
  i=0; until docker exec "$name" /garage status >/dev/null 2>&1; do i=$((i + 1)); [ "$i" -gt 60 ] && break; sleep 1; done
  node=$(docker exec "$name" /garage status 2>/dev/null | awk '/^[0-9a-f]{16}/ {print $1; exit}')
  docker exec "$name" /garage layout assign -z dc1 -c 1G "$node" >/dev/null
  docker exec "$name" /garage layout apply --version 1 >/dev/null
  docker exec "$name" /garage key import --yes GK0123456789abcdef01234567 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef -n stellarr-test >/dev/null
  docker exec "$name" /garage key allow --create-bucket GK0123456789abcdef01234567 >/dev/null
  i=0; until curl -s -o /dev/null http://127.0.0.1:19000/; do i=$((i + 1)); [ "$i" -gt 60 ] && break; sleep 1; done
  run garage GK0123456789abcdef01234567 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef http://127.0.0.1:19000 uploads
  docker rm -f "$name" >/dev/null 2>&1
;; esac

case $kinds in all|versitygw)
  name=stellarr-port-s3-versitygw
  docker run -d --name "$name" -p "127.0.0.1:19001:7070" versity/versitygw:v1.11.0 \
    --access stellarr-test --secret stellarr-test-secret-0123456789 posix /tmp >/dev/null
  i=0; until curl -s -o /dev/null http://127.0.0.1:19001/; do i=$((i + 1)); [ "$i" -gt 60 ] && break; sleep 1; done
  run versitygw stellarr-test stellarr-test-secret-0123456789 http://127.0.0.1:19001 uploads
  docker rm -f "$name" >/dev/null 2>&1
;; esac

exit "$status"
