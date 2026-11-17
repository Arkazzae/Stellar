# Stellarr scripts

Development utilities for the local Stellarr stack.

## `seed-dev.sh` / `seed-dev.py`

Populates the development SQLite database with sample accounts, channels, messages, and media while preserving any existing owner, personal accounts, and user-created messages.

### Seeded data

- **Accounts and roles**: 28 member accounts across four custom roles (`Admin`, `Moderator`, `Core Maintainer`, `Voice Host`), plus `Owner` and `Member`. Default password for all seeded accounts: `dev-seed-2026` (override with `--password`; examples: `maya`, `theo`, `ola`, `priya`).
- **Sections and channels**: 14 channels (10 text, 4 voice alongside `#general` and `#voice`, totaling 16) under four sections:
  - `Information`: `#announcements`, `#qa-markdown`
  - `Engineering`: `#backend-dotnet`, `#frontend-vite`, `#rust-media-sfu`, `#homelab-infra`, `eng-standup` (voice)
  - `Community`: `#polski`, `#lfg-coop`, `#llm-local`, `#random-chat`, `co-op-lounge` (voice)
  - `History & Chill`: `#qa-long-history` (long multi-day history for scroll/pagination testing), `chill-room` (voice)
- **Messages and social state**: Sample messages with replies, edits, pins, reactions, code blocks, mentions, FTS5 search entries, unread markers, friendships, pending friend requests, direct messages, invites, audit entries, and two extra servers (`Homelab & Infra`, `Friday Co-op`).
- **Presence fixture**: Writes `seed-presence.json` on the data volume so development startup shows a mixed `online` / `idle` / `dnd` roster. Signing in as a seeded account replaces its fallback status with a live session.
- **Sample media**: Screenshots, GIFs, and silent MP4 clips from `scripts/seed-media/`. Skip with `--no-media` and `--no-presence`.

### Usage

Make sure the dev stack has initialized the database once (`docker compose -f docker-compose.dev.yml up -d`), then run from the repository root:

```bash
# Seed the docker-compose.dev.yml SQLite volume (pauses/restarts api automatically)
./scripts/seed-dev.sh

# Replace previously seeded data without touching personal accounts
./scripts/seed-dev.sh --reset

# Seed a standalone local SQLite file directly via host python3
./scripts/seed-dev.sh --db ./path/to/stellarr-dev.db --reset

# Bootstrap an empty schema-only DB with a starter 'admin' server first
./scripts/seed-dev.sh --reset --bootstrap-owner

# Skip media files or presence fixture
./scripts/seed-dev.sh --no-media --no-presence
```

Run `./scripts/seed-dev.sh --help` or `python3 scripts/seed-dev.py --help` for all flags, including `--data-root`.
