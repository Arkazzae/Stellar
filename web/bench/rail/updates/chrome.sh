#!/bin/sh
# Chrome as ../../cdp.mjs starts it, plus what a call with a file for a
# microphone needs, and loopback candidates: the probe stack's media server
# advertises 127.0.0.1, which Chrome cannot reach from its LAN sockets.
# probe.mjs points STELLARR_CHROMIUM here and sets:
#
#   STELLARR_CALL_CHROME       the Chrome to run
#   STELLARR_CALL_MICROPHONE   the WAV file the fake microphone plays, in a loop
#
# The flags go in front of the page address cdp.mjs passes last.
set -eu
n=$#
i=0
while [ "$i" -lt "$n" ]; do
  arg=$1
  shift
  if [ "$arg" = "about:blank" ]; then
    set -- "$@" --ignore-certificate-errors --use-fake-device-for-media-stream \
      --use-fake-ui-for-media-stream --autoplay-policy=no-user-gesture-required \
      "--use-file-for-fake-audio-capture=${STELLARR_CALL_MICROPHONE}" \
      --disable-features=AudioServiceSandbox --allow-loopback-in-peer-connection
  fi
  set -- "$@" "$arg"
  i=$((i + 1))
done
exec "${STELLARR_CALL_CHROME}" "$@"
