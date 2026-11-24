#!/usr/bin/env bash
# Uses a built image the way a person would, on a fresh data volume: waits for
# it to be healthy, takes the setup code from its log, founds the owner through
# nginx, signs in, posts a message and sees it arrive over the hub's WebSocket,
# uploads a file and downloads it back, makes a full backup, restores it with
# the operator's tool and a restart, signs in again, and runs `stellarr status`.
# A message written after the backup must be gone after the restore, so that a
# restore that did nothing fails.
#
#   STELLARR_SMOKE_IMAGE=stellarr:ci bash infra/tests/smoke.test.sh
#
# STELLARR_SMOKE_IMAGE   the image to run (default stellarr:ci)
# STELLARR_SMOKE_NAME    the container's name; its volume is NAME-data (default
#                        stellarr-smoke)
# STELLARR_SMOKE_PORT    the host port nginx's HTTPS is published on, on
#                        loopback only (default 8444)
# STELLARR_SMOKE_LIMITS  extra arguments for docker run, such as
#                        "--memory 512m --cpus 1", to see the instance in a small
#                        machine (default none)
# STELLARR_SMOKE_KEEP    set to anything to keep the container, the volume and
#                        the work folder, for someone who wants to look
#
# After the steps it prints the container's memory, peak and current, and the
# resident memory of every process in it, between MEMORY lines that are easy to
# find in a log. The numbers decide nothing; only a failed step fails the run.
#
# Needs docker, curl and python3 (its standard library only).
set -Eeuo pipefail

image="${STELLARR_SMOKE_IMAGE:-stellarr:ci}"
name="${STELLARR_SMOKE_NAME:-stellarr-smoke}"
port="${STELLARR_SMOKE_PORT:-8444}"
read -r -a limits <<<"${STELLARR_SMOKE_LIMITS:-}"
volume="$name-data"
base="https://localhost:$port"
work="$(mktemp -d "${TMPDIR:-/tmp}/stellarr-smoke.XXXXXX")"
jar="$work/cookies"
password='Passw0rd!2026'
step=''

cleanup() {
  local status=$?
  if [[ $status -ne 0 ]]; then
    echo "FAIL at: $step" >&2
    docker logs "$name" 2>&1 | tail -n 60 >&2 || true
  fi
  if [[ -n ${STELLARR_SMOKE_KEEP:-} ]]; then
    echo "Kept container $name, volume $volume and $work." >&2
  else
    docker rm -f "$name" >/dev/null 2>&1 || true
    docker volume rm "$volume" >/dev/null 2>&1 || true
    rm -rf "$work"
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

fail() {
  echo "$*" >&2
  exit 1
}

# curl as the web client calls: the client headers the instance asks for and
# the origin the address says. Writes the body to $work/body and prints the
# status.
call() {
  local method=$1 path=$2
  shift 2
  curl -ksS -X "$method" -o "$work/body" -w '%{http_code}' \
    -b "$jar" -c "$jar" \
    -H 'X-Stellarr-Client: web' -H 'X-Stellarr-Client-Version: 2.0.0' \
    -H "Origin: $base" "$@" "$base$path"
}

expect_status() {
  local want=$1 got
  shift
  got=$(call "$@") || fail "$2 could not be reached."
  [[ $got == "$want" ]] || fail "$1 $2 answered $got, not $want: $(head -c 400 "$work/body")"
}

# A member of the JSON body of the last call, by its path such as user.id.
field() {
  python3 -I -c '
import json, sys
value = json.load(open(sys.argv[1]))
for part in sys.argv[2].split("."):
    value = value[int(part)] if part.isdigit() else value[part]
print(value)' "$work/body" "$1"
}

started_at() { docker inspect -f '{{.State.StartedAt}}' "$name"; }

# Waits until the container, started after $1 (its StartedAt before), is healthy.
wait_healthy() {
  local before=$1 status=''
  for _ in $(seq 1 90); do
    status=$(docker inspect -f '{{.State.Health.Status}}' "$name")
    if [[ $(started_at) != "$before" && $status == healthy ]]; then
      return 0
    fi
    [[ $status != unhealthy ]] || break
    sleep 2
  done
  fail "The container is $status, not healthy."
}

sign_in() {
  : >"$jar"
  expect_status 200 POST /api/auth/login -H 'Content-Type: application/json' \
    -d "{\"username\":\"owner\",\"password\":\"$password\"}"
}

# Waits for the next MessageCreated record of the hub that holds $1 in its
# text, over the TLS WebSocket nginx serves, with the session cookie. Prints
# READY when it listens and OK when the record came.
listen_script='
import socket, ssl, struct, sys, json, time, base64, os
port, cookie, marker = int(sys.argv[1]), sys.argv[2], sys.argv[3]
context = ssl.create_default_context()
context.check_hostname = False
context.verify_mode = ssl.CERT_NONE
raw = socket.create_connection(("localhost", port), timeout=20)
sock = context.wrap_socket(raw, server_hostname="localhost")
key = base64.b64encode(os.urandom(16)).decode()
sock.sendall((
    "GET /hubs/app HTTP/1.1\r\nHost: localhost:%d\r\nUpgrade: websocket\r\n"
    "Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: %s\r\n"
    "Sec-WebSocket-Protocol: stellarr.v1\r\n"
    "Origin: https://localhost:%d\r\nCookie: %s\r\n\r\n" % (port, key, port, cookie)
).encode())
head = b""
while not head.endswith(b"\r\n\r\n"):
    chunk = sock.recv(1)
    if not chunk:
        sys.exit("the hub closed the upgrade: %r" % head)
    head += chunk
if b" 101 " not in head.split(b"\r\n")[0]:
    sys.exit("the hub refused the upgrade: %r" % head)
if b"\r\nsec-websocket-protocol: stellarr.v1\r\n" not in head.lower():
    sys.exit("the hub did not select stellarr.v1: %r" % head)

def send(opcode, payload):
    mask = os.urandom(4)
    sock.sendall(bytes([0x80 | opcode, 0x80 | len(payload)]) + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))

