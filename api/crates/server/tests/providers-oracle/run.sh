#!/bin/sh
# Runs every flow of the identity provider group against the first server (the
# oracle) and this one, both pointed at one fake provider (fake-provider.mjs),
# and diffs the two conversations.
#
#   PROVIDERS_ORACLE_HOST=<this machine's LAN address> sh run.sh [flows|limits]
#
# The first server runs in a container and this one on the host, and both must
# reach the provider by the same issuer address. Inside the container this
# machine is host.docker.internal, but on the host that name does not resolve,
# so the issuer is this machine's address on the network (for example
# `ipconfig getifaddr en0`), which has to be given. The certificate the script
# makes names that address, and both servers are told to trust its authority
# through SSL_CERT_FILE.
#
# Ports: the provider 5281 (control 5282), this server 5283, the oracle 5228,
# and the oracle's container is named `providers`. Somebody else's run on the
# same machine takes the same ones, so each can be moved:
# PROVIDER_PORT, PROVIDER_CONTROL, RUST_PORT, ORACLE_PORT and ORACLE_NAME.
#
# After the conversations of `flows` both servers are stopped and the rows each
# left (linked and waiting identities, sessions and what they count as, the
# challenges, the security events and the alerts queued for them) are compared
# table by table with ids, moments and secrets replaced (tables.py).
# Needs: docker, the `oracle` script beside `acargo`, node and openssl, and a
# built stellarr-api (STELLARR_API_BIN, else the one of the `providers` build
# directory). KEEP=1 keeps the work folder; it is removed otherwise, and the
# servers are stopped on every way out.
set -eu

: "${PROVIDERS_ORACLE_HOST:?set PROVIDERS_ORACLE_HOST to this machine's LAN address (the oracle cannot reach it by name, nor this machine the oracle's)}"
host=$PROVIDERS_ORACLE_HOST
pass=${1:-flows}
here=$(cd "$(dirname "$0")" && pwd)
root=${API_PORT_ROOT:-/private/tmp/claude-501/-Users-arkazz-Documents-GitHub-Stellarr-newest/0a891a01-3b9d-48a0-908d-e6ab7a67d243/scratchpad/api-port}
bin=${STELLARR_API_BIN:-$root/targets/providers/debug/stellarr-api}
work=$(mktemp -d "${TMPDIR:-/tmp}/providers-oracle.XXXXXX")
pids=""
provider_port=${PROVIDER_PORT:-5281}
control_port=${PROVIDER_CONTROL:-5282}
rust_port=${RUST_PORT:-5283}
oracle_port=${ORACLE_PORT:-5228}
oracle_name=${ORACLE_NAME:-providers}
rust_pid=

cleanup() {
  for pid in $pids; do kill "$pid" 2>/dev/null || true; done
  "$root/bin/oracle" stop "$oracle_name" >/dev/null 2>&1 || true
  if [ -z "${KEEP:-}" ]; then rm -rf "$work"; else echo "kept $work" >&2; fi
}
trap cleanup EXIT INT TERM

cd "$work"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout ca.key -out ca.pem -days 2 \
  -subj "/CN=providers oracle CA" -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign" 2>/dev/null
openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout leaf.key -out leaf.csr -subj "/CN=fake" 2>/dev/null
printf 'subjectAltName=IP:%s,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature\nextendedKeyUsage=serverAuth\n' "$host" > ext.cnf
openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out leaf.pem -days 2 -extfile ext.cnf 2>/dev/null

node "$here/fake-provider.mjs" "$host" "$provider_port" "$control_port" leaf.pem leaf.key &
pids="$pids $!"

# The settings both servers get: the original provider at /p0 and p1..p45.
{
  echo "Oidc__Enabled=true"
  echo "Oidc__Authority=https://$host:$provider_port/p0"
  echo "Oidc__ClientId=stellarr-tests"
  echo "Oidc__ClientSecret=test-client-secret"
  echo "Oidc__DisplayName=Test IdP"
  i=1
  while [ $i -le 45 ]; do
    echo "Oidc__Providers__p${i}__Authority=https://$host:$provider_port/p${i}"
    echo "Oidc__Providers__p${i}__ClientId=stellarr-tests"
    echo "Oidc__Providers__p${i}__ClientSecret=test-client-secret"
    echo "Oidc__Providers__p${i}__DisplayName=Provider ${i}"
    i=$((i + 1))
  done
  if [ "$pass" = flows ]; then
    echo "RateLimiting__Login__PermitLimit=1000"
    echo "RateLimiting__AuthWrite__PermitLimit=1000"
    echo "RateLimiting__SecurityWrite__PermitLimit=1000"
    echo "RateLimiting__LoginPoll__PermitLimit=1000"
  fi
} > settings.env

cp settings.env oracle.env
echo "SSL_CERT_FILE=/tmp/ca.pem" >> oracle.env
ORACLE_ENV_FILE="$work/oracle.env" "$root/bin/oracle" start "$oracle_name" "$oracle_port" Production >/dev/null
docker cp ca.pem "stellarr-port-oracle-$oracle_name:/tmp/ca.pem"

mkdir -p data
(
  while IFS= read -r line; do export "$line"; done < settings.env
  export SSL_CERT_FILE="$work/ca.pem" ASPNETCORE_ENVIRONMENT=Production ASPNETCORE_URLS=http://127.0.0.1:$rust_port \
    STELLARR_DATA_ROOT="$work/data" "ConnectionStrings__Database=Data Source=$work/data/stellarr.db;Pooling=True" \
    Rtc__TurnSharedSecret=oracle-turn-secret-oracle-turn-secret-0123 Operations__PublicUrl=http://127.0.0.1:$rust_port
  exec "$bin"
) > rust.log 2>&1 &
rust_pid=$!
pids="$pids $rust_pid"

code_of() { grep -Eo '[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{4}' | head -1; }
i=0; rust_code=""
while [ $i -lt 100 ] && [ -z "$rust_code" ]; do rust_code=$(code_of < rust.log || true); i=$((i + 1)); sleep 0.3; done
oracle_code=$("$root/bin/oracle" logs "$oracle_name" 2>&1 | code_of || true)
[ -n "$rust_code" ] && [ -n "$oracle_code" ] || { echo "no setup code: rust=[$rust_code] oracle=[$oracle_code]" >&2; exit 1; }

node "$here/driver.mjs" first "http://127.0.0.1:$oracle_port" "$oracle_code" "$control_port" "$pass" "$host" > first.txt
node "$here/driver.mjs" this "http://127.0.0.1:$rust_port" "$rust_code" "$control_port" "$pass" "$host" > this.txt
if [ -n "${OUT:-}" ]; then cp first.txt "$OUT.first"; cp this.txt "$OUT.this"; fi
if diff first.txt this.txt > diff.txt; then echo "identical ($(wc -l < first.txt) lines)"; else
  cat diff.txt; echo "--- $(grep -c '^<' diff.txt) lines differ of $(wc -l < first.txt)"
  if [ -n "${OUT:-}" ]; then cp diff.txt "$OUT.diff"; fi
fi

status=0
if [ "$pass" = flows ]; then
  # Both servers are stopped first, so that each file holds everything it wrote.
  kill "$rust_pid" 2>/dev/null || true
  wait "$rust_pid" 2>/dev/null || true
  docker stop -t 20 "stellarr-port-oracle-$oracle_name" >/dev/null
  docker cp "stellarr-port-oracle-$oracle_name:/tmp/data/stellarr.db" oracle.db >/dev/null
  echo "== the rows"
  python3 -I "$here/tables.py" oracle.db data/stellarr.db || status=1
fi
exit $status
