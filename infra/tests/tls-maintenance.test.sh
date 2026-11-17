#!/usr/bin/env bash
# Run with Bash and OpenSSL 3 or newer; no running container is needed.
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
maintenance="${1:-$repo_root/infra/tls-maintenance.sh}"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/stellarr-tls-test.XXXXXX")"
trap 'rm -r -- "$scratch"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

fingerprint() {
  openssl x509 -in "$1/tls/fullchain.pem" -noout -fingerprint -sha256
}

prepare() {
  if ! STELLARR_DATA_ROOT="$1" PUBLIC_HOST=localhost bash "$maintenance" prepare > "$scratch/prepare.log" 2>&1; then
    cat "$scratch/prepare.log" >&2
    fail "TLS preparation failed."
  fi
}

# Explicit historical dates avoid clock changes, waiting and fake openssl
# output. With an issuer, the leaf belongs to that CA rather than to itself.
expired_pair() {
  local root="$1" issuer="${2:-}" tls="$1/tls"
  local signer
  mkdir -p "$tls" "$root/ca/newcerts"
  : > "$root/ca/index"
  printf '1000\n' > "$root/ca/serial"
  cat > "$root/ca/config" <<EOF
[ca]
default_ca=local
[local]
database=$root/ca/index
serial=$root/ca/serial
new_certs_dir=$root/ca/newcerts
default_md=sha256
policy=names
[names]
commonName=supplied
[extensions]
subjectAltName=DNS:localhost
basicConstraints=critical,CA:TRUE
EOF
  openssl req -new -newkey rsa:2048 -nodes -keyout "$tls/privkey.pem" \
    -out "$root/request.pem" -subj /CN=localhost >/dev/null 2>&1
  if [[ -n "$issuer" ]]; then
    signer=(-cert "$issuer/cert.pem" -keyfile "$issuer/key.pem")
  else
    signer=(-selfsign -keyfile "$tls/privkey.pem")
  fi
  openssl ca -config "$root/ca/config" "${signer[@]}" -in "$root/request.pem" \
    -startdate 20000101000000Z -enddate 20010101000000Z \
    -out "$tls/fullchain.pem" -extensions extensions -batch -notext >/dev/null 2>&1
  if openssl x509 -in "$tls/fullchain.pem" -noout -checkend 0 >/dev/null 2>&1; then
    fail "The expired-certificate fixture is still valid."
  fi
}

for marker in present absent; do
  root="$scratch/self-signed-$marker"
  expired_pair "$root"
  [[ "$marker" != present ]] || touch "$root/tls/.stellarr-self-signed"
  before="$(fingerprint "$root")"
  prepare "$root"
  [[ "$(fingerprint "$root")" != "$before" ]] || fail "Expired self-signed certificate was kept ($marker marker)."
  [[ -f "$root/tls/.stellarr-self-signed" ]] || fail "The renewed certificate was not marked as managed."
  openssl x509 -in "$root/tls/fullchain.pem" -noout -checkend 2592000 >/dev/null 2>&1 \
    || fail "The replacement certificate expires within 30 days."
  openssl verify -check_ss_sig -CAfile "$root/tls/fullchain.pem" \
    -verify_hostname localhost "$root/tls/fullchain.pem" >/dev/null 2>&1 \
    || fail "The replacement certificate is not valid for localhost."
  before="$(fingerprint "$root")"
  prepare "$root"
  [[ "$(fingerprint "$root")" == "$before" ]] || fail "A valid managed certificate was renewed unnecessarily."
done

# Preserve both an ordinary CA-signed certificate and one with the same
# subject and issuer name (self-issued, but not signed with the leaf's key).
for ca_name in 'External CA' localhost; do
  case_root="$scratch/external-${ca_name// /-}"
  issuer="$case_root/issuer"
  root="$case_root/instance"
  mkdir -p "$issuer"
  openssl req -x509 -newkey rsa:2048 -nodes -days 365 \
    -keyout "$issuer/key.pem" -out "$issuer/cert.pem" -subj "/CN=$ca_name" \
    -addext 'basicConstraints=critical,CA:TRUE' >/dev/null 2>&1
  expired_pair "$root" "$issuer"
  touch "$root/tls/.stellarr-self-signed"
  before="$(fingerprint "$root")"
  key_before="$(openssl pkey -in "$root/tls/privkey.pem" -pubout 2>/dev/null)"
  prepare "$root"
  [[ "$(fingerprint "$root")" == "$before" ]] || fail "An administrator's expired certificate was replaced ($ca_name)."
  [[ "$(openssl pkey -in "$root/tls/privkey.pem" -pubout 2>/dev/null)" == "$key_before" ]] \
    || fail "An administrator's private key was replaced ($ca_name)."
  [[ ! -e "$root/tls/.stellarr-self-signed" ]] || fail "A stale generated marker was retained ($ca_name)."
done

echo 'PASS: expired managed certificates renew; valid and external certificates are preserved.'
