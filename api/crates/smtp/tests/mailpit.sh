#!/bin/sh
# Runs tests/mailpit.rs against real Mailpit servers.
#
#   api/crates/smtp/tests/mailpit.sh
#
# It starts three containers from the Mailpit image that is already on the
# machine, on free ports of 127.0.0.1:
#   stellarr-port-smtp-mailpit           plain SMTP, no login
#   stellarr-port-smtp-mailpit-starttls  STARTTLS required, login required
#   stellarr-port-smtp-mailpit-tls       TLS from the first byte, login required
# The certificate is the one of tests/fixtures (a throwaway CA and a leaf for
# localhost), the login is jo with the password the test knows. All of it is
# removed when the script ends. Set ACARGO to the cargo wrapper to use.
set -eu

image=axllent/mailpit:v1.34.0
here=$(cd "$(dirname "$0")" && pwd)
api=$(cd "$here/../../.." && pwd)
cargo=${ACARGO:-cargo}
work=$(mktemp -d)

cleanup() {
  docker rm -f stellarr-port-smtp-mailpit stellarr-port-smtp-mailpit-starttls stellarr-port-smtp-mailpit-tls >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT INT TERM

openssl x509 -inform DER -in "$here/fixtures/server.der" -out "$work/cert.pem"
openssl pkcs8 -inform DER -nocrypt -in "$here/fixtures/server-key.der" -out "$work/key.pem"
printf 'jo:s3cr3t-pässword\n' > "$work/auth.txt"
chmod 755 "$work"
chmod 644 "$work"/*

free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}

start() { # name smtp-port http-port options...
  name=$1 smtp=$2 http=$3
  shift 3
  docker run -d --name "$name" -p "127.0.0.1:$smtp:1025" -p "127.0.0.1:$http:8025" \
    -v "$work:/certs:ro" "$image" "$@" >/dev/null
  i=0
  until python3 -c "import urllib.request,sys; urllib.request.urlopen('http://127.0.0.1:$http/api/v1/info', timeout=1)" 2>/dev/null; do
    i=$((i + 1))
    [ "$i" -lt 50 ] || { echo "$name did not start" >&2; docker logs "$name" >&2; exit 1; }
    sleep 0.2
  done
}

p1=$(free_port); h1=$(free_port); p2=$(free_port); h2=$(free_port); p3=$(free_port); h3=$(free_port)
start stellarr-port-smtp-mailpit "$p1" "$h1"
start stellarr-port-smtp-mailpit-starttls "$p2" "$h2" \
  --smtp-tls-cert /certs/cert.pem --smtp-tls-key /certs/key.pem \
  --smtp-require-starttls --smtp-auth-file /certs/auth.txt
start stellarr-port-smtp-mailpit-tls "$p3" "$h3" \
  --smtp-tls-cert /certs/cert.pem --smtp-tls-key /certs/key.pem \
  --smtp-require-tls --smtp-auth-file /certs/auth.txt

# What the server offers, as Python's own client sees it.
python3 - "$p2" <<'PY'
import smtplib, sys
c = smtplib.SMTP("127.0.0.1", int(sys.argv[1]))
c.ehlo()
c.starttls()
c.ehlo()
print("mailpit offers after STARTTLS:", sorted(c.esmtp_features.items()))
c.quit()
PY

cd "$api"
STELLARR_TEST_MAILPIT_PLAIN="$p1,$h1" \
STELLARR_TEST_MAILPIT_STARTTLS="$p2,$h2" \
STELLARR_TEST_MAILPIT_TLS="$p3,$h3" \
  "$cargo" test -p stellarr-smtp --test mailpit -- --ignored --nocapture --test-threads=1
