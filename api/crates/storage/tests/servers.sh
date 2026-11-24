#!/bin/sh
# Runs the tests that need a real S3-compatible server against Garage and the
# Versity gateway: the contract every store keeps, the twelve questions about
# what a server does with the library's requests (and a thirteenth, about the
# conditional create), and the large-object run. Everything it makes is removed.
#
#   CARGO=/path/to/cargo ./servers.sh [garage|versitygw|all] [http|https|both]
#   ./servers.sh certs DIR        only make the test certificates in DIR
#
# Both servers need their image on the machine (or pullable); it is never
# removed. Docker, curl, openssl and cargo are needed. The images:
#
#   dxflrs/garage:v2.5.0          Garage. Plain HTTP only: it has no TLS of its own.
#   versity/versitygw:v1.11.0     the Versity S3 gateway, posix backend. HTTP and HTTPS.
#
# Environment:
#   CARGO                      the cargo to run, with -p arguments (default: cargo)
#   GARAGE_IMAGE, VERSITYGW_IMAGE   other image names or tags
#   S3_GARAGE_PORT, S3_VERSITYGW_PORT, S3_VERSITYGW_TLS_PORT   host ports on
#                              127.0.0.1 (19000, 19001, 19443)
#   STELLARR_TEST_LARGE_MIB    size of the large-object test (300)
#   STELLARR_TEST_TRANSCRIPTS  a folder to keep the transcripts of the questions in
#
# A container is called stellarr-port-s3-<kind>, listens on 127.0.0.1 only, and
# is removed on exit with the temporary folder that holds its data and
# certificates, however the script ends. Versity keeps its data inside the
# container, because its posix backend needs extended attributes, which a
# bind-mounted folder may not have.
#
# What the conditional create does on each server is found out, not assumed:
# the questions run first, say whether `If-None-Match: *` was honoured, and the
# contract run is told.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
api=$(cd "$here/../../.." && pwd)
cargo=${CARGO:-cargo}
garage_image=${GARAGE_IMAGE:-dxflrs/garage:v2.5.0}
versity_image=${VERSITYGW_IMAGE:-versity/versitygw:v1.11.0}
garage_port=${S3_GARAGE_PORT:-19000}
versity_port=${S3_VERSITYGW_PORT:-19001}
versity_tls_port=${S3_VERSITYGW_TLS_PORT:-19443}

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
  echo "servers.sh: certificates made in $2"
  exit 0
fi

kinds=${1:-all}
schemes=${2:-both}
case $kinds in all|garage|versitygw) ;; *) echo "servers.sh: garage, versitygw or all; then http, https or both; or certs DIR" >&2; exit 2 ;; esac
case $schemes in both|http|https) ;; *) echo "servers.sh: http, https or both" >&2; exit 2 ;; esac

