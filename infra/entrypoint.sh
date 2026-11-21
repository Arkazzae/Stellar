#!/usr/bin/env bash
# Container entrypoint: prepares the data volume, exports the settings the
# services read, renders the coturn and nginx configuration from their
# templates, writes the web client's instance language file, and hands over to
# stellarr-init, the process manager that runs the services as PID 1.
#
# It reads only STELLARR_* variables, the names in .env. A setting it shares
# with the API is exported again under the same name with the value resolved
# here, so the API uses what coturn, nginx and the media server use. The API
# reads those names through its own table (server/Stellarr.Api/Operations/
# StellarrVariables.cs); the operator CLI, which docker exec starts without
# these exports, repeats the rules in server/Stellarr.Cli/ContainerSettings.cs.
set -Eeuo pipefail
# Without this a failing `openssl rand` inside $(...) would yield an empty secret.
shopt -s inherit_errexit

fail() {
  echo "$*" >&2
  exit 1
}

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

require_boolean() {
  case "$2" in
    true|false) ;;
    *) fail "$1 must be 'true' or 'false'." ;;
  esac
}

require_port() {
  if [[ ! "$2" =~ ^[0-9]{1,5}$ ]] || (( 10#$2 < 1 || 10#$2 > 65535 )); then
    fail "$1 must be an integer between 1 and 65535."
  fi
}

# Prints the secret stored under secrets/<name>, creating it on first use. The
# files are read back on every later start, so they must never be renamed.
read_or_create_secret() {
  local file="$data_root/secrets/$1" pending
  if [[ ! -s "$file" ]]; then
    pending="$(mktemp "$file.XXXXXX")" # mode 600
    openssl rand -hex 32 > "$pending"
    mv -f "$pending" "$file"
  fi
  tr -d '\r\n' < "$file"
}

# Prints the external IPv4 address coturn and the media server should
# advertise, or nothing.
resolve_public_ip() {
  local resolved
  case "$public_ip_setting" in
    '') ;;
    auto)
      resolved="$(getent ahostsv4 "$public_host" 2>/dev/null | awk '$2 == "STREAM" { print $1; exit }')" || true
      is_ipv4_address "$resolved" ||
        fail "STELLARR_PUBLIC_IP=auto could not resolve a public IPv4 address for STELLARR_DOMAIN=$public_host."
      echo "Resolved STELLARR_DOMAIN=$public_host to the external IPv4 $resolved for TURN and the media server." >&2
      echo "$resolved"
      ;;
    *)
      is_ipv4_address "$public_ip_setting" ||
        fail "STELLARR_PUBLIC_IP must be an IPv4 address or the value 'auto'."
      echo "$public_ip_setting"
      ;;
  esac
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

# Writes the instance's default language from STELLARR_LANGUAGE to the file
# nginx serves as /stellarr-instance.js, which the web client's document runs
# before its first paint. One image serves every instance, so the language
# cannot be part of the build. The value lands in a script, so only a language
# the client ships is written; anything else is written as null, and the
# client falls back to English. The bytes are those of instanceLanguageSource
# in web/src/stellarr/i18n/localeScript.ts, which web/documents.test.ts checks
# against this function.
write_instance_language() {
  local file="$1" language="${STELLARR_LANGUAGE:-}" value=null
  case "$language" in
    en|pl) value="\"$language\"" ;;
    '') ;;
    *) echo "STELLARR_LANGUAGE=$language is not a language the web client ships (en, pl); it starts in English for anyone who has not chosen one." >&2 ;;
  esac
  mkdir -p "$(dirname "$file")"
  printf 'window.stellarrInstanceLanguage=%s;\n' "$value" > "$file"
}

