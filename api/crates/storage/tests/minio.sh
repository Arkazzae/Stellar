#!/bin/sh
# Runs the tests that need a real S3-compatible server against MinIO, once over
# plain HTTP and once over HTTPS, and removes everything it made.
#
#   CARGO=/path/to/cargo ./minio.sh          both runs
#   ./minio.sh http                          only the plain-HTTP run
#   ./minio.sh https                         only the HTTPS run
#   ./minio.sh certs DIR                     only make the test certificates in DIR
#
# What it needs: docker, curl, openssl and cargo. The MinIO image must be on
# the machine, or pullable as minio/minio or quay.io/minio/minio (the script
# says so and stops if neither is). It never removes the image.
#
# Environment:
#   CARGO                     the cargo to run, with -p arguments (default: cargo)
#   MINIO_IMAGE               the image to use (default: the first of the two names that exists)
#   MINIO_HTTP_PORT/_HTTPS_PORT   host ports on 127.0.0.1 (19000 / 19443)
#   STELLARR_TEST_LARGE_MIB   size of the large-object test (300)
#   STELLARR_TEST_TRANSCRIPTS a folder to keep the transcripts of the questions in
#
# The container is called stellarr-port-minio and listens on 127.0.0.1 only.
# Its data and the certificates live in a temporary folder that is removed on
# exit, however the script ends.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
api=$(cd "$here/../../.." && pwd)
cargo=${CARGO:-cargo}
name=stellarr-port-minio
user=stellarr-test
secret=stellarr-test-secret-0123456789
http_port=${MINIO_HTTP_PORT:-19000}
https_port=${MINIO_HTTPS_PORT:-19443}

make_certs() {
  dir=$1
  mkdir -p "$dir"
  # A root, and a certificate for 127.0.0.1 and localhost that it signs. A
  # certificate that is its own root cannot be used as a server's.
  openssl req -x509 -newkey rsa:2048 -nodes -keyout "$dir/ca.key" -out "$dir/ca.crt" \
    -days 2 -subj "/CN=stellarr storage test root" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -keyout "$dir/private.key" -out "$dir/leaf.csr" \
    -subj "/CN=localhost" 2>/dev/null
  printf '%s\n' \
    'subjectAltName=DNS:localhost,IP:127.0.0.1' \
    'basicConstraints=critical,CA:FALSE' \
    'keyUsage=critical,digitalSignature,keyEncipherment' \
    'extendedKeyUsage=serverAuth' > "$dir/leaf.ext"
  openssl x509 -req -in "$dir/leaf.csr" -CA "$dir/ca.crt" -CAkey "$dir/ca.key" \
    -CAcreateserial -out "$dir/public.crt" -days 2 -extfile "$dir/leaf.ext" 2>/dev/null
  openssl verify -CAfile "$dir/ca.crt" "$dir/public.crt" >/dev/null
  chmod 644 "$dir/private.key"
}

if [ "${1:-}" = certs ]; then
  make_certs "${2:?a folder}"
  echo "minio.sh: certificates made in $2"
  exit 0
fi

what=${1:-both}
case $what in
  both|http|https) ;;
  *) echo "minio.sh: http, https, both or certs DIR" >&2; exit 2 ;;
esac

work=$(mktemp -d "${TMPDIR:-/tmp}/stellarr-minio.XXXXXX")
cleanup() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT INT TERM

# The image: the one named, or the first of the usual names that is here or can be had.
image=${MINIO_IMAGE:-}
if [ -z "$image" ]; then
  for candidate in minio/minio quay.io/minio/minio; do
    if docker image inspect "$candidate" >/dev/null 2>&1 || docker pull "$candidate" >/dev/null 2>&1; then
      image=$candidate
      break
    fi
  done
fi
if [ -z "$image" ]; then
  echo "minio.sh: no MinIO image: neither minio/minio nor quay.io/minio/minio is here or can be pulled" >&2
  exit 3
fi
echo "minio.sh: image $image ($(docker image inspect --format '{{index .RepoDigests 0}}' "$image" 2>/dev/null || echo 'no digest'))"

docker rm -f "$name" >/dev/null 2>&1 || true

start() { # start http|https
  scheme=$1
  mkdir -p "$work/data-$scheme"
  if [ "$scheme" = https ]; then
    make_certs "$work/certs"
    port=$https_port
    mounts="-v $work/certs/public.crt:/root/.minio/certs/public.crt:ro -v $work/certs/private.key:/root/.minio/certs/private.key:ro"
    cacert="--cacert $work/certs/ca.crt"
  else
    port=$http_port
    mounts=""
    cacert=""
  fi
  # shellcheck disable=SC2086
  docker run -d --name "$name" -p "127.0.0.1:$port:9000" \
    -e MINIO_ROOT_USER="$user" -e MINIO_ROOT_PASSWORD="$secret" \
    -v "$work/data-$scheme:/data" $mounts "$image" server /data >/dev/null
  i=0
  # shellcheck disable=SC2086
  until curl -fsS $cacert "$scheme://127.0.0.1:$port/minio/health/live" >/dev/null 2>&1; do
    i=$((i + 1))
    if [ "$i" -gt 60 ]; then
      echo "minio.sh: MinIO did not become ready" >&2
      docker logs "$name" 2>&1 | tail -20 >&2
      exit 4
    fi
    sleep 1
  done
  echo "minio.sh: MinIO is up on $scheme://127.0.0.1:$port"
  docker logs "$name" 2>&1 | grep -i -E 'version|release' | head -3 || true
}

stop() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$work/data-http" "$work/data-https"
}

run() { # run http|https
  scheme=$1
  start "$scheme"
  if [ "$scheme" = https ]; then
    port=$https_port
    export STELLARR_TEST_S3_CA="$work/certs/ca.crt"
  else
    port=$http_port
    unset STELLARR_TEST_S3_CA || true
  fi
  export STELLARR_TEST_S3_ENDPOINT="$scheme://127.0.0.1:$port"
  export STELLARR_TEST_S3_ACCESS_KEY="$user"
  export STELLARR_TEST_S3_SECRET_KEY="$secret"
  if [ -n "${STELLARR_TEST_TRANSCRIPTS:-}" ]; then
    mkdir -p "$STELLARR_TEST_TRANSCRIPTS"
  fi
  cd "$api"
  # The contract and the questions. A failure is reported after the large
  # test too, so that one run says everything.
  status=0
  "$cargo" test -p stellarr-storage --test minio -- --ignored --nocapture --test-threads=2 || status=$?
  if [ "$scheme" = http ]; then
    "$cargo" test -p stellarr-storage --test large -- --ignored --nocapture \
      the_bucket_store_streams_a_large_object_through_a_real_store || status=$?
  fi
  stop
  return "$status"
}

overall=0
case $what in
  both|http) run http || overall=$? ;;
esac
case $what in
  both|https) run https || overall=$? ;;
esac
exit "$overall"