work=$(mktemp -d "${TMPDIR:-/tmp}/stellarr-s3.XXXXXX")
cleanup() {
  docker rm -f stellarr-port-s3-garage stellarr-port-s3-versitygw >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT INT TERM

have_image() { # have_image NAME: here, or pulled
  docker image inspect "$1" >/dev/null 2>&1 || docker pull "$1" >/dev/null 2>&1
}

wait_for() { # wait_for URL [curl options]
  url=$1
  shift
  i=0
  until curl -sS -o /dev/null "$@" "$url" 2>/dev/null; do
    i=$((i + 1))
    if [ "$i" -gt 60 ]; then
      echo "servers.sh: $url did not answer" >&2
      return 1
    fi
    sleep 1
  done
}

# Each start_* sets access, secret, endpoint and cacert (empty for plain HTTP).
start_garage() {
  name=stellarr-port-s3-garage
  mkdir -p "$work/garage/meta" "$work/garage/data"
  rpc_secret=$(openssl rand -hex 32)
  cat > "$work/garage/garage.toml" <<TOML
metadata_dir = "/meta"
data_dir = "/data"
db_engine = "sqlite"
replication_factor = 1
rpc_bind_addr = "127.0.0.1:3901"
rpc_public_addr = "127.0.0.1:3901"
rpc_secret = "$rpc_secret"

[s3_api]
s3_region = "us-east-1"
api_bind_addr = "0.0.0.0:3900"

[admin]
api_bind_addr = "127.0.0.1:3903"
TOML
  docker run -d --name "$name" -p "127.0.0.1:$garage_port:3900" \
    -v "$work/garage/garage.toml:/etc/garage.toml:ro" \
    -v "$work/garage/meta:/meta" -v "$work/garage/data:/data" "$garage_image" >/dev/null
  i=0
  until docker exec "$name" /garage status >/dev/null 2>&1; do
    i=$((i + 1))
    [ "$i" -gt 60 ] && { echo "servers.sh: garage did not start" >&2; docker logs "$name" 2>&1 | tail -20 >&2; return 1; }
    sleep 1
  done
  node=$(docker exec "$name" /garage status 2>/dev/null | awk '/^[0-9a-f]{16}/ {print $1; exit}')
  docker exec "$name" /garage layout assign -z dc1 -c 1G "$node" >/dev/null
  docker exec "$name" /garage layout apply --version 1 >/dev/null
  access=GK0123456789abcdef01234567
  secret=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
  docker exec "$name" /garage key import --yes "$access" "$secret" -n stellarr-test >/dev/null
  docker exec "$name" /garage key allow --create-bucket "$access" >/dev/null
  endpoint=http://127.0.0.1:$garage_port
  cacert=
  wait_for "$endpoint/"
  echo "servers.sh: garage up on $endpoint ($(docker exec "$name" /garage --version 2>&1 | head -1))"
}

start_versitygw() { # start_versitygw http|https
  scheme=$1
  name=stellarr-port-s3-versitygw
  access=stellarr-test
  secret=stellarr-test-secret-0123456789
  if [ "$scheme" = https ]; then
    make_certs "$work/certs"
    docker run -d --name "$name" -p "127.0.0.1:$versity_tls_port:7070" -v "$work/certs:/certs:ro" \
      "$versity_image" --access "$access" --secret "$secret" \
      --cert /certs/public.crt --key /certs/private.key posix /tmp >/dev/null
    endpoint=https://127.0.0.1:$versity_tls_port
    cacert=$work/certs/ca.crt
    wait_for "$endpoint/" --cacert "$cacert"
  else
    docker run -d --name "$name" -p "127.0.0.1:$versity_port:7070" \
      "$versity_image" --access "$access" --secret "$secret" posix /tmp >/dev/null
    endpoint=http://127.0.0.1:$versity_port
    cacert=
    wait_for "$endpoint/"
  fi
  echo "servers.sh: versitygw up on $endpoint ($(docker logs "$name" 2>&1 | grep -m1 -o 'Version v[0-9.]*'))"
}

stop() {
  docker rm -f stellarr-port-s3-garage stellarr-port-s3-versitygw >/dev/null 2>&1 || true
  rm -rf "$work/garage" "$work/certs"
}

# run KIND SCHEME
run() {
  kind=$1
  scheme=$2
  case $kind in
    garage) start_garage ;;
    versitygw) start_versitygw "$scheme" ;;
  esac
  export STELLARR_TEST_S3_KIND=$kind
  export STELLARR_TEST_S3_ENDPOINT=$endpoint
  export STELLARR_TEST_S3_ACCESS_KEY=$access
  export STELLARR_TEST_S3_SECRET_KEY=$secret
  if [ -n "$cacert" ]; then export STELLARR_TEST_S3_CA=$cacert; else unset STELLARR_TEST_S3_CA || true; fi
  if [ -n "${STELLARR_TEST_TRANSCRIPTS:-}" ]; then mkdir -p "$STELLARR_TEST_TRANSCRIPTS"; fi
  cd "$api"
  status=0

  # The questions first: they find out whether the conditional create is
  # honoured, and the contract is run knowing.
  out=$work/points-$kind-$scheme.txt
  "$cargo" test -p stellarr-storage --test servers -- --ignored --nocapture what_the_real_server \
    | tee "$out" || status=$?
  if grep -q '^CONDITIONAL=yes' "$out"; then
    export STELLARR_TEST_S3_CONDITIONAL=yes
  else
    export STELLARR_TEST_S3_CONDITIONAL=no
  fi
  echo "servers.sh: $kind over $scheme: conditional create honoured: $STELLARR_TEST_S3_CONDITIONAL"
  "$cargo" test -p stellarr-storage --test servers -- --ignored --nocapture --test-threads=2 \
    --skip what_the_real_server || status=$?
  if [ "$scheme" = http ]; then
    "$cargo" test -p stellarr-storage --test large -- --ignored --nocapture \
      the_bucket_store_streams_a_large_object_through_a_real_store || status=$?
  fi
  stop
  return "$status"
}

overall=0
for kind in garage versitygw; do
  case $kinds in all|"$kind") ;; *) continue ;; esac
  for scheme in http https; do
    case $schemes in both|"$scheme") ;; *) continue ;; esac
    if [ "$kind" = garage ] && [ "$scheme" = https ]; then
      echo "servers.sh: garage has no TLS of its own; the https run is skipped"
      continue
    fi
    have_image "$( [ "$kind" = garage ] && echo "$garage_image" || echo "$versity_image" )" \
      || { echo "servers.sh: the image of $kind is not here and cannot be pulled" >&2; overall=3; continue; }
    run "$kind" "$scheme" || overall=$?
  done
done
exit "$overall"