# The settings, each with the image's default. The host ports default to the
# container's own: Compose publishes HTTPS on another port and says so.
data_root="$(or_default "${STELLARR_DATA_ROOT:-}" /data/stellarr)"
# The name people reach the instance at. Without it, TLS and TURN use localhost
# and the instance has no public address.
domain="$(or_default "${STELLARR_DOMAIN:-}")"
public_host="${domain:-localhost}"
public_url_setting="$(or_default "${STELLARR_PUBLIC_URL:-}")"
public_ip_setting="$(or_default "${STELLARR_PUBLIC_IP:-}")"
https_port="$(or_default "${STELLARR_HTTPS_PORT:-}" 443)"
media_port="$(or_default "${STELLARR_MEDIA_PORT:-}" 49000)"
turn_tcp_enabled="$(or_default "${STELLARR_TURN_TCP_ENABLED:-}" false)"
turns_enabled="$(or_default "${STELLARR_TURNS_ENABLED:-}" false)"
language="$(or_default "${STELLARR_LANGUAGE:-}" en)"
media_advertise="$(or_default "${STELLARR_MEDIA_ADVERTISE:-}")"
media_log="$(or_default "${STELLARR_MEDIA_LOG:-}" info)"

require_port STELLARR_HTTPS_PORT "$https_port"
https_port=$((10#$https_port))
# The HTTPS port browsers use: the published one, or 443 behind a proxy.
public_port="$(or_default "${STELLARR_PUBLIC_PORT:-}" "$https_port")"
require_port STELLARR_PUBLIC_PORT "$public_port"
# A browser can only reach the port it is told, so the media server advertises
# the host port of 49000/udp and the API names it in the bootstrap.
require_port STELLARR_MEDIA_PORT "$media_port"
media_port=$((10#$media_port))
require_boolean STELLARR_TURN_TCP_ENABLED "$turn_tcp_enabled"
require_boolean STELLARR_TURNS_ENABLED "$turns_enabled"

public_ip="$(resolve_public_ip)"

# A backup staged with `stellarr backup restore` replaces the instance's data
# here, before any service starts and before the secrets below are read, so
# every service sees the restored instance. An archive the restore refuses
# leaves the data as it was. A restore that failed part-way and could not undo
# itself leaves a marker, and the instance does not start on that volume.
restore_dir="$data_root/restore"
[[ ! -e "$restore_dir/broken" ]] ||
  fail "A restore failed part-way and could not put the previous data back; see $restore_dir/last-restore.json. Repair the data volume, then remove $restore_dir/broken."
if [[ -f "$restore_dir/staged.zip" ]]; then
  echo "Restoring the backup staged in $restore_dir/staged.zip." >&2
  restore_status=0
  /usr/local/bin/stellarr --data-root "$data_root" backup apply --yes || restore_status=$?
  case "$restore_status" in
    0) ;;
    4) echo "The staged backup was not restored, and the instance starts with its previous data; see $restore_dir/last-restore.json." >&2 ;;
    *) fail "Restoring the staged backup failed; see $restore_dir/last-restore.json." ;;
  esac
fi

mkdir -p "$data_root"/{db,files,keys,logs,secrets,tls} /run/nginx

# The secrets the API shares with coturn and the media server are the ones in
# the data volume, never a setting: the exports below replace any value the
# environment brings under the names the services read.
turn_secret="$(read_or_create_secret turn-shared-secret)"
media_control_secret="$(read_or_create_secret media-control-secret)"

# What the API, the TLS script and the media server read, as resolved above.
export STELLARR_DATA_ROOT="$data_root"
export STELLARR_DOMAIN="$domain"
export STELLARR_LANGUAGE="$language"
export STELLARR_MEDIA_PORT="$media_port"
export STELLARR_TURN_TCP_ENABLED="$turn_tcp_enabled"
export STELLARR_TURNS_ENABLED="$turns_enabled"
# Values with no STELLARR_* name, which only the entrypoint derives.
export ConnectionStrings__Database="Data Source=$data_root/db/stellarr.db;Pooling=True"
export Rtc__MediaControlUrl="http://127.0.0.1:5200"
export Rtc__MediaControlSecret="$media_control_secret"
export Rtc__TurnSharedSecret="$turn_secret"

# The address in mail links and the desktop client's canonical origin: the
# domain on the HTTPS port browsers use. STELLARR_PUBLIC_URL, when set,
# overrides it. Without a domain or a URL there is no public address: reset
# links stay off and desktop clients keep the address they connected to.
if [[ -n "$domain" || -n "$public_url_setting" ]]; then
  public_url="$(public_url_for "$public_host" "$public_port" "$public_url_setting")"
  export Operations__PublicUrl="$public_url"
  export Desktop__CanonicalOrigin="$public_url"
  echo "The public address of this instance is $public_url." >&2
else
  echo "STELLARR_DOMAIN is not set, so the instance has no public address: reset links stay off and desktop clients keep the address they connected to." >&2
fi

# Stellarr Media carries every call. Its candidates are the public address,
# when one is known, and the container's own address, which is where coturn
# relays a browser that needs TURN. STELLARR_MEDIA_ADVERTISE, when set,
# replaces both and is passed through.
export STELLARR_MEDIA_BIND="0.0.0.0:49000"
export STELLARR_MEDIA_PUBLIC_IP="$public_ip"
export STELLARR_MEDIA_PUBLIC_PORT="$media_port"
export STELLARR_MEDIA_CONTROL_BIND="127.0.0.1:5200"
export STELLARR_MEDIA_CONTROL_SECRET="$media_control_secret"
export STELLARR_MEDIA_LOG="$media_log"
if [[ -n "$media_advertise" ]]; then
  export STELLARR_MEDIA_ADVERTISE="$media_advertise"
else
  unset STELLARR_MEDIA_ADVERTISE
fi

if [[ -z "$public_ip" && -z "$media_advertise" ]]; then
  echo "STELLARR_PUBLIC_IP is empty, so the media server advertises only this container's own address." \
    "Voice from a browser outside this host reaches it only through TURN, and gets no connection when TURN is blocked too." \
    "Set STELLARR_PUBLIC_IP to the IPv4 address browsers reach this host at, or to 'auto'." >&2
fi

# The ASP.NET base image sets one of these, and it would conflict with
# ASPNETCORE_URLS.
unset ASPNETCORE_HTTP_PORTS ASPNETCORE_HTTPS_PORTS
export ASPNETCORE_URLS="http://127.0.0.1:5000"
export ASPNETCORE_ENVIRONMENT="${ASPNETCORE_ENVIRONMENT:-Production}"

/usr/local/bin/stellarr-tls-maintenance prepare

# A disabled transport is closed in coturn, not merely left unadvertised.
turn_tcp_line="# TURN/TCP client listener enabled"
turn_tls_line="# TURN/TLS client listener enabled"
turn_dtls_line="# TURN/DTLS client listener enabled"
external_ip_line="# external-ip is not set; set STELLARR_PUBLIC_IP when the host is behind NAT"
[[ "$turn_tcp_enabled" == "true" ]] || turn_tcp_line="no-tcp"
if [[ "$turns_enabled" != "true" ]]; then
  turn_tls_line="no-tls"
  turn_dtls_line="no-dtls"
fi
if [[ -n "$public_ip" ]]; then
  external_ip_line="external-ip=$public_ip"
fi

sed \
  -e "s|__REALM__|$public_host|g" \
  -e "s|__STATIC_AUTH_SECRET__|$turn_secret|g" \
  -e "s|__EXTERNAL_IP_LINE__|$external_ip_line|g" \
  -e "s|__TURN_TCP_LINE__|$turn_tcp_line|g" \
  -e "s|__TURN_TLS_LINE__|$turn_tls_line|g" \
  -e "s|__TURN_DTLS_LINE__|$turn_dtls_line|g" \
  /etc/turnserver.conf.template > /etc/turnserver.conf

write_instance_language /run/stellarr/stellarr-instance.js

public_https_suffix=""
if (( https_port != 443 )); then
  public_https_suffix=":$https_port"
fi
sed -e "s|__PUBLIC_HTTPS_SUFFIX__|$public_https_suffix|g" \
  /etc/nginx/nginx.conf.template > /etc/nginx/nginx.conf

# The services and how each is started, checked and stopped are compiled into
# stellarr-init (init/src/services.rs). They inherit the environment exported
# above.
exec /usr/local/bin/stellarr-init run
