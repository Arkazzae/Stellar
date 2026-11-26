#!/usr/bin/env bash
# The release image has Bash. A 200 status also checks that the service is responding.
set -eu
exec 3<>"/dev/tcp/127.0.0.1/$1"
printf 'GET %s HTTP/1.0\r\nX-Forwarded-Proto: https\r\n\r\n' "$2" >&3
read -r status <&3
[[ "$status" == HTTP/*" 200 "* ]]
