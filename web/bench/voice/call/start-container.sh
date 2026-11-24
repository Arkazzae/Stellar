#!/bin/sh
# An instance for call.mjs: an existing Stellarr image with a build of web/
# mounted over the web client it carries, published on loopback only.
#
#   start-container.sh <image> <web/dist> [name] [https port]
#
# Builds nothing. Stop it with: docker rm -f <name>; docker volume rm <name>-data
set -eu
IMAGE=${1:?image, for example stellarr:local}
DIST=$(cd "${2:?the web build to serve, for example web/dist}" && pwd)
NAME=${3:-stellarr-voice-call}
HTTPS=${4:-18743}
MEDIA=$((HTTPS + 30000))
docker run -d --name "$NAME" \
  -e STELLARR_DOMAIN=localhost \
  -e STELLARR_LANGUAGE=en \
  -e STELLARR_MAIL_ENABLED=false \
  -e STELLARR_BACKUP_ENABLED=false \
  -e STELLARR_HTTPS_PORT="$HTTPS" \
  -e STELLARR_TURN_ENABLED=false \
  -e STELLARR_MEDIA_PORT="$MEDIA" \
  -e STELLARR_MEDIA_ADVERTISE="127.0.0.1:$MEDIA" \
  --security-opt no-new-privileges:true \
  --memory 3g --cpus 3 --pids-limit 512 \
  -p "127.0.0.1:$HTTPS:443" \
  -p "127.0.0.1:$MEDIA:49000/udp" \
  -p "127.0.0.1:$MEDIA:49000/tcp" \
  -v "$NAME-data:/data/stellarr" \
  -v "$DIST:/opt/stellarr/web:ro" \
  "$IMAGE"
echo "https://localhost:$HTTPS"
