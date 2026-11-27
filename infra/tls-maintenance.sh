#!/usr/bin/env bash
# Keeps the TLS pair in $STELLARR_DATA_ROOT/tls usable.
#
#   prepare  run once by the entrypoint before the services start
#   watch    run by stellarr-init; re-checks the pair periodically, and when a
#            renewal changed the certificate, has it reload nginx and restart
#            TURN
#
# Only a self-signed certificate is managed here. A pair that an administrator
# supplied is never overwritten: it can only produce a warning or an error.
set -Eeuo pipefail

settings=/usr/local/lib/stellarr/settings.sh
if [[ ! -f "$settings" ]]; then
  settings="$(dirname "${BASH_SOURCE[0]}")/settings.sh"
fi
# shellcheck source-path=SCRIPTDIR source=settings.sh
source "$settings"

mode="${1:-prepare}"
data_root="${STELLARR_DATA_ROOT:-/data/stellarr}"
# The name people reach the instance at, as the entrypoint exports it.
public_host="$(or_default "${STELLARR_DOMAIN:-}" localhost)"
require_public_host "$public_host" || exit 1
tls_root="$data_root/tls"
fullchain="$tls_root/fullchain.pem"
private_key="$tls_root/privkey.pem"
# Present while the pair on disk is one this script generated.
generated_marker="$tls_root/.stellarr-self-signed"
renew_before_seconds="${STELLARR_TLS_RENEW_BEFORE_SECONDS:-2592000}"
check_interval_seconds="${STELLARR_TLS_RENEW_CHECK_SECONDS:-43200}"
foreign_warning_seconds=604800

runtime_tls=/run/stellarr-tls
runtime_turn=/run/stellarr-turn
container_mode=false
[[ "$0" != /usr/local/bin/stellarr-tls-maintenance ]] || container_mode=true
external_tls=false
if [[ "$container_mode" == true && ! -L "$tls_root" ]] && mountpoint -q -- "$tls_root"; then
  external_tls=true
fi

as_tls_owner() {
  if [[ "$container_mode" == true && "$external_tls" == false && "$EUID" == 0 ]]; then
    setpriv --reuid=stellarr --regid=stellarr --clear-groups -- "$@"
  else
    "$@"
  fi
}

install_runtime_pair() {
  local chain key
  install -d -m 0700 "$runtime_tls" || return 1
  chain="$(mktemp "$runtime_tls/.fullchain.XXXXXX")" || return 1
  key="$(mktemp "$runtime_tls/.privkey.XXXXXX")" || return 1
  if ! as_tls_owner cat -- "$fullchain" > "$chain" ||
     ! as_tls_owner cat -- "$private_key" > "$key"; then
    rm -f -- "$chain" "$key"
    return 1
  fi
  mv -f "$chain" "$runtime_tls/fullchain.pem" || return 1
  mv -f "$key" "$runtime_tls/privkey.pem" || return 1
  if [[ "${STELLARR_TURN_ENABLED:-false}" == true ]]; then
    if (( EUID != 0 )); then
      install -m 0600 "$runtime_tls/fullchain.pem" "$runtime_tls/privkey.pem" "$runtime_turn" || return 1
      return 0
    fi
    install -d -m 0750 -o root -g turnserver "$runtime_turn" || return 1
    install -d -m 0700 -o turnserver -g turnserver "$runtime_turn/state" || return 1
    install -m 0600 -o turnserver -g turnserver \
      "$runtime_tls/fullchain.pem" "$runtime_tls/privkey.pem" "$runtime_turn" || return 1
  fi
}

host_is_ip_address() {
  is_ipv4_address "$public_host" || is_ipv6_address "$public_host"
}

expires_within() {
  ! openssl x509 -in "$fullchain" -noout -checkend "$1" >/dev/null 2>&1
}

certificate_is_self_signed() {
  local subject issuer
  subject="$(openssl x509 -in "$fullchain" -noout -subject -nameopt RFC2253 2>/dev/null || true)"
  issuer="$(openssl x509 -in "$fullchain" -noout -issuer -nameopt RFC2253 2>/dev/null || true)"
  # Ownership does not expire with the certificate. Check its own signature,
  # but leave validity dates to expires_within so an expired managed pair is
  # still renewed. Matching names alone would also accept a self-issued
  # certificate signed by somebody else's key.
  [[ -n "$subject" && "${subject#subject=}" == "${issuer#issuer=}" ]] &&
    openssl verify -no_check_time -check_ss_sig -CAfile "$fullchain" "$fullchain" >/dev/null 2>&1
}

certificate_matches_host() {
  if host_is_ip_address; then
    openssl verify -CAfile "$fullchain" -verify_ip "$public_host" "$fullchain" >/dev/null 2>&1
  else
    openssl verify -CAfile "$fullchain" -verify_hostname "$public_host" "$fullchain" >/dev/null 2>&1
  fi
}

certificate_matches_key() {
  local from_certificate from_key
  from_certificate="$(openssl x509 -in "$fullchain" -pubkey -noout 2>/dev/null || true)"
  from_key="$(openssl pkey -in "$private_key" -pubout 2>/dev/null || true)"
  [[ -n "$from_certificate" && "$from_certificate" == "$from_key" ]]
}

