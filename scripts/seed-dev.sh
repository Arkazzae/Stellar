#!/usr/bin/env bash
# Runs seed-dev.py against the database the development API actually reads.
# Written for the bash 3.2 that macOS ships: no associative arrays, and empty
# arrays are expanded with ${name[@]+...} so that `set -u` accepts them.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

PROJECT="${STELLARR_DEV_PROJECT:-stellarr-dev}"
CONTAINER="${STELLARR_DEV_API_CONTAINER:-}"
VOLUME_NAME="${STELLARR_DEV_VOLUME:-}"
COMPOSE_FILES=()
if [[ -n "${STELLARR_DEV_COMPOSE_FILES:-}" ]]; then
  IFS=',' read -r -a COMPOSE_FILES <<<"$STELLARR_DEV_COMPOSE_FILES"
fi
DOCKER_IMAGE="${SEED_DOCKER_IMAGE:-python:3-alpine}"
PYTHON_BIN="${SEED_PYTHON:-python3}"
API_URL="${STELLARR_DEV_API_URL:-}"
READY_TIMEOUT="${STELLARR_DEV_READY_TIMEOUT:-300}"

DB_PATH=""
NO_RESTART=0
DRY_RUN=0
PY_ARGS=()

usage() {
  cat <<'EOF'
Seed the Stellarr development SQLite database with sample users, channels,
multi-day conversations, reactions, mentions, a friend graph, direct messages,
test media and a mixed online/idle/dnd presence for the seeded accounts.

Usage:
  ./scripts/seed-dev.sh [options]

By default the seed goes to the database of the dev stack's api container: the
container of compose project "stellarr-dev" and service "api", running or
stopped. Its data volume, database path and published port are read from the
container itself, so a stack started with extra Compose files is seeded
where it actually keeps its data. A
running api is stopped while SQLite is written and started again afterwards.

Options:
  --reset                  Replace previously seeded records while keeping
                           pre-existing non-seeded accounts and messages
  --db PATH                Seed a local SQLite file directly with host python3
                           (3.9 or newer) instead of a Docker volume; nothing is
                           stopped, so no API may have that file open
  --container NAME         The api container to seed (default: the "api"
                           service of the compose project)
  --project NAME           Compose project whose api container is used
                           (default: stellarr-dev)
  --volume NAME            Seed this Docker volume instead of the one the api
                           container mounts at its DataRoot
  --compose-file PATH      Compose file; repeat for several. Only used to find
                           the volume when no api container exists (default:
                           the files the container was started from, else
                           ./infra/compose/dev.yml)
  --password PASS          Password for seeded accounts (default: dev-seed-2026)
  --seed INT               Deterministic RNG seed (default: 20261115)
  --bootstrap-owner        If the schema exists with 0 servers, create the owner
                           account and a 'Stellarr' server before seeding
  --owner-username USER    Owner username for --bootstrap-owner (default: admin)
  --data-root PATH         The API's DataRoot, where files/ and the presence
                           fixture live (default: the container's
                           STELLARR_DATA_ROOT, or the folder of --db)
  --no-media               Skip screenshots, GIFs and silent clips
  --no-presence            Skip the online/idle/dnd fixture for seeded accounts
  --no-restart             Do not start the api container again after seeding
  --dry-run                Print the container, volume, database and port that
                           would be used, then exit without touching anything
  -h, --help               Show this help message

Environment:
  STELLARR_DEV_PROJECT, STELLARR_DEV_API_CONTAINER, STELLARR_DEV_VOLUME and
  STELLARR_DEV_COMPOSE_FILES (comma-separated) set the defaults of the options
  above. STELLARR_DEV_API_URL overrides the readiness address,
  STELLARR_DEV_READY_TIMEOUT (seconds, default 300) how long to wait for it,
  SEED_DOCKER_IMAGE the image that runs the seed (default: python:3-alpine)
  and SEED_PYTHON the host interpreter for --db (default: python3).
EOF
}