def read(n):
    data = b""
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise EOFError
        data += chunk
    return data

ready = False
clock_answered = False
deadline = time.time() + 30
sock.settimeout(5)
while time.time() < deadline:
    try:
        first, second = read(2)
    except socket.timeout:
        continue
    length = second & 0x7F
    if length == 126:
        length = struct.unpack(">H", read(2))[0]
    elif length == 127:
        length = struct.unpack(">Q", read(8))[0]
    payload = read(length)
    if first & 0x0F == 9:
        send(0xA, payload)
        continue
    if first & 0x0F != 1:
        continue
    record = json.loads(payload)
    if not ready:
        if record.get("type") != "ready":
            sys.exit("the first native message must be ready: %r" % record)
        ready = True
        send(1, b"{\"type\":\"call\",\"id\":1,\"method\":\"GetRtcClock\",\"args\":[]}")
        continue
    if record.get("id") == 1:
        if record.get("type") != "result" or not isinstance(record.get("value"), int):
            sys.exit("GetRtcClock failed: %r" % record)
        clock_answered = True
        print("READY", flush=True)
        continue
    if clock_answered and record.get("type") == "event" and record.get("name") == "MessageCreated" and marker in json.dumps(record.get("data")):
        print("OK", flush=True)
        sys.exit(0)
sys.exit("no MessageCreated with the marker within 30 seconds" if clock_answered else "GetRtcClock did not answer")
'

step="starting $image as $name on port $port"
docker rm -f "$name" >/dev/null 2>&1 || true
docker volume rm "$volume" >/dev/null 2>&1 || true
docker run -d --name "$name" -v "$volume:/data/stellarr" \
  -e STELLARR_DOMAIN=localhost -e "STELLARR_HTTPS_PORT=$port" \
  --health-interval 5s ${limits[@]+"${limits[@]}"} -p "127.0.0.1:$port:443" "$image" >/dev/null
before=''
step="waiting for the container to be healthy"
wait_healthy "$before"

step="reading the setup code from the log"
code=''
for _ in $(seq 1 30); do
  code=$(docker logs "$name" 2>&1 | sed -n 's/.*Setup code: \([^ ]*\)\. Open.*/\1/p' | tail -n 1)
  [[ -z $code ]] || break
  sleep 1
done
[[ -n $code ]] || fail "The log shows no setup code."

step="founding the owner"
expect_status 201 POST /api/auth/register -H 'Content-Type: application/json' \
  -d "{\"username\":\"owner\",\"displayName\":\"Owner\",\"email\":\"owner@example.test\",\"password\":\"$password\",\"setupCode\":\"$code\"}"

