# shellcheck shell=bash
# shellcheck disable=SC2034 # the scripts that source this file read what read_settings sets
# The rules the container reads its settings by: each STELLARR_* setting with
# the image's default, a blank value counting as not set, the values it
# refuses, and the instance's public address derived from them. Installed at
# /usr/local/lib/stellarr/settings.sh and sourced by the entrypoint and by the
# operator CLI's wrapper, so a CLI that docker exec starts derives the address
# the services were given.
#
# It only defines functions. read_settings sets shell variables and exports
# nothing; it reads no file and no secret, resolves no name and renders no
# template. What to do with a value it refuses is up to the script that
# sources it.

# Prints $1, or $2 when $1 is empty or only whitespace: a blank setting counts
# as not set, as it does in the API.
or_default() {
  if [[ -z "${1//[[:space:]]/}" ]]; then
    printf '%s' "${2:-}"
  else
    printf '%s' "$1"
  fi
}

is_ipv4_address() {
  local octets octet
  [[ "$1" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ ]] || return 1
  IFS=. read -r -a octets <<< "$1"
  for octet in "${octets[@]}"; do
    (( 10#$octet <= 255 )) || return 1
  done
}

# Each check prints why it refuses the value of $1 and returns 1.
require_boolean() {
  case "$2" in
    true|false) ;;
    *)
      echo "$1 must be 'true' or 'false'." >&2
      return 1
      ;;
  esac
}

require_port() {
  if [[ ! "$2" =~ ^[0-9]{1,5}$ ]] || (( 10#$2 < 1 || 10#$2 > 65535 )); then
    echo "$1 must be an integer between 1 and 65535." >&2
    return 1
  fi
}

# Prints the instance's public URL: the third argument as given when it is set,
# otherwise https:// with the host and, unless it is 443, the port.
public_url_for() {
  local host="$1" port="$2" explicit="$3"
  if [[ -n "$explicit" ]]; then
    printf '%s' "${explicit%/}"
    return
  fi
  [[ "$host" == *:* ]] && host="[$host]"
  if (( 10#$port == 443 )); then
    printf 'https://%s' "$host"
  else
    printf 'https://%s:%s' "$host" "$((10#$port))"
  fi
}

# Reads the settings into the variables below. On a value it cannot use it
# prints why and returns 1 at once, so the variables after that one are not
# all set. The host ports default to the container's own: Compose publishes
# HTTPS on another port and says so.
read_settings() {
  public_url=''
  data_root="$(or_default "${STELLARR_DATA_ROOT:-}" /data/stellarr)"
  # The name people reach the instance at. Without it, TLS and TURN use
  # localhost and the instance has no public address.
  domain="$(or_default "${STELLARR_DOMAIN:-}")"
  public_host="${domain:-localhost}"
  public_url_setting="$(or_default "${STELLARR_PUBLIC_URL:-}")"
  public_ip_setting="$(or_default "${STELLARR_PUBLIC_IP:-}")"
  https_port="$(or_default "${STELLARR_HTTPS_PORT:-}" 443)"
  media_port="$(or_default "${STELLARR_MEDIA_PORT:-}" 49000)"
  turn_enabled="$(or_default "${STELLARR_TURN_ENABLED:-}" false)"
  turn_tcp_enabled="$(or_default "${STELLARR_TURN_TCP_ENABLED:-}" false)"
  turns_enabled="$(or_default "${STELLARR_TURNS_ENABLED:-}" false)"
  language="$(or_default "${STELLARR_LANGUAGE:-}" en)"
  media_advertise="$(or_default "${STELLARR_MEDIA_ADVERTISE:-}")"
  media_log="$(or_default "${STELLARR_MEDIA_LOG:-}" info)"

  require_port STELLARR_HTTPS_PORT "$https_port" || return 1
  https_port=$((10#$https_port))
  # The HTTPS port browsers use: the published one, or 443 behind a proxy.
  public_port="$(or_default "${STELLARR_PUBLIC_PORT:-}" "$https_port")"
  require_port STELLARR_PUBLIC_PORT "$public_port" || return 1
  # A browser can only reach the port it is told, so the media server
  # advertises the host port of 49000 and the API names it in the
  # bootstrap.
  require_port STELLARR_MEDIA_PORT "$media_port" || return 1
  media_port=$((10#$media_port))
  require_boolean STELLARR_TURN_ENABLED "$turn_enabled" || return 1
  require_boolean STELLARR_TURN_TCP_ENABLED "$turn_tcp_enabled" || return 1
  require_boolean STELLARR_TURNS_ENABLED "$turns_enabled" || return 1
  # auto is resolved by the entrypoint, which looks the domain up.
  if [[ -n "$public_ip_setting" && "$public_ip_setting" != auto ]] && ! is_ipv4_address "$public_ip_setting"; then
    echo "STELLARR_PUBLIC_IP must be an IPv4 address or the value 'auto'." >&2
    return 1
  fi

  # The address in mail links and the desktop client's canonical origin: the
  # domain on the HTTPS port browsers use. STELLARR_PUBLIC_URL, when set,
  # overrides it. Without a domain or a URL there is none, and public_url
  # stays empty.
  if [[ -n "$domain" || -n "$public_url_setting" ]]; then
    public_url="$(public_url_for "$public_host" "$public_port" "$public_url_setting")"
  fi
}