die() {
  echo "Error: $*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --reset|--bootstrap-owner|--no-media|--no-presence)
      PY_ARGS+=("$1")
      shift
      ;;
    --db|--container|--project|--volume|--compose-file|--password|--seed|--owner-username|--data-root)
      [[ $# -ge 2 ]] || { echo "Error: $1 requires a value" >&2; exit 2; }
      case "$1" in
        --db) DB_PATH="$2" ;;
        --container) CONTAINER="$2" ;;
        --project) PROJECT="$2" ;;
        --volume) VOLUME_NAME="$2" ;;
        --compose-file) COMPOSE_FILES+=("$2") ;;
        *) PY_ARGS+=("$1" "$2") ;;
      esac
      shift 2
      ;;
    --no-restart)
      NO_RESTART=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

# --- A local file, seeded by the host's own Python -------------------------

if [[ -n "$DB_PATH" ]]; then
  command -v "$PYTHON_BIN" >/dev/null 2>&1 ||
    die "$PYTHON_BIN is required to seed a local --db path (set SEED_PYTHON to another interpreter)."
  if ! "$PYTHON_BIN" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)' 2>/dev/null; then
    die "$PYTHON_BIN ($("$PYTHON_BIN" -V 2>&1 || true)) cannot run the seed, which needs Python 3.9 or newer (set SEED_PYTHON)."
  fi
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "Would seed $DB_PATH with $("$PYTHON_BIN" -V 2>&1) ($(command -v "$PYTHON_BIN"))."
    exit 0
  fi
  exec "$PYTHON_BIN" -B "$SCRIPT_DIR/seed-dev.py" --db "$DB_PATH" ${PY_ARGS[@]+"${PY_ARGS[@]}"}
fi

# --- The dev stack's Docker volume -----------------------------------------

command -v docker >/dev/null 2>&1 ||
  die "docker is required (or pass --db PATH to seed a local SQLite file)."

# The api container: the one named, else the project's "api" service. A
# running one wins over stopped leftovers.
API_CID=""
if [[ -n "$CONTAINER" ]]; then
  API_CID="$(docker inspect -f '{{.Id}}' "$CONTAINER" 2>/dev/null)" ||
    die "No container named '$CONTAINER'."
else
  filters=(--filter "label=com.docker.compose.project=$PROJECT" --filter "label=com.docker.compose.service=api")
  candidates="$(docker ps -q "${filters[@]}")"
  [[ -n "$candidates" ]] || candidates="$(docker ps -a -q "${filters[@]}")"
  if [[ "$(printf '%s' "$candidates" | grep -c . || true)" -gt 1 ]]; then
    die "Several api containers belong to project '$PROJECT'; choose one with --container:
$(docker ps -a "${filters[@]}" --format '  {{.Names}} ({{.Status}})')"
  fi
  API_CID="$candidates"
fi

container_env() {
  docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$API_CID" | sed -n "s/^$1=//p" | head -n 1
}

API_NAME=""
API_RUNNING="false"
DATA_ROOT="/data/stellarr"
DB_IN_CONTAINER=""
CONTAINER_VOLUME=""
CONTAINER_PORT="5000"
if [[ -n "$API_CID" ]]; then
  API_NAME="$(docker inspect -f '{{.Name}}' "$API_CID" | sed 's#^/##')"
  API_RUNNING="$(docker inspect -f '{{.State.Running}}' "$API_CID")"

  configured_root="$(container_env STELLARR_DATA_ROOT)"
  [[ -n "$configured_root" ]] && DATA_ROOT="${configured_root%/}"

  # The first address the API listens on, such as 0.0.0.0:5000.
  listen="$(container_env STELLARR_API_LISTEN)"
  if [[ -n "$listen" ]]; then
    first="${listen%%,*}"
    port="${first##*:}"
    [[ "$port" =~ ^[0-9]+$ ]] && CONTAINER_PORT="$port"
  fi

  mount="$(docker inspect -f "{{range .Mounts}}{{if eq .Destination \"$DATA_ROOT\"}}{{.Type}}|{{.Name}}|{{.Source}}{{end}}{{end}}" "$API_CID")"
  case "$mount" in
    volume\|*)
      CONTAINER_VOLUME="$(printf '%s' "$mount" | cut -d'|' -f2)"
      ;;
    bind\|*)
      [[ -n "$VOLUME_NAME" ]] ||
        die "$API_NAME keeps $DATA_ROOT in a host folder ($(printf '%s' "$mount" | cut -d'|' -f3)), not a volume.