step="signing in"
sign_in
expect_status 200 GET /api/bootstrap
channel=$(python3 -I -c '
import json, sys
channels = json.load(open(sys.argv[1]))["channels"]
print(next(c["id"] for c in channels if c["name"] == "general"))' "$work/body")
[[ -n $channel ]] || fail "The instance has no general channel."

step="posting a message and seeing it over the hub"
cookie=$(awk -F'\t' '$6 ~ /session/ { print $6 "=" $7; exit }' "$jar")
[[ -n $cookie ]] || fail "Signing in set no session cookie."
python3 -I -c "$listen_script" "$port" "$cookie" "smoke-before-backup" >"$work/hub" 2>"$work/hub-error" &
hub=$!
for _ in $(seq 1 30); do
  grep -q READY "$work/hub" && break
  kill -0 "$hub" 2>/dev/null || fail "The hub listener ended: $(cat "$work/hub-error")"
  sleep 1
done
grep -q READY "$work/hub" || fail "The hub listener never connected."
expect_status 201 POST "/api/channels/$channel/messages" -H 'Content-Type: application/json' \
  -d '{"content":"smoke-before-backup"}'
wait "$hub" || fail "The message did not arrive over the hub: $(cat "$work/hub-error")"
grep -q OK "$work/hub" || fail "The hub listener did not report the message."

step="uploading a file and downloading it back"
{ printf '%%PDF-1.4\n'; head -c 4096 /dev/urandom; } >"$work/upload.pdf"
expect_status 201 POST /api/uploads/images -F "file=@$work/upload.pdf;type=application/pdf;filename=smoke.pdf"
url=$(field url)
expect_status 200 GET "$url"
cmp -s "$work/upload.pdf" "$work/body" || fail "The downloaded file differs from the uploaded one."

step="making a full backup"
expect_status 202 POST /api/cockpit/backups -H 'Content-Type: application/json' \
  -d '{"database":true,"objects":true,"keys":true,"instance":true}'
archive=$(field id)
settled=''
for _ in $(seq 1 60); do
  expect_status 200 GET /api/cockpit/backups
  if grep -q "\"id\":\"$archive\"" "$work/body" && [[ $(field job) == None ]]; then
    settled=yes
    break
  fi
  sleep 1
done
[[ -n $settled ]] || fail "The backup $archive did not finish."

step="posting a message after the backup"
expect_status 201 POST "/api/channels/$channel/messages" -H 'Content-Type: application/json' \
  -d '{"content":"smoke-after-backup"}'

step="staging the restore with the operator's tool"
docker exec "$name" stellarr backup restore "$archive" --yes
before=$(started_at)
step="restarting, which applies the staged backup"
docker restart -t 75 "$name" >/dev/null
wait_healthy "$before"

step="signing in again after the restore"
sign_in
expect_status 200 GET "/api/channels/$channel/messages"
grep -q 'smoke-before-backup' "$work/body" || fail "The message from before the backup is gone."
if grep -q 'smoke-after-backup' "$work/body"; then
  fail "The message written after the backup is still there: the restore changed nothing."
fi
expect_status 200 GET "$url"
cmp -s "$work/upload.pdf" "$work/body" || fail "The restored file differs from the uploaded one."

step="stellarr status"
docker exec "$name" stellarr status

step="measuring the memory"
echo "=== MEMORY (limits: ${STELLARR_SMOKE_LIMITS:-none}) ==="
for file in memory.peak memory.current memory.max; do
  value=$(docker exec "$name" cat "/sys/fs/cgroup/$file" 2>/dev/null || echo unavailable)
  if [[ $value =~ ^[0-9]+$ ]]; then
    echo "$file: $((value / 1048576)) MiB"
  else
    echo "$file: $value"
  fi
done
docker stats --no-stream --format 'docker stats: {{.MemUsage}} ({{.MemPerc}}), cpu {{.CPUPerc}}' "$name" || true
echo "resident memory by process (KiB):"
docker exec "$name" sh -c '
  for status in /proc/[0-9]*/status; do
    awk "/^Name:/ { name = \$2 } /^VmRSS:/ { rss = \$2 } END { if (rss != \"\") printf \"%10d  %s\\n\", rss, name }" "$status" 2>/dev/null
  done | sort -rn' || true
echo "=== END MEMORY ==="

step=''
echo "PASS: the image founded an owner, carried a message over the hub, kept a file, and came back from a backup."
