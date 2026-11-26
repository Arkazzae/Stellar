#!/usr/bin/env bash
# Container entrypoint: prepares the data volume, exports the settings the
# services read, renders the nginx and optional coturn configuration from their
# templates, writes the web client's instance language file, and hands over to
# stellarr-init, the process manager that runs the services as PID 1.
#
# It reads only STELLARR_* variables, the names in .env. A setting it shares
# with the API is exported again under the same name with the value resolved
# here, so the API uses what coturn, nginx and the media server use. The API
# reads those names through its own table (api/crates/server/src/settings/
# variables.rs). How each setting is read, defaulted and refused, and
# the public address that follows from them, is in settings.sh, which the
# operator CLI's wrapper sources too: docker exec starts the CLI without the
# exports below.
set -Eeuo pipefail
# Without this a failing `openssl rand` inside $(...) would yield an empty secret.
shopt -s inherit_errexit

# shellcheck source-path=SCRIPTDIR source=settings.sh
source /usr/local/lib/stellarr/settings.sh

fail() {
  echo "$*" >&2
  exit 1
}

write_trusted_proxies() {
  local entry ip prefix width
  local -a proxies=()
  read -r -a proxies <<< "$(printf '%s' "${STELLARR_TRUSTED_PROXIES:-}" | tr ',[:space:]' ' ')"
  for entry in "${proxies[@]}"; do
    ip="${entry%%/*}"
    if is_ipv4_address "$ip"; then
      width=32
    elif is_ipv6_address "$ip"; then
      width=128
    else
      fail "STELLARR_TRUSTED_PROXIES must list IP addresses or CIDR ranges."
    fi
    if [[ "$entry" == */* ]]; then
      prefix="${entry#*/}"
      if [[ ! "$prefix" =~ ^[0-9]{1,3}$ ]] || (( 10#$prefix < 1 || 10#$prefix > width )); then
        fail "STELLARR_TRUSTED_PROXIES contains an invalid CIDR prefix."
      fi
    fi
    printf '%s 1;\n' "$entry"
  done
}

# Prints the external IPv4 address coturn and the media server should
# advertise, or nothing. read_settings has checked the setting's form.
resolve_public_ip() {
  local resolved
  case "$public_ip_setting" in
    '') ;;
    auto)
      resolved="$(getent ahostsv4 "$public_host" 2>/dev/null | awk '$2 == "STREAM" { print $1; exit }')" || true
      is_ipv4_address "$resolved" ||
        fail "STELLARR_PUBLIC_IP=auto could not resolve a public IPv4 address for STELLARR_DOMAIN=$public_host."
      echo "Resolved STELLARR_DOMAIN=$public_host to the external IPv4 $resolved for media." >&2
      echo "$resolved"
      ;;
    *) echo "$public_ip_setting" ;;
  esac
}

# Writes the instance's default language from STELLARR_LANGUAGE to the file
# nginx serves as /stellarr-instance.js, which the web client's document runs
# before its first paint. One image serves every instance, so the language
# cannot be part of the build. Only serialize a plain language tag here.
# The pre-paint script resolves it against the catalogs shipped in that build.
write_instance_language() {
  local file="$1" language="${STELLARR_LANGUAGE:-}" value=null
  local LC_ALL=C
  if [[ "$language" =~ ^[a-z]{2,3}(-[A-Za-z0-9]{1,8})*$ ]]; then
    value="\"$language\""
  fi
  mkdir -p "$(dirname "$file")"
  printf 'window.stellarrInstanceLanguage=%s;\n' "$value" > "$file"
}

