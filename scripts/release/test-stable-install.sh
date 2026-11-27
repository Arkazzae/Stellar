#!/usr/bin/env bash
# Run on a fresh runner after the latest installer and image have been promoted.
set -euo pipefail
version=${1:?expected version}
work=$(mktemp -d)
cp "$(dirname "$0")/../../install.sh" "$work/install.sh"
cd "$work"
bash install.sh --yes --dir "$work/instance" --domain localhost --public-ip 127.0.0.1 --disable-turn
test "$(docker inspect --format '{{.Config.Image}}' stellarr)" = "ghcr.io/arkazzae/stellarr:$version"
curl -kfsS https://localhost/health/ready >/dev/null
volume=$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "/data/stellarr"}}{{.Name}}{{end}}{{end}}' stellarr)
test -n "$volume"
# Switch the installed instance to the stable series and follow it on a rerun.
bash install.sh --yes --dir "$work/instance" --version "${version%.*}"
bash install.sh --yes --dir "$work/instance"
grep -Fx "STELLARR_TRACK=${version%.*}" "$work/instance/.env"
test "$(docker inspect --format '{{.Config.Image}}' stellarr)" = "ghcr.io/arkazzae/stellarr:$version"
test "$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "/data/stellarr"}}{{.Name}}{{end}}{{end}}' stellarr)" = "$volume"
curl -kfsS https://localhost/health/ready >/dev/null