Stop it and seed that folder's database with --db PATH instead."
      ;;
    *)
      [[ -n "$VOLUME_NAME" ]] || die "$API_NAME mounts nothing at its DataRoot $DATA_ROOT; pass --volume NAME."
      ;;
  esac

  if [[ "${#COMPOSE_FILES[@]}" -eq 0 ]]; then
    label="$(docker inspect -f '{{index .Config.Labels "com.docker.compose.project.config_files"}}' "$API_CID")"
    if [[ -n "$label" && "$label" != '<no value>' ]]; then
      IFS=',' read -r -a COMPOSE_FILES <<<"$label"
      # Containers created before the move still label the old file.
      for index in "${!COMPOSE_FILES[@]}"; do
        file="${COMPOSE_FILES[index]}"
        case "$file" in
          */docker-compose.dev.yml)
            migrated="${file%/*}/infra/compose/dev.yml"
            if [[ ! -f "$file" && -f "$migrated" ]]; then
              COMPOSE_FILES[index]="$migrated"
            fi
            ;;
        esac
      done
    fi
  fi
fi

if [[ -z "$DB_IN_CONTAINER" ]]; then
  DB_IN_CONTAINER="$DATA_ROOT/db/stellarr.db"
fi
case "$DB_IN_CONTAINER" in
  "$DATA_ROOT"/*) ;;
  *) die "The database $DB_IN_CONTAINER is outside the DataRoot $DATA_ROOT; seed it with --db PATH." ;;
esac

if [[ "${#COMPOSE_FILES[@]}" -eq 0 ]]; then
  COMPOSE_FILES=("$REPO_ROOT/infra/compose/dev.yml")
fi
compose_args=(--project-name "$PROJECT")
for file in "${COMPOSE_FILES[@]}"; do
  compose_args+=(-f "$file")
done

# Without a container, the volume is the one the compose files give the api
# service at its data folder, under the name compose resolves for it.
if [[ -z "$VOLUME_NAME" && -z "$CONTAINER_VOLUME" ]]; then
  for file in "${COMPOSE_FILES[@]}"; do
    [[ -f "$file" ]] || die "Compose file not found: $file"
  done
  config="$(docker compose "${compose_args[@]}" config --format json)" ||
    die "docker compose could not read ${COMPOSE_FILES[*]}."
  CONTAINER_VOLUME="$(printf '%s' "$config" | docker run --rm -i "$DOCKER_IMAGE" python3 -c '
import json, sys
config = json.load(sys.stdin)
target = sys.argv[1]
for mount in config.get("services", {}).get("api", {}).get("volumes", []):
    if mount.get("target") == target and mount.get("type") == "volume":
        declared = config.get("volumes", {}).get(mount.get("source"), {})
        print(declared.get("name") or config.get("name", "") + "_" + mount.get("source", ""))
        break
' "$DATA_ROOT")"
  [[ -n "$CONTAINER_VOLUME" ]] ||
    die "The api service in ${COMPOSE_FILES[*]} has no named volume at $DATA_ROOT; pass --volume NAME."
fi

TARGET_VOLUME="${VOLUME_NAME:-$CONTAINER_VOLUME}"

# Stopping and starting touch only an api container whose data volume is the
# one being seeded; any other would gain nothing from a restart.
OWNS_TARGET=0
if [[ -n "$API_CID" && "$TARGET_VOLUME" == "$CONTAINER_VOLUME" ]]; then
  OWNS_TARGET=1
fi

HOST_PORT=""
if [[ -n "$API_CID" && "$API_RUNNING" == "true" ]]; then
  HOST_PORT="$(docker port "$API_CID" "$CONTAINER_PORT/tcp" 2>/dev/null | head -n 1 | sed 's/.*://')"
fi
if [[ -z "$API_URL" && -n "$HOST_PORT" ]]; then
  API_URL="http://127.0.0.1:$HOST_PORT"
fi

start_hint() {
  printf 'docker compose --project-name %q' "$PROJECT"
  for file in "${COMPOSE_FILES[@]}"; do
    printf ' -f %q' "$file"
  done
  printf ' up -d\n'
}

echo "Seed target:"
if [[ -n "$API_CID" ]]; then
  if [[ "$API_RUNNING" == "true" ]]; then state="running"; else state="stopped"; fi
  echo "  api container  $API_NAME ($state)"
else
  echo "  api container  none in project '$PROJECT'"
fi
echo "  data volume    $TARGET_VOLUME at $DATA_ROOT"
echo "  database       $DB_IN_CONTAINER"
echo "  compose files  ${COMPOSE_FILES[*]}"
if [[ -n "$API_CID" && "$OWNS_TARGET" -eq 0 ]]; then
  echo "  note           $API_NAME reads $CONTAINER_VOLUME, not $TARGET_VOLUME; it is left alone"
fi
if [[ "$OWNS_TARGET" -eq 1 && -n "$API_URL" ]]; then
  echo "  readiness      $API_URL/health/ready"
fi

if ! docker volume inspect "$TARGET_VOLUME" >/dev/null 2>&1; then
  die "Docker volume '$TARGET_VOLUME' not found. Start the dev stack once so the API creates the database:
  $(start_hint)"
fi

if [[ "$DRY_RUN" -eq 1 ]]; then
  exit 0
fi

RESTART_NEEDED=0
restart_api_if_needed() {
  if [[ "$RESTART_NEEDED" -eq 1 && "$NO_RESTART" -eq 0 ]]; then
    echo "Starting $API_NAME..."
    docker start "$API_CID" >/dev/null
    RESTART_NEEDED=0
  fi
}
trap restart_api_if_needed EXIT

if [[ "$OWNS_TARGET" -eq 1 && "$API_RUNNING" == "true" ]]; then
  echo "Stopping $API_NAME while SQLite is written..."
  docker stop -t 30 "$API_CID" >/dev/null
  RESTART_NEEDED=1
fi

docker run --rm \
  -v "$TARGET_VOLUME:$DATA_ROOT" \
  -v "$SCRIPT_DIR:/scripts:ro" \
  "$DOCKER_IMAGE" \
  python3 -B /scripts/seed-dev.py --db "$DB_IN_CONTAINER" --data-root "$DATA_ROOT" ${PY_ARGS[@]+"${PY_ARGS[@]}"}

if [[ "$RESTART_NEEDED" -eq 0 ]]; then
  if [[ -n "$API_CID" && "$OWNS_TARGET" -eq 1 && "$API_RUNNING" != "true" ]]; then
    echo "$API_NAME was not running; start the stack to see the data:"
    echo "  $(start_hint)"
  fi
  exit 0
fi

if [[ "$NO_RESTART" -eq 1 ]]; then
  echo "$API_NAME stays stopped (--no-restart); start it with: docker start $API_NAME"
  exit 0
fi

restart_api_if_needed
if [[ -z "$API_URL" ]]; then
  echo "$API_NAME publishes no port for $CONTAINER_PORT/tcp; follow its start with: docker logs -f $API_NAME"
  exit 0
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "curl is missing, so readiness is not checked; follow the start with: docker logs -f $API_NAME"
  exit 0
fi

# The watcher builds before it serves, which takes a while on a cold start.
printf "Waiting for %s/health/ready" "$API_URL"
waited=0
while [[ "$waited" -lt "$READY_TIMEOUT" ]]; do
  if curl -fsS "$API_URL/health/ready" >/dev/null 2>&1; then
    echo " ready."
    exit 0
  fi
  printf "."
  sleep 2
  waited=$((waited + 2))
done
echo " not ready after ${READY_TIMEOUT}s; check: docker logs $API_NAME"