# The settings, each with the image's default. A value that cannot be used
# stops the container here, before anything is written.
read_settings || exit 1
activity_domain="$(or_default "${STELLARR_ACTIVITY_DOMAIN:-}")"
activity_domain="${activity_domain,,}"
activity_https_suffix=""
if [[ -n "$activity_domain" ]]; then
  (( ${#activity_domain} <= 253 )) &&
    [[ "$activity_domain" =~ ^([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]] ||
    fail "STELLARR_ACTIVITY_DOMAIN must be a DNS suffix."
  activity_instance="${domain,,}"
  activity_instance="${activity_instance%.}"
  [[ -n "$activity_instance" && "$activity_instance" != "$activity_domain" && "$activity_instance" != *."$activity_domain" ]] ||
    fail "Set STELLARR_DOMAIN to an instance name outside STELLARR_ACTIVITY_DOMAIN."
  activity_origin="$(or_default "${STELLARR_ACTIVITY_ORIGIN:-}" "$public_url")"
  [[ "$activity_origin" =~ ^https://([A-Za-z0-9.-]+)(:([0-9]{1,5}))?/?$ ]] ||
    fail "Installed activities require an HTTPS instance origin."
  activity_parent="${BASH_REMATCH[1],,}"
  activity_port="${BASH_REMATCH[3]:-443}"
  require_public_host "$activity_parent" || exit 1
  require_port STELLARR_ACTIVITY_ORIGIN "$activity_port" || exit 1
  [[ "${activity_parent%.}" != "$activity_domain" && "${activity_parent%.}" != *."$activity_domain" ]] ||
    fail "The activity's parent origin must be outside STELLARR_ACTIVITY_DOMAIN."
  if (( 10#$activity_port != 443 )); then
    activity_https_suffix=":$((10#$activity_port))"
  fi
  export STELLARR_ACTIVITY_DOMAIN="$activity_domain"
  export STELLARR_ACTIVITY_ORIGIN="https://$activity_parent$activity_https_suffix"
fi
trusted_proxy_rules="$(write_trusted_proxies)"
public_ip="$(resolve_public_ip)"
# A numeric domain can be used as the host candidate without DNS. Local
# installs need the published loopback address instead of the Docker IP.
media_public_ip="$public_ip"
if [[ -z "$media_public_ip" ]]; then
  if is_ipv4_address "$public_host"; then
    media_public_ip="$public_host"
  elif [[ "$public_host" == localhost ]]; then
    media_public_ip=127.0.0.1
  fi
fi

# The optional Compose stack selects a component explicitly. Its API shares
# loopback with nginx; only the authenticated media control crosses a bridge.
component="${1:-all}"
# Stellarr Media advertises the public and LAN addresses when configured,
# plus the container address that optional coturn uses to reach it.
# STELLARR_MEDIA_ADVERTISE overrides that list.
export STELLARR_MEDIA_BIND="0.0.0.0:49000"
export STELLARR_MEDIA_PUBLIC_IP="$media_public_ip"
export STELLARR_MEDIA_PUBLIC_PORT="$media_port"
export STELLARR_MEDIA_LAN_IP="$media_lan_ip"
export STELLARR_MEDIA_CONTROL_BIND="127.0.0.1:5200"
export STELLARR_MEDIA_LOG="$media_log"
if [[ -n "$media_advertise" ]]; then
  export STELLARR_MEDIA_ADVERTISE="$media_advertise"
else
  unset STELLARR_MEDIA_ADVERTISE
fi

if [[ -z "$media_public_ip" && -z "$media_lan_ip" && -z "$media_advertise" ]]; then
  echo "STELLARR_PUBLIC_IP is empty, so the media server advertises only this container's own address." \
    "Browsers outside the container network need a reachable media candidate." \
    "Set STELLARR_PUBLIC_IP to the IPv4 address browsers reach this host at, or to 'auto'." >&2
fi

# Each shared secret has a single writer and a dedicated volume. The API
# mounts it read-only after the producing service passes its health check.
split_secret() {
  local root="/run/credentials/$1" name="$2"
  if [[ "$component" == "split-$1" ]]; then
    install -d -m 0700 "$root/secrets"
  fi
  read_or_create_secret "$root" "$name"
}

if [[ "$component" == split-media ]]; then
  STELLARR_MEDIA_CONTROL_SECRET="$(split_secret media media-control-secret)"
  export STELLARR_MEDIA_CONTROL_SECRET STELLARR_MEDIA_CONTROL_BIND="0.0.0.0:5200"
  exec setpriv --reuid=nobody --regid=nogroup --clear-groups -- /opt/stellarr/media/stellarr-media
fi

run_as_api() {
  setpriv --reuid=stellarr --regid=stellarr --clear-groups -- "$@"
}

prepare_instance_data() {
  # Migrate existing volumes before any operation that touches instance data.
  # Physical traversal and non-dereferencing chown leave symlinks alone.
  mkdir -p "$data_root"
  local -a private_dirs=("$data_root"/{db,files,keys,logs,secrets})
  local -a mounted_tls=()
  if [[ ! -L "$data_root/tls" ]] && mountpoint -q -- "$data_root/tls"; then
    mounted_tls=(-path "$data_root/tls" -prune -o)
  else
    private_dirs+=("$data_root/tls")
  fi
  find -P "$data_root" -xdev "${mounted_tls[@]}" \( -type d -o -type f \) ! -user stellarr \
    -exec chown -h stellarr:stellarr -- {} +

  # A backup staged with `stellarr backup restore` replaces the instance's data
  # here, before any service starts and before the secrets below are read, so
  # every service sees the restored instance. An archive the restore refuses
  # leaves the data as it was. A restore that failed part-way and could not undo
  # itself leaves a marker, and the instance does not start on that volume.
  local restore_dir="$data_root/restore" restore_status
  [[ ! -e "$restore_dir/broken" ]] ||
    fail "A restore failed part-way and could not put the previous data back; see $restore_dir/last-restore.json. Repair the data volume, then remove $restore_dir/broken."
  if [[ -f "$restore_dir/staged.zip" ]]; then
    echo "Restoring the backup staged in $restore_dir/staged.zip." >&2
    restore_status=0
    run_as_api /usr/local/bin/stellarr --data-root "$data_root" backup apply --yes || restore_status=$?
    case "$restore_status" in
      0) ;;
      4) echo "The staged backup was not restored, and the instance starts with its previous data; see $restore_dir/last-restore.json." >&2 ;;
      *) fail "Restoring the staged backup failed; see $restore_dir/last-restore.json." ;;
    esac
  fi

  run_as_api mkdir -p "${private_dirs[@]}"
  run_as_api chmod 700 "$data_root" "${private_dirs[@]}"
}

case "$component" in
  split-web) install -d -m 0700 -o stellarr -g stellarr "$data_root" ;;
  split-turn) [[ "$turn_enabled" == true ]] || fail "The TURN service requires STELLARR_TURN_ENABLED=true." ;;
  *) prepare_instance_data ;;
esac
mkdir -p /run/nginx

# The secrets the API shares with coturn and the media server are the ones in
# the data volume, never a setting: the exports below replace any value the
# environment brings under the names the services read.

read_instance_secret() {
  # shellcheck disable=SC2016 # the child shell needs its own positional arguments
  run_as_api bash -e -c 'source /usr/local/lib/stellarr/settings.sh; read_or_create_secret "$1" "$2"' \
    read-instance-secret "$data_root" "$1"
}
case "$component" in
  split-api) media_control_secret="$(split_secret media media-control-secret)" ;;
  split-web|split-turn) media_control_secret='' ;;
  *) media_control_secret="$(read_instance_secret media-control-secret)" ;;
