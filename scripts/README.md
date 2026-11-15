# Stellarr Scripts

Development utilities for the local Stellarr stack.

## `seed-dev.sh` / `seed-dev.py`

Seeds the SQLite development database with realistic multi-day activity while preserving any existing owner, personal accounts, and pre-existing messages.

### What Gets Seeded

- **Users & Roles:** 28 member accounts (joining existing owner/local accounts for ~30+ members) across 4 custom roles (`Admin`, `Moderator`, `Core Maintainer`, `Voice Host`) plus `Owner` and `Member`.
  - Default password for all seeded accounts: `dev-seed-2026` (override with `--password`).
  - Example logins: `maya` / `dev-seed-2026`, `theo` / `dev-seed-2026`, `ola` / `dev-seed-2026`, `priya` / `dev-seed-2026`.
- **Sections & Channels:** 14 additional channels (10 text + 4 total voice alongside starter `#general` and `#voice`, totaling 16 channels) organized under 4 sections:
  - `Information`: `#announcements`, `#qa-markdown`
  - `Engineering`: `#backend-dotnet`, `#frontend-nextjs`, `#rust-media-sfu`, `#homelab-infra`, `eng-standup` (voice)
  - `Community`: `#polski`, `#lfg-coop`, `#llm-local`, `#random-chat`, `co-op-lounge` (voice)
  - `History & Chill`: `#qa-long-history` (180 messages across multiple calendar days), `chill-room` (voice)
- **Conversations & Rich Features:** ~700+ messages with replies, edits, pins, emoji reactions, markdown/code blocks, mentions, FTS5 search entries, unread channel markers, mention notifications, friendships, pending friend requests, direct messages, invites, audit log entries, and two secondary servers (`Homelab & Infra`, `Friday Co-op`) for server-rail switching.

### Usage

Ensure the dev stack has initialized the SQLite database once (`docker compose -f docker-compose.dev.yml up -d`), then run from the repository root:

```bash
# Seed the docker-compose.dev.yml SQLite volume (automatically pauses/restarts api)
./scripts/seed-dev.sh

# Replace previously seeded data cleanly without touching personal accounts
./scripts/seed-dev.sh --reset

# Seed a standalone local SQLite file directly via host python3
./scripts/seed-dev.sh --db ./path/to/stellarr-dev.db --reset

# Initialize an empty schema-only DB (0 servers) with a starter 'admin' server first
./scripts/seed-dev.sh --reset --bootstrap-owner
```

Run `./scripts/seed-dev.sh --help` or `python3 scripts/seed-dev.py --help` for all options.
