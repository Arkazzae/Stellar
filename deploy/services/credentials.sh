#!/usr/bin/env bash
# Only this job mounts the credential volumes writable. Older installations
# have root-owned, mode 0600 files here. Keep their values during upgrades.
set -Eeuo pipefail
shopt -s inherit_errexit
# shellcheck source=infra/settings.sh
source /usr/local/lib/stellarr/settings.sh

prepare() (
  local root="/run/credentials/$1" file="/run/credentials/$1/secrets/$2"
  exec 3<"$root"
  flock -x 3
  mkdir -p "$root/secrets"
  read_or_create_secret "$root" "$2" >/dev/null
  chgrp stellarr "$root/secrets" "$file"
  chmod 0750 "$root/secrets"
  chmod 0640 "$file"
)

prepare media media-control-secret
if [[ "${STELLARR_TURN_ENABLED:-false}" == true ]]; then
  prepare turn turn-shared-secret
fi
