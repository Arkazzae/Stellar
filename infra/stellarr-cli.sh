#!/usr/bin/env bash
# The operator CLI, a program of its own in /opt/stellarr/cli, built in the
# same workspace as the API.
#
# docker exec starts it with the container's configured environment, not with
# what the entrypoint exported. So it reads the settings by the entrypoint's
# rules (settings.sh) and hands the CLI the public address under the two keys
# that have no STELLARR_* name. It exports nothing under an operator's name:
# the CLI tells a value from the environment from a default by what it finds
# there. The CLI is how an instance that does not start is repaired, so a
# setting the entrypoint refuses only prints the entrypoint's message, and the
# CLI runs without the public address. Nothing here stops it from running.

# shellcheck source-path=SCRIPTDIR source=settings.sh
if source /usr/local/lib/stellarr/settings.sh && read_settings && [[ -n "$public_url" ]]; then
  export Operations__PublicUrl="$public_url"
  export Desktop__CanonicalOrigin="$public_url"
fi
exec /opt/stellarr/cli/stellarr "$@"