esac
export STELLARR_MEDIA_CONTROL_SECRET="$media_control_secret"

# What the API, the TLS script and the media server read, as resolved above.
export STELLARR_DATA_ROOT="$data_root"
export STELLARR_DOMAIN="$domain"
export STELLARR_LANGUAGE="$language"
export STELLARR_MEDIA_PORT="$media_port"
export STELLARR_TURN_ENABLED="$turn_enabled"
export STELLARR_TURN_TCP_ENABLED="$turn_tcp_enabled"
export STELLARR_TURNS_ENABLED="$turns_enabled"
# Values with no STELLARR_* name, which only the entrypoint derives.
export Rtc__MediaControlUrl="http://127.0.0.1:5200"
export Rtc__MediaControlSecret="$media_control_secret"
if [[ "$turn_enabled" == "true" ]]; then
  case "$component" in
    split-api|split-turn) turn_secret="$(split_secret turn turn-shared-secret)" ;;
    split-web) turn_secret='' ;;
    *) turn_secret="$(read_instance_secret turn-shared-secret)" ;;
  esac
  export Rtc__TurnSharedSecret="$turn_secret"
else
  unset Rtc__TurnSharedSecret
fi

# The address in mail links and the desktop client's canonical origin, as
# read_settings derived it. Without one, reset links stay off and desktop
# clients keep the address they connected to.
if [[ -n "$public_url" ]]; then
  export Operations__PublicUrl="$public_url"
  export Desktop__CanonicalOrigin="$public_url"
  echo "The public address of this instance is $public_url." >&2
else
  echo "STELLARR_DOMAIN is not set, so the instance has no public address: reset links stay off and desktop clients keep the address they connected to." >&2
fi

# Where the API listens, and the environment it runs in. Both are assigned
# whatever .env holds: nginx forwards to the loopback address, and the API
# trusts the last X-Forwarded-For entry only because nothing else can reach it,
# and any environment but production would publish a development secret and
# send the session cookie over plain HTTP.
export STELLARR_API_LISTEN="127.0.0.1:5000"
export STELLARR_ENVIRONMENT="Production"

if [[ "$component" == split-api ]]; then
  export Rtc__MediaControlUrl="http://media:5200"
  cd /opt/stellarr/api
  umask 077
  exec setpriv --reuid=stellarr --regid=stellarr --clear-groups -- ./stellarr-api
fi

if [[ "$component" != split-turn ]]; then
  /usr/local/bin/stellarr-tls-maintenance prepare
fi

