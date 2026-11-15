#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

COMPOSE_FILE="$REPO_ROOT/docker-compose.dev.yml"
VOLUME_NAME="${STELLARR_DEV_VOLUME:-stellarr-dev_stellarr_dev_data}"
DOCKER_IMAGE="${SEED_DOCKER_IMAGE:-python:3-alpine}"
API_PORT="${STELLARR_DEV_API_PORT:-5000}"

DB_PATH=""
NO_RESTART=0
PY_ARGS=()

usage() {
  cat <<'EOF'
Seed the Stellarr development SQLite database with sample users, channels,
multi-day conversations, reactions, mentions, friend graph, DMs, test media
and a mixed online/idle/dnd presence for the seeded accounts.

Usage:
  ./scripts/seed-dev.sh [options]

Options:
  --reset                  Replace previously seeded records while keeping
                           pre-existing non-seeded accounts and messages
  --db PATH                Seed a local SQLite database directly via host python3
                           instead of using the Docker Compose dev volume
  --compose-file PATH      Compose file path (default: ./docker-compose.dev.yml)
  --volume NAME            Docker volume name (default: stellarr-dev_stellarr_dev_data)
  --password PASS          Password for seeded accounts (default: dev-seed-2026)
  --seed INT               Deterministic RNG seed (default: 20261115)
  --bootstrap-owner        If the schema exists with 0 servers, create initial
                           owner + Stellarr server before seeding
  --owner-username USER    Owner username for --bootstrap-owner (default: admin)
  --data-root PATH         The API's DataRoot, where files/ and the presence
                           fixture live (default: derived from --db/volume)
  --no-media               Skip screenshots, GIFs and silent clips
  --no-presence            Skip the online/idle/dnd fixture for seeded accounts
  --no-restart             Do not start the API container again after seeding
  -h, --help               Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --reset|--bootstrap-owner|--no-media|--no-presence)
      PY_ARGS+=("$1")
      shift
      ;;
    --db)
      [[ $# -ge 2 ]] || { echo "Error: --db requires a path" >&2; exit 2; }
      DB_PATH="$2"
      shift 2
      ;;
    --compose-file)
      [[ $# -ge 2 ]] || { echo "Error: --compose-file requires a path" >&2; exit 2; }
      COMPOSE_FILE="$2"
      shift 2
      ;;
    --volume)
      [[ $# -ge 2 ]] || { echo "Error: --volume requires a name" >&2; exit 2; }
      VOLUME_NAME="$2"
      shift 2
      ;;
    --password|--seed|--owner-username|--data-root)
      [[ $# -ge 2 ]] || { echo "Error: $1 requires a value" >&2; exit 2; }
      PY_ARGS+=("$1" "$2")
      shift 2
      ;;
    --no-restart)
      NO_RESTART=1
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

if [[ -n "$DB_PATH" ]]; then
  if ! command -v python3 >/dev/null 2>&1; then
    echo "Error: python3 is required when seeding a local --db path." >&2
    exit 1
  fi
  exec python3 -B "$SCRIPT_DIR/seed-dev.py" --db "$DB_PATH" "${PY_ARGS[@]}"
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "Error: docker is required (or pass --db PATH to seed a local SQLite file)." >&2
  exit 1
fi

if [[ ! -f "$COMPOSE_FILE" ]]; then
  echo "Error: Compose file not found: $COMPOSE_FILE" >&2
  exit 1
fi

if ! docker volume inspect "$VOLUME_NAME" >/dev/null 2>&1; then
  echo "Error: Docker volume '$VOLUME_NAME' not found. Start the dev stack first:" >&2
  echo "  docker compose -f \"$COMPOSE_FILE\" up -d" >&2
  exit 1
fi

RESTART_NEEDED=0
API_CID="$(docker compose -f "$COMPOSE_FILE" ps -q api 2>/dev/null || true)"
if [[ -n "$API_CID" ]]; then
  IS_RUNNING="$(docker inspect -f '{{.State.Running}}' "$API_CID" 2>/dev/null || echo "false")"
  if [[ "$IS_RUNNING" == "true" ]]; then
    RESTART_NEEDED=1
  fi
fi

restart_api_if_needed() {
  if [[ "$RESTART_NEEDED" -eq 1 && "$NO_RESTART" -eq 0 ]]; then
    echo "Starting api container..."
    docker compose -f "$COMPOSE_FILE" start api >/dev/null
    RESTART_NEEDED=0
  fi
}
trap restart_api_if_needed EXIT

if [[ "$RESTART_NEEDED" -eq 1 ]]; then
  echo "Stopping api container cleanly before updating SQLite..."
  docker compose -f "$COMPOSE_FILE" stop api >/dev/null
fi

docker run --rm \
  -v "$VOLUME_NAME:/data" \
  -v "$SCRIPT_DIR:/scripts:ro" \
  "$DOCKER_IMAGE" \
  python3 -B /scripts/seed-dev.py --db /data/stellarr-dev.db "${PY_ARGS[@]}"

if [[ "$RESTART_NEEDED" -eq 1 && "$NO_RESTART" -eq 0 ]]; then
  restart_api_if_needed
  if command -v curl >/dev/null 2>&1; then
    printf "Waiting for API readiness"
    for _ in $(seq 1 30); do
      if curl -fsS "http://127.0.0.1:${API_PORT}/health/ready" >/dev/null 2>&1; then
        echo " ready."
        exit 0
      fi
      printf "."
      sleep 1
    done
    echo " (timed out waiting for /health/ready on port ${API_PORT}; check container logs)."
  fi
fi
