# Stellarr development scripts

Development utilities for seeding and inspecting the local Stellarr stack.

## `seed-dev.sh` / `seed-dev.py`

Populates a development SQLite database with multi-day sample activity (accounts, roles, sections, text and voice channels, messages, attachments, friendships, direct messages, invites, audit entries, and presence) while preserving any pre-existing owner account, personal test accounts, and messages authored by non-seeded users.

### What gets seeded

- **Accounts and roles**: Member accounts (`USERS_SPEC` in `seed-dev.py`) assigned across four custom roles (`Admin`, `Moderator`, `Core Maintainer`, `Voice Host`) in addition to `Owner` and `Member`. All seeded accounts share the default password `dev-seed-2026` (override with `--password`; example usernames: `maya`, `theo`, `priya`, `jonas`, `kenji`, `ola`).
- **Sections and channels**: Text and voice channels alongside the default `#general` and `#voice` channels, organized under four sections:
  - `Information`: `#announcements`, `#qa-markdown`
  - `Engineering`: `#backend-rust`, `#frontend-vite`, `#rust-media-sfu`, `#homelab-infra`, `eng-standup` (voice)
  - `Community`: `#polski`, `#lfg-coop`, `#llm-local`, `#random-chat`, `co-op-lounge` (voice)
  - `History & Chill`: `#qa-long-history` (multi-day message history for scroll and pagination testing), `chill-room` (voice)
- **Messages and social graph**: Multi-day messages with replies, edits, pins, reactions, code blocks, `@mentions`, FTS5 search index rows, unread markers, friendships, pending friend requests, direct message threads, invites, audit log entries, and two secondary servers (`Homelab & Infra`, `Friday Co-op`) for server-rail switching.
- **Presence fixture**: Writes `seed-presence.json` into the API `DataRoot` so the development roster displays a mix of `online`, `idle`, and `dnd` statuses at startup. Signing in as a seeded account overrides its fixture status with a live session.
- **Sample media attachments**: Copies screenshots, animated GIFs, video posters, and silent MP4 clips from `scripts/seed-media/` into the data volume (`--no-media` and `--no-presence` skip these writes).

### Usage

By default, `./scripts/seed-dev.sh` inspects the `api` container of the `stellarr-dev` Compose project to discover its mounted data volume, `STELLARR_DATA_ROOT` (the database is `db/stellarr.db` in it), and published port. Pass `--project stellarr-dev-fresh` if you started the stack with that project name. If the `api` container is running, `seed-dev.sh` stops it before writing to SQLite, runs `seed-dev.py`, restarts the container, and waits for `/health/ready`.

```bash
# Seed the active dev stack volume (discovers volume/path from the api container)
./scripts/seed-dev.sh

# Replace all previously seeded records while keeping personal accounts and messages
./scripts/seed-dev.sh --reset

# Preview the resolved container, volume, database path, and port without writing
./scripts/seed-dev.sh --dry-run

# Seed a standalone local SQLite file directly using host python3 (3.9+)
./scripts/seed-dev.sh --db ./path/to/db/stellarr.db --reset

# Bootstrap an empty schema-only database with a starter 'admin' owner and server first
./scripts/seed-dev.sh --reset --bootstrap-owner

# Skip media files or the seed-presence.json fixture
./scripts/seed-dev.sh --no-media --no-presence
```

### Options

| Option | Description |
| --- | --- |
| `--reset` | Remove and recreate all seeded records (including actions performed while signed in as a seeded account) while preserving non-seeded accounts and their messages |
| `--db PATH` | Seed a local SQLite file directly with host `python3` (compatible with Python 3.9+); ensure no API process has the file open |
| `--container NAME` | Target a specific `api` container name |
| `--project NAME` | Compose project name used to locate the `api` container (default `stellarr-dev`) |
| `--volume NAME` | Override the Docker volume mounted at the container's `DataRoot` |
| `--compose-file PATH` | Compose file(s) used to resolve the volume when no `api` container exists yet |
| `--password PASS` | Password for all seeded accounts (default `dev-seed-2026`) |
| `--seed INT` | Deterministic RNG seed (default `20261115`) |
| `--bootstrap-owner` | Create a starter owner account and `Stellarr` server if the database has 0 servers |
| `--owner-username USER` | Username for `--bootstrap-owner` (default `admin`) |
| `--data-root PATH` | Override the API `DataRoot` directory where `files/` and `seed-presence.json` are written |
| `--no-media` | Skip copying sample images, GIFs, and MP4 clips |
| `--no-presence` | Skip writing `seed-presence.json` |
| `--no-restart` | Leave the `api` container stopped after seeding |
| `--dry-run` | Print the resolved target container, volume, database path, and readiness URL and exit |