# Render TURN configuration only in the container serving TURN.
if [[ "$turn_enabled" == "true" && "$component" != split-web ]]; then
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

  # Media listens on all local IPv4 interfaces. Never extend this list to
  # an address merely because it was advertised by the operator.
  read -r -a local_addresses <<< "$(hostname -I)"
  turn_peers=()
  for ip in "${local_addresses[@]}"; do
    if is_ipv4_address "$ip" && [[ "$ip" != 127.* && "$ip" != 0.0.0.0 ]]; then
      turn_peers+=("allowed-peer-ip=$ip")
    fi
  done
  (( ${#turn_peers[@]} > 0 )) || fail "TURN needs a non-loopback local IPv4 address for the media server."

  turn_config="$(</etc/turnserver.conf.template)"
  turn_config="${turn_config//__REALM__/$public_host}"
  turn_config="${turn_config//__STATIC_AUTH_SECRET__/$turn_secret}"
  turn_config="${turn_config//__EXTERNAL_IP_LINE__/$external_ip_line}"
  turn_config="${turn_config//__TURN_RELAY_MIN__/$turn_relay_min}"
  turn_config="${turn_config//__TURN_RELAY_MAX__/$turn_relay_max}"
  turn_config="${turn_config//__TURN_TCP_LINE__/$turn_tcp_line}"
  turn_config="${turn_config//__TURN_TLS_LINE__/$turn_tls_line}"
  turn_config="${turn_config//__TURN_DTLS_LINE__/$turn_dtls_line}"
  turn_pending="$(mktemp /etc/turnserver.conf.XXXXXX)"
  printf '%s\n' "$turn_config" "${turn_peers[@]}" > "$turn_pending"
  chown turnserver:turnserver "$turn_pending"
  mv -f "$turn_pending" /etc/turnserver.conf
  if [[ "$component" == split-turn ]]; then
    exec setpriv --reuid=turnserver --regid=turnserver --clear-groups -- /usr/bin/turnserver -c /etc/turnserver.conf
  fi
fi

write_instance_language /run/stellarr/stellarr-instance.js

public_https_suffix=""
if (( https_port != 443 )); then
  public_https_suffix=":$https_port"
fi
# shellcheck disable=SC2016 # nginx variable
redirect_host="${domain:-\$host}"
[[ "$domain" != *:* ]] || redirect_host="[$domain]"
nginx_config="$(</etc/nginx/nginx.conf.template)"
nginx_config="${nginx_config//__REDIRECT_HOST__/$redirect_host}"
nginx_config="${nginx_config//__PUBLIC_HTTPS_SUFFIX__/$public_https_suffix}"
activity_frames=""
: > /etc/nginx/stellarr-activity-server.conf
if [[ -n "$activity_domain" ]]; then
  activity_frames="https://*.$activity_domain$activity_https_suffix"
  cat > /etc/nginx/stellarr-activity-server.conf <<EOF
server {
  listen 80;
  listen [::]:80;
  listen 443 ssl http2;
  listen [::]:443 ssl http2;
  server_name .$activity_domain;
  ssl_certificate /run/stellarr-tls/fullchain.pem;
  ssl_certificate_key /run/stellarr-tls/privkey.pem;
  ssl_protocols TLSv1.2 TLSv1.3;
  if (\$redirect_to_https) { return 308 https://\$host$activity_https_suffix\$request_uri; }
  location / {
    proxy_pass http://127.0.0.1:5000/_activity-files/;
    proxy_set_header Host \$host;
    proxy_set_header X-Forwarded-Host \$http_host;
    proxy_set_header X-Forwarded-Proto \$effective_forwarded_proto;
    proxy_set_header X-Forwarded-For \$stellarr_forwarded_for;
  }
}
EOF
fi
nginx_config="${nginx_config//__ACTIVITY_FRAME_ORIGIN__/$activity_frames}"
script_hashes="$(</etc/nginx/stellarr-csp-script-hashes)"
[[ "$script_hashes" =~ ^(\'sha256-[A-Za-z0-9+/]{43}=\'\ ?)*$ ]] ||
  fail "The web client's CSP script hashes are invalid."
nginx_config="${nginx_config//__SCRIPT_HASHES__/$script_hashes}"
printf '%s\n' "$trusted_proxy_rules" > /etc/nginx/stellarr-trusted-proxies.conf
printf '%s\n' "$nginx_config" > /etc/nginx/nginx.conf

# The services and how each is started, checked and stopped are compiled into
# stellarr-init (init/src/services.rs), which passes each service its settings.
umask 077
if [[ "$component" == split-web ]]; then
  exec /usr/sbin/nginx -e stderr -g 'daemon off;'
fi
exec /usr/local/bin/stellarr-init run