generate_self_signed_certificate() {
  local san new_key new_chain common_name="$public_host"
  if host_is_ip_address; then
    san="IP:$public_host"
  else
    san="DNS:$public_host"
  fi

  # The SAN carries the full name. OpenSSL limits CN to 64 characters.
  (( ${#common_name} <= 64 )) || common_name=Stellarr
  # Written next to the live files so that the final mv is an atomic rename.
  new_key="$(mktemp "$tls_root/.privkey.XXXXXX")"
  new_chain="$(mktemp "$tls_root/.fullchain.XXXXXX")"
  if ! openssl req -x509 -newkey rsa:3072 -sha256 -nodes -days 365 \
    -keyout "$new_key" -out "$new_chain" \
    -subj "/CN=$common_name" -addext "subjectAltName=$san"
  then
    rm -f "$new_key" "$new_chain"
    return 1
  fi

  chmod 600 "$new_key" || return 1
  mv -f "$new_key" "$private_key" || return 1
  mv -f "$new_chain" "$fullchain" || return 1
  touch "$generated_marker" || return 1
  echo "Generated a 365-day self-signed TLS certificate for $public_host. Mount a trusted certificate for remote use."
}

prepare_certificate() {
  if [[ "$external_tls" == true && (! -s "$fullchain" || ! -s "$private_key") ]]; then
    echo "The mounted TLS directory must contain fullchain.pem and privkey.pem." >&2
    return 1
  fi
  mkdir -p "$tls_root" || return 1
  if [[ ! -s "$fullchain" || ! -s "$private_key" ]]; then
    generate_self_signed_certificate
    return
  fi

  if ! openssl x509 -in "$fullchain" -noout >/dev/null 2>&1; then
    if [[ "$external_tls" == "false" && -f "$generated_marker" ]]; then
      generate_self_signed_certificate
      return
    fi
    echo "TLS certificate is unreadable. Replace $fullchain with a valid certificate." >&2
    return 1
  fi

  local managed=false
  if certificate_is_self_signed; then
    managed=true
  elif [[ "$external_tls" == "false" && -f "$generated_marker" ]]; then
    # An administrator replaced the generated pair with a trusted one. The
    # marker belongs to the old files and must not allow overwriting theirs.
    rm -f "$generated_marker"
  fi

  if ! certificate_matches_key; then
    if [[ "$external_tls" == "false" && "$managed" == "true" ]]; then
      generate_self_signed_certificate
      return
    fi
    echo "TLS certificate does not match its private key. Replace the mounted certificate pair." >&2
    return 1
  fi

  if [[ "$external_tls" == "false" && "$managed" == "true" ]]; then
    if expires_within "$renew_before_seconds" || ! certificate_matches_host; then
      generate_self_signed_certificate
    else
      # Also marks a self-signed pair from before the marker existed.
      touch "$generated_marker"
    fi
  elif expires_within "$foreign_warning_seconds"; then
    echo "Mounted TLS certificate expires within seven days; replace it before it expires." >&2
  fi
}

# Through the process manager, which signals nginx only once it is ready and
# logs both. If enabled, coturn restarts to pick up the new certificate.
reload_tls_consumers() {
  /usr/local/bin/stellarr-init reload nginx || echo "Could not reload nginx after TLS renewal." >&2
  if [[ "${STELLARR_TURN_ENABLED:-false}" == "true" ]]; then
    /usr/local/bin/stellarr-init restart turn || echo "Could not restart TURN after TLS renewal." >&2
  fi
}

fingerprint() {
  as_tls_owner openssl x509 -in "$fullchain" -noout -fingerprint -sha256 2>/dev/null || true
}

watch_certificate() {
  trap 'exit 0' TERM INT
  local before after
  while true; do
    # Sleeping in the background keeps the trap responsive.
    sleep "$check_interval_seconds" &
    wait $!
    as_tls_owner test -f "$generated_marker" || continue
    if [[ "$container_mode" == true ]]; then
      before="$(openssl x509 -in "$runtime_tls/fullchain.pem" -noout -fingerprint -sha256 2>/dev/null || true)"
    else
      before="$(fingerprint)"
    fi
    if ! as_tls_owner bash "$0" prepare-data; then
      echo "Managed TLS certificate check failed; retrying later." >&2
      continue
    fi
    after="$(fingerprint)"
    if [[ -n "$after" && "$after" != "$before" ]]; then
      if [[ "$container_mode" == true ]] && ! install_runtime_pair; then
        echo "Could not install the renewed TLS pair." >&2
        continue
      fi
      reload_tls_consumers
    fi
  done
}

case "$mode" in
  prepare)
    as_tls_owner bash "$0" prepare-data
    if [[ "$container_mode" == true ]]; then
      install_runtime_pair
    fi
    ;;
  prepare-data) prepare_certificate ;;
  watch) watch_certificate ;;
  *)
    echo "Usage: $0 prepare|watch" >&2
    exit 2
    ;;
esac
