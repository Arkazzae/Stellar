#!/usr/bin/env python3
"""Seed a Stellarr development SQLite database with realistic multi-day activity.

Populates 28 member accounts, 4 custom roles, 4 sections, 14 additional channels
(12 text + 4 voice total on a standard starter server), ~700+ messages with
replies/edits/pins/reactions, FTS5 search index rows (via SQLite triggers),
unread markers, @mention notifications, friendships, pending friend requests,
direct message conversations, invites, audit log entries, and two compact
secondary servers for testing rail switching.

All primary server IDs, roles, starter channels, and pre-existing users are
discovered dynamically from SQLite — no machine-specific GUIDs are hardcoded.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import random
import re
import sqlite3
import string
import sys
import uuid
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Tuple, Union

DEFAULT_DB_PATH = "/data/stellarr/db/stellarr.db"
DEFAULT_SEED_PASSWORD = "dev-seed-2026"
DEFAULT_RNG_SEED = 20261115
PBKDF2_ITERATIONS = 210_000
SEED_MEDIA_DIR = Path(__file__).resolve().parent / "seed-media"

# A runtime alias, so it cannot use the `X | Y` form: that needs Python 3.10,
# and macOS still ships 3.9 as python3.
ScriptLine = Union[Tuple[str, str], Tuple[str, str, str]]


@dataclass(frozen=True)
class MediaAsset:
    file_name: str
    content_type: str
    extension: str
    folder: str
    width: int
    height: int
    duration_seconds: float | None = None
    poster_file: str | None = None


MEDIA_ASSETS: dict[str, MediaAsset] = {
    "rail-overview": MediaAsset("channel-rail-overview.png", "image/png", ".png", "images", 1200, 675),
    "mobile-drawer": MediaAsset("member-drawer-mobile.png", "image/png", ".png", "images", 390, 780),
    "theme-swatches": MediaAsset("six-themes.png", "image/png", ".png", "images", 960, 540),
    "release-banner": MediaAsset("stellarr-release-banner.png", "image/png", ".png", "images", 1200, 420),
    "icon-motion": MediaAsset("icon-motion-preview.gif", "image/gif", ".gif", "images", 320, 240),
    "scroll-burst": MediaAsset("scroll-burst.gif", "image/gif", ".gif", "images", 320, 240),
    "scroll-demo": MediaAsset(
        "scroll-demo.mp4",
        "video/mp4",
        ".mp4",
        "videos",
        640,
        360,
        duration_seconds=6.0,
        poster_file="scroll-demo.poster.png",
    ),
    "voice-quality": MediaAsset(
        "voice-quality.mp4",
        "video/mp4",
        ".mp4",
        "videos",
        480,
        270,
        duration_seconds=4.0,
        poster_file="voice-quality.poster.png",
    ),
}

USERS_SPEC: list[tuple[str, str, str, str, str, int]] = [
    ("maya", "Maya Lin", "she/her", "Designing the rail polish", "Product & design systems. Coffee snob.", 15),
    ("theo", "Theo Vance", "he/him", "Debugging ICE candidates", "Rust, WebRTC, and overly complicated homelabs.", 42),
    ("priya", "Priya Nair", "she/her", "Shipping v2.1 perf", "Distributed systems & SQLite internals.", 75),
    ("jonas", "Jonas Lindqvist", "he/him", "In a voice lounge", "Linux tinkerer, Factorio architect, bass player.", 110),
    ("kenji", "Kenji Takahashi", "he/him", "Building local LLMs", "Rust + CUDA + mechanical keyboards.", 145),
    ("ola", "Ola Kowalska", "she/her", "Na kawie ☕", "Frontend dev z Warszawy. TypeScript & CSS.", 185),
    ("kuba", "Kuba Nowak", "he/him", "Refactoring queries", "Backend Rust, SQLite FTS5, homelab NAS.", 210),
    ("zuzanna", "Zuzanna Zielińska", "she/her", "Testuję tłumaczenia", "QA lead, accessibility & i18n enthusiast.", 245),
    ("marek", "Marek Wiśniewski", "he/him", "ZFS scrub running...", "Storage, Proxmox, WireGuard & backups.", 275),
    ("tomek", "Tomek Kamiński", "he/him", "Co-op wieczorem?", "DevOps, Nginx, Terraform and Helldivers.", 310),
    ("kasia", "Kasia Wójcik", "she/her", "Working on icons", "Illustration, pixel art, typography.", 340),
    ("piotr", "Piotr Lewandowski", "he/him", "Benchmarking scroll", "Web perf, Playwright, Flamegraphs.", 22),
    ("lena", "Lena Weber", "she/her", "Berlin -> Warsaw", "Security engineering, passkeys & WebAuthn.", 58),
    ("marcus", "Marcus Reid", "he/him", "Reading RFCs", "Networking, BGP, DNS and low-latency audio.", 92),
    ("sofia", "Sofia Morales", "she/her", "Deep work 🎧", "Mobile & Android RTC client.", 128),
    ("liam", "Liam O'Connor", "he/him", "Brewing V60", "Platform reliability, Prometheus & logs.", 162),
    ("noah", "Noah Goldstein", "he/him", "Reviewing PRs", "Compilers, WASM, Bergamot runtime.", 198),
    ("elena", "Elena Popescu", "she/her", "Writing docs", "Technical writer & VitePress maintainer.", 232),
    ("mateusz", "Mateusz Dąbrowski", "he/him", "Sprawdzam logi", "Rust, axum & WebSockets.", 265),
    ("natasha", "Natasha Ivanova", "she/her", "Audio tuning", "DSP, Opus bitrates & noise suppression.", 295),
    ("viktor", "Viktor Horvat", "he/him", "On-call this week", "Containers, systemd, kernel tuning.", 328),
    ("hanna", "Hanna Lindholm", "she/her", "Sketching layouts", "UI motion, haptics, design tokens.", 10),
    ("filip", "Filip Kaczmarek", "he/him", "Gram w Deep Rock", "Fullstack TS + Rust CLI tools.", 48),
    ("amara", "Amara Okafor", "she/her", "Training evals", "Applied ML, Whisper ASR & RAG.", 88),
    ("lucas", "Lucas Bernard", "he/him", "Tuning Nginx gzip", "Edge proxies, TLS & Cloudflare Workers.", 135),
    ("claire", "Claire Moreau", "she/her", "Moderating #general", "Community ops, events & game nights.", 172),
    ("oskar", "Oskar Szymański", "he/him", "Printing enclosures", "3D printing, KiCad PCBs & ESPHome.", 220),
    ("ania", "Ania Mazur", "she/her", "Czytam zaległości", "Data viz, React 19 & keyboard shortcuts.", 355),
]

# The permission masks of the two system roles, as the API writes them for a
# new server (api/crates/server/src/permissions/): the owner role stores every
# permission, and the base role lets everybody see the channels, send messages,
# attach files, add reactions and join voice.
OWNER_PERMISSIONS = (1 << 15) - 1
MEMBER_PERMISSIONS = (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12) | (1 << 14)

ROLES_SPEC: list[tuple[str, str, int, int]] = [
    ("Admin", "oklch(0.68 0.18 35)", 254, 1),
    ("Moderator", "oklch(0.70 0.15 160)", 240, 2),
    ("Core Maintainer", "oklch(0.72 0.14 235)", 18, 3),
    ("Voice Host", "oklch(0.74 0.14 310)", 0, 4),
]

# 14 additional channels paired with existing #general + #voice = 16 channels total.
SECTIONS_AND_CHANNELS: list[tuple[str, list[tuple[str, str, str]]]] = [
    ("Information", [
        ("announcements", "text", "Official instance releases, maintenance notes, and milestones."),
        ("qa-markdown", "text", "Formatting playground: code blocks, quotes, spoilers, and @mentions."),
    ]),
    ("Engineering", [
        ("backend-rust", "text", "The Rust API: axum, SQLite, the realtime hub, and the operator CLI."),
        ("frontend-vite", "text", "Web client and Cockpit on Vite, React 19 state, and CSS design tokens."),
        ("rust-media-sfu", "text", "stellarr-media WebRTC SFU, str0m, ICE, and Opus forwarding."),
        ("homelab-infra", "text", "Unraid, Proxmox, ZFS, Nginx, backups, and self-hosting."),
        ("eng-standup", "voice", ""),
    ]),
    ("Community", [
        ("polski", "text", "Rozmowy po polsku — luźne pogaduchy, tech i spotkania."),
        ("lfg-coop", "text", "Looking for group — co-op nights, Deep Rock, Factorio, Helldivers."),
        ("llm-local", "text", "Local open-weights, llama.cpp, Whisper, and Bergamot WASM."),
        ("random-chat", "text", "Anything goes — coffee, keyboards, music, and random banter."),
        ("co-op-lounge", "voice", ""),
    ]),
    ("History & Chill", [
        ("qa-long-history", "text", "Deep multi-day history for scroll, pagination, and FTS5 search."),
        ("chill-room", "voice", ""),
    ]),
]

# "viewer" and "owner" tokens are resolved dynamically at runtime to the discovered
# non-owner viewer (e.g., patryk if present) and server owner (e.g., admin).
# Optional 3rd tuple item attaches a test media asset from scripts/seed-media/
# (always authored by a seeded account so --reset never touches real user files).
CONVERSATIONS: dict[str, list[ScriptLine]] = {
    "general": [
        ("maya", "Morning everyone! Just pushed the new channel rail spacing tweaks — curious how it feels on tall server lists.", "rail-overview"),
        ("viewer", "Looks super crisp already. Wanna test it with 20+ active members and proper multi-day conversations."),
        ("theo", "Wait until we open all sections and jump between channels — great check for unread state and rail sync."),
        ("priya", "Earlier today `BootstrapEndpoints` loaded all read markers in one round trip — took under 2ms on SQLite."),
        ("ola", "Właśnie zaktualizowałam tłumaczenia PL w ustawieniach powiadomień, rzućcie okiem jak będziecie testować!"),
        ("kuba", "@ola sprawdziłem przed chwilą, wygląda czysto. Podoba mi się nowy podział sekcji w ustawieniach."),
        ("jonas", "Anyone hopping into `#co-op-lounge` tonight around 20:30? We still need one more for Deep Rock Galactic."),
        ("filip", "Ja się piszę! Wezmę Scouta, tylko najpierw skończę build kontenera."),
        ("zuzanna", "Dropped a quick accessibility pass on the channel search filter — `Escape` clears the query before closing the dialog."),
        ("piotr", "Measured frame times on the transcript benchmark: scrolling through 180+ rows stayed pinned under 4ms/frame 🚀", "scroll-demo"),
        ("owner", "Nice! Keep an eye on `stellarr-media` stats too when people jump between voice channels."),
        ("natasha", "Opus `clarity` vs `premium` bitrate profiles are night and day on acoustic guitar tests in `#chill-room`."),
        ("marek", "Backup drill finished on my Unraid box: SQLite online backup + `/data/stellarr/files` tar restored cleanly in 6 seconds."),
        ("lena", "Reminder: passkeys and recovery codes are independent now — test regenerating recovery codes in Account Security if you haven't yet."),
        ("marcus", "Checked TURN over TLS on port 5349 behind a strict firewall earlier — connected on the first ICE pair."),
        ("sofia", "Mobile viewport drawer feels much snappier now that the backdrop uses the shared layer stack.", "mobile-drawer"),
        ("liam", "Anyone else dialing in a new espresso roast today? Pulled a super bright Ethiopian Guji this morning ☕"),
        ("noah", "Just benchmarked the Bergamot Polish-English model in Chrome — translation of a 20-message thread finished in ~180ms."),
        ("elena", "Updated `docs/install/configuration.md` with the simplified static-export Nginx layout and removed the old RustFS notes."),
        ("mateusz", "Sprawdziłem `POST /api/channels/{id}/messages` z równoległymi retry po `ClientMessageId` — zero duplikatów."),
        ("viktor", "Container idle RAM dropped to ~150 MiB after removing Node from the production image."),
        ("hanna", "Tweaked the hover state on reaction pills across all six themes (`Obsidian`, `Midnight`, `Forest`, `Parchment`, `Slate`, `Rose`).", "theme-swatches"),
        ("amara", "Running Whisper `large-v3-turbo` locally for voice note transcription tests — crazy accurate on mixed PL/EN."),
        ("lucas", "Nginx `gzip_static on` serves pre-compressed `.gz` assets right out of `/srv/web` with zero runtime CPU."),
        ("claire", "Reminder: Friday co-op poll is open! Factorio Space Age vs Helldivers 2."),
        ("oskar", "Just finished printing a custom wall mount for my mini PC — 3h print in matte black PETG."),
        ("ania", "Ktoś używa tu skrótów klawiszowych do szybkiego skakania między kanałami? Mega przyspiesza robotę."),
        ("kenji", "Added a QMK macro layer on my split keyboard specifically for mute/deafen shortcuts."),
        ("tomek", "Postawiłem drugą instancję testową za Cloudflare Tunnel i śmiga bez żadnego problemu."),
        ("kasia", "Uploading the refreshed broken-style icon set later tonight — let me know if any 16px glyph looks soft.", "icon-motion"),
        ("maya", "Love seeing this channel actually busy! Everything holds up super cleanly under real message volume."),
        ("viewer", "Świetnie, zaraz przeklikam wszystkie kanały, wątki i DMy z tym seedem."),
    ],
    "announcements": [
        ("owner", "**Welcome to Stellarr Dev!**\nThis development instance is seeded with 30+ accounts, 16 channels across 4 sections, and hundreds of messages spanning the past two days."),
        ("priya", "**v2.0 Architecture Milestone**\n- Static web export served directly by Nginx (`~830 MB` total image)\n- Direct filesystem object storage under `/data/stellarr/files` (no RustFS needed)\n- Model-driven SQLite schema with FTS5 prefix search", "release-banner"),
        ("theo", "**Voice & SFU Update**\n`stellarr-media` handles all voice rooms over one shared UDP and TCP port (`49000` in prod, `49100` in dev). The SFU uses `str0m`."),
        ("maya", "**Design System Refresh**\nAll six palettes and `@stellarr/ui` primitives are aligned across Web and Cockpit (`/cockpit/`)."),
        ("lena", "**Security Hardening**\nPlain HTTP local development uses `stellarr-dev-session`, while HTTPS production enforces `__Host-stellarr-session`."),
        ("elena", "**Docs Updated**\nCheck `docs/` for the latest VitePress guides on installation, CLI recovery, backups, and RTC ports."),
    ],
    "qa-markdown": [
        ("zuzanna", "Formatting smoke suite:\n- **Bold text** and *italic text* and ~~strikethrough~~\n- Inline `const x = 42;` and spoiler ||secret password||\n- Escaped stars \\*not italic\\*"),
        ("piotr", "Code block test with TypeScript:\n```ts\nexport function greet(name: string): string {\n  return `Hello, ${name}!`;\n}\n```"),
        ("kuba", "SQL FTS5 query snippet:\n```sql\nSELECT m.\"Id\", m.\"Content\"\nFROM \"MessageSearch\" ms\nJOIN \"Messages\" m ON m.\"Id\" = ms.\"MessageId\"\nWHERE \"MessageSearch\" MATCH 'stellarr*';\n```"),
        ("maya", "> Blockquote check: small details make interfaces feel effortless.\n> Second line of the quote with a safe doc link: https://docs.example.test/formatting"),
        ("ola", "Testing mentions inside code shouldn't notify: `@__VIEWER__` vs real mention outside: @__VIEWER__ check this out!"),
        ("theo", "Rust snippet check:\n```rust\nlet epoch = Uuid::new_v4().simple().to_string();\ntracing::info!(%epoch, \"stellarr-media ready\");\n```"),
        ("mateusz", "Rust struct check:\n```rust\npub struct ChannelDto {\n    pub id: String,\n    pub name: String,\n    pub kind: String,\n}\n```"),
        ("noah", "Nested formatting test: ***bold italic*** and `code with **stars** inside` plus ||hidden spoiler link||."),
        ("elena", "Ordered list check:\n1. First step — verify parser output\n2. Second step — verify FTS5 indexing\n3. Third step — verify copy button on code blocks"),
        ("viewer", "Test długiej wiadomości po polsku: zażółć gęślą jaźń — ZAŻÓŁĆ GĘŚLĄ JAŹŃ ąęćłńóśźż!"),
    ],
    "backend-rust": [
        ("kuba", "Moved schema creation into one `schema.sql` that runs when the database is first opened — dropped over 3,000 lines of legacy migration code."),
        ("mateusz", "Checked the query plans on `Messages`: `(ChannelId, CreatedAt)` keeps history pagination flat."),
        ("priya", "Good thing the message loader fetches reactions and attachments in separate queries — avoids cartesian explosion when messages carry both."),
        ("kuba", "Verified the mention reader ignores fenced code blocks and URLs cleanly."),
        ("lena", "Wait: did we also verify `@__OWNER__` mentions inside blockquotes still trigger notifications?"),
        ("kuba", "Yep! Blockquotes are normal prose, only code fences, inline backticks, escaped `@`, and URLs are excluded."),
        ("mateusz", "Tested `POST /api/channels/{id}/messages` retry idempotency with `ClientMessageId` — returns `200 OK` without duplicating rows."),
        ("priya", "The slow-mode check also runs inside the insert's transaction so parallel tabs can't bypass slow mode."),
        ("viktor", "How fast is `PRAGMA wal_checkpoint(TRUNCATE)` during online backups?"),
        ("kuba", "Under 3ms on NVMe, and SQLite's backup API doesn't even block concurrent readers."),
        ("viewer", "@kuba @mateusz sprawdźcie jeszcze proszę jak zachowuje się `/api/bootstrap` przy 16 kanałach i 30 userach."),
        ("mateusz", "@__VIEWER__ wszystko wraca w jednym zapytaniu poniżej 5ms!"),
    ],
    "frontend-vite": [
        ("ola", "Po ostatnich zmianach w `TranscriptRow` memoizacja robi ogromną różnicę przy szybkich aktualizacjach reakcji."),
        ("piotr", "Isolating `contain: layout style` per row cut forced reflows to near zero when new messages arrive at the bottom."),
        ("ania", "Did anyone check how Vite's dev server handles `@stellarr/ui` hot reload in `infra/compose/dev.yml`?"),
        ("ola", "Works great with `STELLARR_DEV_POLLING=true` — it polls every 300ms, so edits in `packages/stellarr-ui/src` show up without a restart."),
        ("maya", "Remember `npm run build` writes both apps as static files to `web/dist/` for nginx, while `npm run dev` proxies `/api`, `/assets`, `/health`, and `/hubs` to the API."),
        ("piotr", "Added `npm run bench:transcript` so we can measure row render regressions objectively.", "scroll-demo"),
        ("zuzanna", "@maya also verified that delayed emoji picker autofocus no longer steals focus when switching channels rapidly."),
        ("hanna", "Check out the new empty-state illustrations and focus ring offsets — zero layout shift on keyboard tabbing.", "theme-swatches"),
        ("filip", "Did the `marked` removal cut bundle size much?"),
        ("piotr", "Yeah! Our custom parser in `packages/` is tiny, deterministic, and shares exact rules with backend mention parsing."),
    ],
    "rust-media-sfu": [
        ("theo", "Updated `str0m` to `0.25.0` with `aws-lc-rs` crypto backend — compiles cleanly for both `amd64` and `arm64`."),
        ("natasha", "How's CPU usage looking with 8 active speakers and 1 screen share at 1080p60?"),
        ("theo", "Barely 4% of a single core on my mini PC. Pure RTP forwarding avoids transcoding completely."),
        ("marcus", "Checked `STELLARR_MEDIA_ADVERTISE` in dev: advertising `127.0.0.1:49100` lets two local browser tabs connect over UDP without coturn."),
        ("tomek", "The optional dev relay uses `STELLARR_TURN_ENABLED=true` and `--profile turn`. The API advertises it on UDP port `3478`."),
        ("theo", "Exact command:\n```bash\nSTELLARR_TURN_ENABLED=true docker compose -f infra/compose/dev.yml --profile turn up\n```"),
        ("natasha", "Tested RNNoise / `@sapphi-red/web-noise-suppressor` on mechanical keyboard clicks — suppresses Cherry Browns nicely without clipping voice.", "voice-quality"),
        ("marcus", "Also verified `reset: every session closed` handles API restarts gracefully so stale rooms never linger."),
    ],
    "homelab-infra": [
        ("marek", "Finally finished migrating my TrueNAS pool from 4x 8TB mirrors to 6x 14TB RAIDZ2. Scrub ran at 620 MB/s."),
        ("liam", "Nice! Did you keep an NVMe metadata special vdev or plain ARC?"),
        ("marek", "64 GB ECC RAM for ARC + two mirrored 1TB NVMe drives for Docker appdata and SQLite databases."),
        ("viktor", "Having SQLite on NVMe with `WAL` mode makes Stellarr practically instant even with thousands of indexed messages."),
        ("tomek", "Has anyone deployed the optional Cloudflare R2 backup relay worker from `deploy/cloudflare/backup-relay`?"),
        ("lucas", "Yes! Works cleanly with Cockpit's scheduled backup retention policy — encrypted tarball uploads off-site automatically."),
        ("oskar", "Just added NUT UPS monitoring to my rack. Power draw at idle for the whole stack + switch is 38W."),
        ("marek", "38W total is super tidy. My old Xeon rack was pulling 140W just warming the hallway."),
    ],
    "polski": [
        ("viewer", "Cześć wszystkim! Sprawdzam właśnie jak wygląda serwer po nowym seedzie."),
        ("ola", "Hej! Kilkanaście kanałów podzielonych na 4 sekcje i ponad 30 użytkowników wygląda dużo czytelniej.", "rail-overview"),
        ("kuba", "Przewijanie listy kanałów i wyszukiwanie FTS5 z polskimi znakami (ąęćłńóśźż) działa błyskawicznie."),
        ("zuzanna", "Pamiętajcie też przetestować tłumaczenie w locie w przeglądarce — model `pl-en` powinien przełożyć ten wątek bez wysyłania czegokolwiek poza przeglądarkę."),
        ("marek", "Na Unraidzie cały kontener z Nginx, API i `stellarr-media` zajmuje teraz ~150 MB RAM na jałowo. Kosmos."),
        ("tomek", "Ktoś chętny dziś na małego co-opa po 21:00? Wbijajcie na `#co-op-lounge` albo piszcie na `#lfg-coop`."),
        ("ania", "Chętnie! Zrobię sobie jeszcze tylko kawę i dołączam ☕"),
        ("oskar", "Przy okazji wrzucę później zdjęcia nowej obudowy do Raspberry Pi 5 wydrukowanej z PETG.", "mobile-drawer"),
        ("mateusz", "Testujecie dzisiaj też powiadomienia na kanale? @__VIEWER__ rzuć okiem czy dzwonek w inboxie pokazuje wzmiankę."),
        ("filip", "Ja jeszcze ogarniam poprawkę w CLI i za 15 minut wbijam na głosowy."),
    ],
    "lfg-coop": [
        ("jonas", "Deep Rock Galactic Elite Deep Dive at 20:30 CET — need Driller and Engineer!"),
        ("filip", "I can run Engineer with Fat Boy overclock 💥"),
        ("tomek", "Count me in as Driller, prepping cryo cannon."),
        ("claire", "If we fill up quick, I'll open `#chill-room` for a second Helldivers 2 squad around 21:00."),
        ("liam", "Anyone playing Factorio Space Age? Finally unlocked Gleba science without spoiling my base."),
        ("jonas", "Wait until you build your first interplanetary logistics platform — pure dopamine."),
        ("ania", "Dołącza ktoś do Baldur's Gate 3 Honour Mode w weekend? Brakuje nam wsparcia na 2. akcie!"),
    ],
    "llm-local": [
        ("kenji", "Testing Qwen3-32B Q4_K_M on dual 3090s with `llama-server` — getting 74 tok/s at 32k context."),
        ("amara", "Have you tried running Whisper `large-v3-turbo` alongside it for live voice notes?"),
        ("kenji", "Yep, takes under 1.8 GB VRAM in fp16 and transcribes Polish & English accurately in ~250ms."),
        ("noah", "For browser-side stuff, Bergamot WASM models still impress me — zero GPU needed and completely private."),
        ("amara", "Also benchmarked `sqlite-vec` on 50k embeddings — sub-10ms top-k recall inside a single file."),
        ("viewer", "Ktoś testował mniejsze modele na samym CPU z `llama.cpp` do podsumowywania długich wątków?"),
        ("kenji", "Qwen3-8B Q4_K_M na Apple Silicon / nowoczesnym Ryzenie robi spokojnie 25-40 tok/s."),
    ],
    "random-chat": [
        ("liam", "Dialed in an Ethiopian washed Guji today: 18g in -> 40g out in 27s at 93°C. Jasmine and bergamot bomb."),
        ("maya", "Which grinder? Switched to SSP MP burrs last month and clarity jumped immediately."),
        ("liam", "@maya DF64 Gen 2 with stock DLC burrs — honestly hard to beat for the price."),
        ("kasia", "Unpopular opinion: rainy November afternoons are peak coding weather if you have a good lamp and lo-fi beats.", "icon-motion"),
        ("jonas", "100%. Warm desk lamp + rain outside + mechanical keyboard = zero distractions."),
        ("tomek", "Ktoś oglądał wczorajsze kwalifikacje F1? Ostatnie okrążenie było szalone."),
        ("filip", "Oglądałem! Różnica 0.012s na pole position to jakiś absurd."),
        ("claire", "Weekend bread report: 78% hydration sourdough came out with an ear big enough to hear across the kitchen 🍞"),
        ("oskar", "Just bought a 60% split ortholinear kit — brain hurts trying to type brackets right now."),
        ("piotr", "Give it 4 days of Monkeytype and you'll never go back to staggered rows.", "scroll-burst"),
    ],
}

EXTRA_FILLER_TARGETS: dict[str, int] = {
    "general": 22,
    "announcements": 8,
    "qa-markdown": 18,
    "backend-rust": 36,
    "frontend-vite": 38,
    "rust-media-sfu": 34,
    "homelab-infra": 36,
    "polski": 46,
    "lfg-coop": 34,
    "llm-local": 32,
    "random-chat": 42,
}

FILLER_EXCHANGES: list[tuple[str, str]] = [
    ("Checking the latest metrics — everything looks stable after the rebuild.", "Looks good on my end too, latency stayed flat under load."),
    ("Anyone else seeing faster cold starts after trimming the container layers?", "Yep, down to ~1.4 seconds on my box."),
    ("Pinned the summary notes here so newcomers don't have to scroll up through 100 lines.", "Appreciate it! That saves a lot of back-and-forth."),
    ("Just pushed a small cleanup PR for this area, feel free to leave nitpicks.", "Left two tiny comments on naming, otherwise ready to merge."),
    ("Tested on Firefox, Chrome, and Safari — layout lines up cleanly on all three.", "Awesome, let's also double check narrow mobile width later today."),
    ("Quick reminder to verify backups before tweaking any persistent volume config.", "Already ran a snapshot and verified the SHA-256 checksum!"),
    ("Dropped a benchmark comparison above — curious what numbers you get.", "Running it on ARM64 right now, will share results in 5 mins."),
    ("Spotted an interesting edge case with Unicode diacritics in search queries.", "Turns out `unicode61 remove_diacritics 2` handles Polish characters nicely."),
    ("Coffee break over, back to clearing out the triage queue ☕", "Godspeed! Ping me if you need a second pair of eyes on anything."),
    ("Wrzuciłem krótkie podsumowanie zmian z tego tygodnia — zerknijcie w wolnej chwili.", "Przeczytane, wszystko jasne i super opisane!"),
]

LONG_HISTORY_LINES: list[str] = [
    "Batch #{n}: checking scroll anchor stability while loading older pages.",
    "Status check #{n}: SQLite WAL checkpoint completed in 1.1ms.",
    "Note #{n}: `TranscriptRow` memoizes cleanly when only reaction counts change.",
    "Reminder #{n}: inline `code` blocks and links stay legible across all six themes.",
    "Profiler trace #{n} — forced reflow stayed well under 4ms during rapid scroll.",
    "Hej #{n}: sprawdzam paginację wstecz oraz polskie znaki (zażółć gęślą jaźń).",
    "Reply chain check #{n}: jumping to an older quoted message crosses day dividers smoothly.",
    "Link index test #{n}: https://docs.example.test/perf/run-{n} (`HasLinks` check).",
    "Voice & RTC note #{n}: ICE candidate gathering finished in 38ms.",
    "Deployment check #{n}: static assets served compressed with `gzip_static` 🚀",
]

DM_SCRIPTS: list[tuple[str, str, bool, list[ScriptLine]]] = [
    ("viewer", "maya", True, [
        ("maya", "Hej! Wrzuciłam nowe spacingi w lewej kolumnie kanałów."),
        ("viewer", "Super, właśnie sprawdzam większy seed z kilkunastoma kanałami."),
        ("maya", "Daj znać jak scrolluje się lista z sekcjami i zwijaniem!"),
        ("viewer", "Jasne, za chwilę przeklikam."),
        ("maya", "Dorzuciłam jeszcze poprawkę kontrastu dla nieprzeczytanych liczników na motywie Slate.", "rail-overview"),
    ]),
    ("viewer", "ola", True, [
        ("ola", "Cześć! Widziałeś jak działa teraz tłumaczenie wiadomości w locie?"),
        ("viewer", "Zaraz odpalę na `#polski` i `#general`."),
        ("ola", "Daj znać czy wszystko działa bez błędów w konsoli!"),
    ]),
    ("viewer", "theo", False, [
        ("theo", "Hey, media SFU is listening on `127.0.0.1:49100/udp` in dev."),
        ("viewer", "Tested two tabs earlier, ICE connected in under 200ms."),
        ("theo", "Awesome. Let me know if you want to test screen sharing at 60fps.", "voice-quality"),
    ]),
    ("viewer", "kuba", False, [
        ("kuba", "Sprawdzałeś FTS5 po ostatnim czyszczeniu `DatabaseSchema`?"),
        ("viewer", "Tak, triggery na `Messages` ładnie indeksują każdy insert."),
    ]),
    ("owner", "priya", True, [
        ("priya", "Static export cut the production container image down to ~830 MB on disk."),
        ("owner", "Saw that in the commit log, huge win over bundling Node in runtime."),
        ("priya", "Idle RAM dropped to ~150 MiB too!"),
        ("priya", "Can you verify `/cockpit/` diagnostics looks clean on your side?", "mobile-drawer"),
    ]),
    ("owner", "lena", True, [
        ("lena", "Audited the session cookie config — plain HTTP dev keeps `stellarr-dev-session` separate from `__Host-stellarr-session`."),
        ("owner", "Makes local dev painless without loosening prod HTTPS flags."),
    ]),
    ("owner", "jonas", False, [
        ("jonas", "We're running Friday Co-op tonight at 20:30 if you want to hop on voice."),
        ("owner", "Count me in!"),
    ]),
    ("maya", "kasia", False, [
        ("maya", "Love the broken icon variant in the latest `@stellarr/ui` build!"),
        ("kasia", "Thanks! Tweaked the stroke caps to align on the 24px grid.", "icon-motion"),
    ]),
    ("theo", "marcus", False, [
        ("marcus", "Checked TURN over TLS on port 5349 behind a strict firewall — worked first try."),
        ("theo", "coturn ephemeral HMAC tokens are rock solid."),
    ]),
]

EXTRA_SERVERS: list[tuple[str, str, int, list[str], list[tuple[str, list[tuple[str, str]]]]]] = [
    (
        "Homelab & Infra",
        "Self-hosting, Proxmox, ZFS pools, backups, and reverse proxies.",
        1,
        ["owner", "viewer", "theo", "marek", "tomek", "viktor", "liam", "kuba"],
        [("Lab", [("general", "text"), ("zfs-alerts", "text"), ("nginx-tls", "text"), ("lab-voice", "voice")])],
    ),
    (
        "Friday Co-op",
        "Game nights, voice rooms, and modpack configs.",
        0,
        ["owner", "viewer", "jonas", "maya", "priya", "filip", "ola", "claire"],
        [("Lobby", [("general", "text"), ("schedule", "text"), ("deep-rock", "text"), ("lounge", "voice")])],
    ),
]

INVITES_SPEC: list[tuple[str, str, int]] = [
    ("stellarrdev2026a", "general", 14),
    ("coopnight2026b", "lfg-coop", 6),
    ("polishcrew2026c", "polski", 4),
]

AUDIT_SPEC: list[tuple[str, str]] = [
    ("owner", "Created role Admin"),
    ("owner", "Created role Moderator"),
    ("owner", "Created role Core Maintainer"),
    ("maya", "Created 4 channel sections"),
    ("maya", "Added 14 channels across Information, Engineering, Community, and History & Chill"),
    ("owner", "Created invite link for #general"),
    ("lena", "Reviewed security and session configuration"),
    ("tomek", "Created invite link for #lfg-coop"),
]

REACTIONS_POOL = ["👍", "🔥", "❤️", "🎉", "🚀", "👀", "😂", "💯", "🍿", "🧠", "🛠️", "⚡"]
UNREAD_CHANNEL_NAMES = {"backend-rust", "frontend-vite", "polski", "lfg-coop", "qa-long-history"}

# Presence normally lives only in the API process and follows live hub
# connections. A seed cannot open one for every fixture account, so it leaves a
# document on the data volume that the API reads once in development. The cycle
# keeps a believable mix instead of a wall of green dots; accounts mapped to
# `offline` are simply absent from the document.
PRESENCE_FILE_NAME = "seed-presence.json"
PRESENCE_CYCLE = [
    "online", "online", "idle", "online", "dnd", "online", "online", "online",
    "idle", "offline", "online", "online", "dnd", "online", "idle", "online",
    "offline", "online", "online", "idle", "online", "dnd", "online", "online",
    "offline", "online", "online", "idle",
]

SEEDED_USERNAMES = {spec[0] for spec in USERS_SPEC}
SEEDED_DISPLAY_NAMES = {spec[1] for spec in USERS_SPEC}
SEEDED_ROLE_NAMES = {spec[0] for spec in ROLES_SPEC}
SEEDED_SECTION_NAMES = {name for name, _ in SECTIONS_AND_CHANNELS}
SEEDED_CHANNEL_NAMES = {cname for _, chs in SECTIONS_AND_CHANNELS for cname, _, _ in chs}
SEEDED_EXTRA_SERVER_NAMES = {spec[0] for spec in EXTRA_SERVERS}
SEEDED_INVITE_CODES = {spec[0] for spec in INVITES_SPEC}
SEEDED_AUDIT_SUMMARIES = {spec[1] for spec in AUDIT_SPEC}

# The lines #general gets in the name of the owner and the viewer, who are
# real accounts. #general itself stays on --reset, so these are found by their
# text; none of them carries a __VIEWER__ or __OWNER__ placeholder.
SEEDED_GENERAL_LINES_OF_REAL_ACCOUNTS = {
    line[1] for line in CONVERSATIONS["general"] if line[0] in ("owner", "viewer")
}


@dataclass(frozen=True)
class SeedContext:
    main_server_id: str
    main_server_created_at: datetime
    owner_role_id: str
    member_role_id: str
    owner_id: str
    owner_username: str
    owner_display: str
    owner_joined_at: datetime
    viewer_id: str | None
    viewer_username: str | None
    viewer_display: str | None
    viewer_joined_at: datetime | None
    general_channel_id: str
    voice_channel_id: str | None
    next_channel_pos: int
    next_section_pos: int


def new_guid() -> str:
    return str(uuid.uuid4()).upper()


def fmt_dt(dt: datetime) -> str:
    """A UTC time as the API stores one: ``yyyy-MM-dd HH:mm:ss.FFFFFFF``.

    The fraction loses its trailing zeros, and its dot when nothing is left.
    The API compares these values as text (history cursors, read markers), so
    a seeded time must be spelled exactly as one the API wrote.
    """
    utc = dt.astimezone(timezone.utc)
    fraction = f"{utc.microsecond:06d}".rstrip("0")
    return utc.strftime("%Y-%m-%d %H:%M:%S") + (f".{fraction}" if fraction else "")


def parse_dt(raw: str) -> datetime:
    cleaned = raw.strip().rstrip("Zz").replace("T", " ")
    if "." in cleaned:
        head, frac = cleaned.split(".", 1)
        frac = (frac + "000000")[:6]
        return datetime.strptime(f"{head}.{frac}", "%Y-%m-%d %H:%M:%S.%f").replace(tzinfo=timezone.utc)
    return datetime.strptime(cleaned, "%Y-%m-%d %H:%M:%S").replace(tzinfo=timezone.utc)


def hash_password(password: str) -> str:
    """Encode password in Stellarr's PasswordService PBKDF2-SHA256 format."""
    salt = os.urandom(16)
    derived = hashlib.pbkdf2_hmac("sha256", password.encode("utf-8"), salt, PBKDF2_ITERATIONS, 32)
    return (
        f"pbkdf2-sha256${PBKDF2_ITERATIONS}$"
        f"{base64.b64encode(salt).decode('ascii')}$"
        f"{base64.b64encode(derived).decode('ascii')}"
    )


def pair_ids(a: str, b: str) -> tuple[str, str]:
    # Uppercase GUID ordering, which is how the API orders the two ids of a pair.
    return (a, b) if a.upper() < b.upper() else (b, a)


def pair_key(a: str, b: str) -> str:
    x, y = pair_ids(a, b)
    return f"{x.replace('-', '').lower()}:{y.replace('-', '').lower()}"


def placeholders(count: int) -> str:
    return ",".join("?" * count)


FENCED_BLOCK = re.compile(r"```.*?(?:```|\Z)", re.S)
CODE_SPAN = re.compile(r"(`+)(.+?)(?<!`)\1(?!`)", re.S)
WEB_ADDRESS = re.compile(r"https?://\S+")
ESCAPED_CHARACTER = re.compile(r"\\[!-/:-@\[-`{-~]")
MENTION = re.compile(r"(?<![\w.-])@([\w.-]+)")


def mentioned_usernames(content: str) -> set[str]:
    """The upper-cased names a message mentions, by the API's rules in short.

    Like MessageMentions: nothing inside fenced code, code spans, web
    addresses or escapes counts, a name is the whole run of letters, digits,
    ``_``, ``.`` and ``-`` after an ``@`` that no such character precedes, and
    it is 2 to 32 characters long.
    """
    text = FENCED_BLOCK.sub(" ", content)
    text = CODE_SPAN.sub(" ", text)
    text = WEB_ADDRESS.sub(" ", text)
    text = ESCAPED_CHARACTER.sub(" ", text)
    return {
        match.group(1).upper()
        for match in MENTION.finditer(text)
        if 2 <= len(match.group(1)) <= 32
    }


def avatar_color(hue: int) -> str:
    """The colour registration gives a new account, which built-in avatars are drawn on."""
    return f"oklch(0.58 0.14 {hue % 360})"


def welcome_notice(display_name: str) -> str:
    """The system notice the API posts when somebody joins a server.

    It is message markup, so the name has every ASCII punctuation character
    escaped, as ServerWelcome does, and cannot open a span or a mention.
    """
    escaped = "".join(f"\\{ch}" if ch in string.punctuation else ch for ch in display_name)
    return f"**{escaped}** has joined the server. Say hello!"


def infer_data_root(db_path: Path) -> Path:
    """Where the API's DataRoot sits for a database file.

    The API keeps its database under ``db/`` in the data root, the dev stack and
    the container alike. A database elsewhere resolves to its own folder. The
    root is the folder that holds ``files/``, which is what attachments need.
    """
    parent = db_path.resolve().parent
    return parent.parent if parent.name.lower() == "db" else parent


def safe_object_path(root: Path, key: str) -> Path | None:
    """Resolve an object key under the files folder, or None if it escapes it."""
    if not key or key.startswith(("/", "\\")) or "\x00" in key or "\\" in key:
        return None
    if any(segment in ("", ".", "..") for segment in key.split("/")):
        return None
    candidate = (root / key).resolve()
    return candidate if candidate.is_relative_to(root.resolve()) else None


def write_object(root: Path, key: str, data: bytes) -> None:
    path = safe_object_path(root, key)
    if path is None:
        raise RuntimeError(f"Refusing to write a seed object outside the files folder: {key}")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def delete_object(root: Path, key: str | None) -> None:
    if not key:
        return
    path = safe_object_path(root, key)
    if path is None:
        return
    try:
        path.unlink()
    except FileNotFoundError:
        pass


def write_presence_seed(root: Path, users: list[tuple[str, str]]) -> None:
    """Leave the mixed presence a development API restores for offline accounts."""
    payload = {
        "version": 1,
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "users": [{"id": user_id, "presence": presence} for user_id, presence in users],
    }
    path = root / PRESENCE_FILE_NAME
    temporary = path.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def ensure_schema_present(cur: sqlite3.Cursor) -> None:
    required = {
        "Users", "Servers", "ServerAdminSettings", "Sections", "Channels",
        "ServerRoles", "ServerMembers", "Messages", "MessageReactions",
        "MessageAttachments", "ImageAttachments",
        "ChannelReads", "Friendships", "FriendRequests", "DirectConversations",
        "DirectMessages", "DirectMessageAttachments", "DirectConversationReads",
        "UserNotifications", "ServerInvites", "ServerAuditEntries",
        # Filled by the API's triggers on every message the seed inserts.
        "MessageChanges", "MessageIndexEntries", "MessageSearch", "ChannelPostTimes",
    }
    rows = cur.execute("SELECT name FROM sqlite_master WHERE type = 'table';").fetchall()
    existing = {row[0] for row in rows}
    missing = sorted(required - existing)
    if missing:
        raise RuntimeError(
            "Database schema is not initialized (missing tables: "
            + ", ".join(missing)
            + "). Start the API service once so it creates the schema before seeding."
        )
    try:
        cur.execute("CREATE VIRTUAL TABLE temp.seed_fts5_probe USING fts5(x);")
        cur.execute("DROP TABLE temp.seed_fts5_probe;")
    except sqlite3.OperationalError as exc:
        raise RuntimeError(
            f"This Python's SQLite {sqlite3.sqlite_version} has no FTS5, which the message search "
            "triggers need. Use another interpreter (SEED_PYTHON) or seed through Docker."
        ) from exc


def bootstrap_empty_server(
    cur: sqlite3.Cursor,
    now: datetime,
    pw_hash: str,
    owner_username: str,
) -> None:
    """Create starter Stellarr server + owner account when DB schema is completely empty."""
    uname = owner_username.strip().lower()
    if len(uname) < 3 or len(uname) > 32:
        raise RuntimeError(f"Invalid --owner-username '{owner_username}': must be 3-32 characters.")

    created = now - timedelta(hours=48)
    existing_user = cur.execute(
        "SELECT Id, DisplayName FROM Users WHERE NormalizedUsername = ? LIMIT 1",
        (uname.upper(),),
    ).fetchone()
    if existing_user:
        owner_id = existing_user[0]
    else:
        owner_id = new_guid()
        email = f"{uname}@example.test"
        cur.execute(
            """INSERT INTO Users (
                Id, Username, NormalizedUsername, DisplayName, PasswordHash,
                Email, NormalizedEmail, ProviderVerifiedEmail, IsDisabled, SessionVersion, SecurityRecoveryRequired,
                AvatarColor, CreatedAt, AvatarUrl, BannerUrl, CustomStatus, Bio, Pronouns
            ) VALUES (?, ?, ?, ?, ?, ?, ?, NULL, 0, 0, 0, ?, ?, NULL, NULL, 'Maintaining Stellarr Dev', NULL, NULL)""",
            (
                owner_id, uname, uname.upper(), uname.capitalize(), pw_hash, email, email.upper(),
                avatar_color(uuid.UUID(owner_id).int % 360), fmt_dt(created),
            ),
        )

    server_id = new_guid()
    owner_role_id = new_guid()
    member_role_id = new_guid()
    general_id = new_guid()
    voice_id = new_guid()

    cur.execute(
        """INSERT INTO Servers (Id, Name, Description, IsPublic, IconUrl, CreatedAt, SoundboardEveryoneCanPlay, SoundboardRateLimited)
           VALUES (?, 'Stellarr', '', 0, NULL, ?, 1, 1)""",
        (server_id, fmt_dt(created)),
    )
    cur.execute(
        """INSERT INTO ServerAdminSettings (ServerId, AllMessages, MentionsOnly, VoiceJoinSound, MutedChannelIdsJson, ServerMuted)
           VALUES (?, 0, 1, 1, '[]', 0)""",
        (server_id,),
    )
    cur.execute(
        """INSERT INTO ServerRoles (Id, ServerId, Name, Color, Permissions, Position, IsSystem, IsOwner)
           VALUES (?, ?, 'Owner', 'oklch(0.72 0.16 15)', ?, 0, 1, 1)""",
        (owner_role_id, server_id, OWNER_PERMISSIONS),
    )
    cur.execute(
        """INSERT INTO ServerRoles (Id, ServerId, Name, Color, Permissions, Position, IsSystem, IsOwner)
           VALUES (?, ?, 'Member', 'oklch(0.70 0.02 277)', ?, 1, 1, 0)""",
        (member_role_id, server_id, MEMBER_PERMISSIONS),
    )
    cur.execute(
        """INSERT INTO ServerMembers (ServerId, UserId, RoleId, OwnerSlot, JoinedAt)
           VALUES (?, ?, ?, 1, ?)""",
        (server_id, owner_id, owner_role_id, fmt_dt(created)),
    )
    cur.execute(
        """INSERT INTO Channels (
            Id, ServerId, Name, Kind, Position, Topic,
            SlowModeSeconds, HideAfterInactivityHours, UserLimit, SectionId
        ) VALUES (?, ?, 'general', 0, 0, '', 0, 0, 0, NULL)""",
        (general_id, server_id),
    )
    cur.execute(
        """INSERT INTO Channels (
            Id, ServerId, Name, Kind, Position, Topic,
            SlowModeSeconds, HideAfterInactivityHours, UserLimit, SectionId
        ) VALUES (?, ?, 'voice', 1, 1, '', 0, 0, 0, NULL)""",
        (voice_id, server_id),
    )
    # A founded server greets its owner in its first text channel.
    owner_display = cur.execute("SELECT DisplayName FROM Users WHERE Id = ?", (owner_id,)).fetchone()[0]
    cur.execute(
        """INSERT INTO Messages (Id, ChannelId, AuthorId, IsSystem, Content, CreatedAt, EditedAt, PinnedAt, ReplyToMessageId)
           VALUES (?, ?, ?, 1, ?, ?, NULL, NULL, NULL)""",
        (new_guid(), general_id, owner_id, welcome_notice(owner_display), fmt_dt(created)),
    )


def find_primary_server_row(cur: sqlite3.Cursor) -> tuple[str, str, str] | None:
    extra_ph = placeholders(len(SEEDED_EXTRA_SERVER_NAMES))
    row = cur.execute(
        f"SELECT Id, Name, CreatedAt FROM Servers WHERE Name NOT IN ({extra_ph}) ORDER BY CreatedAt ASC, Id ASC LIMIT 1",
        tuple(SEEDED_EXTRA_SERVER_NAMES),
    ).fetchone()
    if row:
        return row
    return cur.execute("SELECT Id, Name, CreatedAt FROM Servers ORDER BY CreatedAt ASC, Id ASC LIMIT 1").fetchone()


def detect_existing_seed(cur: sqlite3.Cursor, main_server_id: str) -> list[str]:
    reasons: list[str] = []

    user_ph = placeholders(len(SEEDED_USERNAMES))
    u_count = cur.execute(
        f"SELECT COUNT(*) FROM Users WHERE Username IN ({user_ph})",
        tuple(SEEDED_USERNAMES),
    ).fetchone()[0]
    if u_count:
        reasons.append(f"{u_count} seeded user(s)")

    sec_ph = placeholders(len(SEEDED_SECTION_NAMES))
    s_count = cur.execute(
        f"SELECT COUNT(*) FROM Sections WHERE ServerId = ? AND Name IN ({sec_ph})",
        (main_server_id, *SEEDED_SECTION_NAMES),
    ).fetchone()[0]
    if s_count:
        reasons.append(f"{s_count} seeded section(s)")

    ch_ph = placeholders(len(SEEDED_CHANNEL_NAMES))
    c_count = cur.execute(
        f"SELECT COUNT(*) FROM Channels WHERE ServerId = ? AND Name IN ({ch_ph})",
        (main_server_id, *SEEDED_CHANNEL_NAMES),
    ).fetchone()[0]
    if c_count:
        reasons.append(f"{c_count} seeded channel(s)")

    srv_ph = placeholders(len(SEEDED_EXTRA_SERVER_NAMES))
    srv_count = cur.execute(
        f"SELECT COUNT(*) FROM Servers WHERE Name IN ({srv_ph})",
        tuple(SEEDED_EXTRA_SERVER_NAMES),
    ).fetchone()[0]
    if srv_count:
        reasons.append(f"{srv_count} seeded secondary server(s)")

    return reasons


def reset_seeded_data(
    cur: sqlite3.Cursor,
    main_server_id: str,
    files_root: Path | None,
) -> dict[str, int]:
    """Remove what earlier seed runs added, and what the seeded accounts did since.

    Accounts, servers, channels and messages the seed did not make stay, with
    two exceptions: whatever a seeded account wrote or uploaded anywhere goes
    with the account, and so does a conversation with one. The message
    journal (MessageChanges) keeps the deletions, so clients that cached the
    seeded messages drop them on their next sync.
    """
    def ids(sql: str, params: tuple = ()) -> list[str]:
        return [row[0] for row in cur.execute(sql, params).fetchall()]

    def any_in(column: str, values: list[str]) -> tuple[str, list[str]]:
        if not values:
            return "", []
        return f"{column} IN ({placeholders(len(values))})", list(values)

    def delete_where(table: str, clauses: list[tuple[str, list[str]]]) -> int:
        live = [(clause, params) for clause, params in clauses if params]
        if not live:
            return 0
        cur.execute(
            f"DELETE FROM {table} WHERE " + " OR ".join(clause for clause, _ in live),
            tuple(param for _, params in live for param in params),
        )
        return cur.rowcount

    seeded_user_ids = ids(
        f"SELECT Id FROM Users WHERE Username IN ({placeholders(len(SEEDED_USERNAMES))})",
        tuple(SEEDED_USERNAMES),
    )
    seeded_server_ids = ids(
        f"SELECT Id FROM Servers WHERE Name IN ({placeholders(len(SEEDED_EXTRA_SERVER_NAMES))})",
        tuple(SEEDED_EXTRA_SERVER_NAMES),
    )
    seeded_section_ids = ids(
        f"SELECT Id FROM Sections WHERE ServerId = ? AND Name IN ({placeholders(len(SEEDED_SECTION_NAMES))})",
        (main_server_id, *SEEDED_SECTION_NAMES),
    )
    channel_clauses = [
        (
            f"(ServerId = ? AND Name IN ({placeholders(len(SEEDED_CHANNEL_NAMES))}))",
            [main_server_id, *SEEDED_CHANNEL_NAMES],
        ),
        any_in("SectionId", seeded_section_ids),
        any_in("ServerId", seeded_server_ids),
    ]
    live_channel_clauses = [(clause, params) for clause, params in channel_clauses if params]
    seeded_channel_ids = ids(
        "SELECT DISTINCT Id FROM Channels WHERE " + " OR ".join(clause for clause, _ in live_channel_clauses),
        tuple(param for _, params in live_channel_clauses for param in params),
    )
    user_set = set(seeded_user_ids)
    channel_set = set(seeded_channel_ids)

    # 1. Messages: all of the seeded channels, all a seeded account wrote
    #    anywhere, and the seeded lines of real accounts in #general. Their
    #    reactions, attachment links, embeds and search rows cascade.
    general_lines = sorted(SEEDED_GENERAL_LINES_OF_REAL_ACCOUNTS)
    message_clauses = [
        (
            f"""(IsSystem = 0 AND Content IN ({placeholders(len(general_lines))})
                 AND ChannelId IN (SELECT Id FROM Channels WHERE ServerId = ?))""",
            [*general_lines, main_server_id],
        ),
        any_in("ChannelId", seeded_channel_ids),
        any_in("AuthorId", seeded_user_ids),
    ]
    live = [(clause, params) for clause, params in message_clauses if params]
    where = " OR ".join(clause for clause, _ in live)
    params = tuple(param for _, values in live for param in values)
    # A real account's last post time in a channel that stays is worked out
    # again from what remains, so a seeded line no longer counts for slow mode.
    post_time_pairs = {
        (channel_id, author_id)
        for channel_id, author_id in cur.execute(
            f"SELECT DISTINCT ChannelId, AuthorId FROM Messages WHERE {where}", params
        ).fetchall()
        if channel_id not in channel_set and author_id not in user_set
    }
    cur.execute(f"DELETE FROM Messages WHERE {where}", params)
    removed_messages = cur.rowcount
    for channel_id, author_id in sorted(post_time_pairs):
        cur.execute("DELETE FROM ChannelPostTimes WHERE ChannelId = ? AND UserId = ?", (channel_id, author_id))
        cur.execute(
            """INSERT INTO ChannelPostTimes (ChannelId, UserId, LastPostedAt)
               SELECT ChannelId, AuthorId, max(CreatedAt) FROM Messages
               WHERE ChannelId = ? AND AuthorId = ? AND IsSystem = 0
               GROUP BY ChannelId, AuthorId""",
            (channel_id, author_id),
        )

    # 2. Conversations with a seeded account, with their messages, links to
    #    files, read markers and encryption epochs, which all cascade.
    delete_where("DirectConversations", [any_in("UserAId", seeded_user_ids), any_in("UserBId", seeded_user_ids)])

    # 3. What a seeded account made that would keep it from being deleted.
    object_keys: set[str] = set()
    if seeded_user_ids:
        object_keys.update(ids(
            f"SELECT ObjectKey FROM ServerEmoji WHERE UploadedById IN ({placeholders(len(seeded_user_ids))})",
            tuple(seeded_user_ids),
        ))
    delete_where("ServerEmoji", [any_in("UploadedById", seeded_user_ids)])
    delete_where("SoundboardSounds", [any_in("CreatedById", seeded_user_ids)])
    delete_where("ServerBans", [any_in("BannedById", seeded_user_ids)])
    delete_where("SecurityNotifications", [any_in("UserId", seeded_user_ids)])

    # 4. Uploads of seeded accounts: every seeded attachment, and any file
    #    somebody uploaded while signed in as one.
    seeded_attachment_ids: list[str] = []
    if seeded_user_ids:
        for attachment_id, *keys in cur.execute(
            f"""SELECT Id, ObjectKey, PosterObjectKey, PreviewObjectKey FROM ImageAttachments
                WHERE UploadedById IN ({placeholders(len(seeded_user_ids))})""",
            tuple(seeded_user_ids),
        ).fetchall():
            seeded_attachment_ids.append(attachment_id)
            object_keys.update(key for key in keys if key)
    delete_where("MessageAttachments", [any_in("AttachmentId", seeded_attachment_ids)])
    delete_where("DirectMessageAttachments", [any_in("AttachmentId", seeded_attachment_ids)])
    delete_where("ImageAttachments", [any_in("Id", seeded_attachment_ids)])

    # 5. Friendships and requests of seeded accounts.
    delete_where("FriendRequests", [any_in("SenderId", seeded_user_ids), any_in("ReceiverId", seeded_user_ids)])
    delete_where("Friendships", [any_in("UserAId", seeded_user_ids), any_in("UserBId", seeded_user_ids)])

    # 6. Inbox entries: of seeded accounts, about seeded channels or servers,
    #    and real accounts' mentions written in a seeded account's name.
    display_names = sorted(SEEDED_DISPLAY_NAMES)
    delete_where("UserNotifications", [
        any_in("UserId", seeded_user_ids),
        any_in("TargetChannelId", seeded_channel_ids),
        any_in("ServerId", seeded_server_ids),
        (
            f"""(ServerId = ? AND SourcePackageId = 'stellarr.mention'
                 AND SourceModuleName IN ({placeholders(len(display_names))}))""",
            [main_server_id, *display_names],
        ),
    ])

    # 7. Invites and audit entries.
    delete_where("ServerInvites", [
        any_in("Code", sorted(SEEDED_INVITE_CODES)),
        any_in("CreatedById", seeded_user_ids),
        any_in("ServerId", seeded_server_ids),
    ])
    delete_where("ServerAuditEntries", [
        (
            f"(ServerId = ? AND Summary IN ({placeholders(len(SEEDED_AUDIT_SUMMARIES))}))",
            [main_server_id, *sorted(SEEDED_AUDIT_SUMMARIES)],
        ),
        any_in("ActorId", seeded_user_ids),
        any_in("ServerId", seeded_server_ids),
    ])

    # 8. Read markers, channels and sections.
    delete_where("ChannelReads", [any_in("ChannelId", seeded_channel_ids), any_in("UserId", seeded_user_ids)])
    delete_where("Channels", [any_in("Id", seeded_channel_ids)])
    delete_where("Sections", [any_in("Id", seeded_section_ids)])

    # 9. Members and roles; real members move off seeded roles first.
    member_role_row = cur.execute(
        """SELECT Id FROM ServerRoles
           WHERE ServerId = ? AND IsSystem = 1 AND IsOwner = 0
           ORDER BY Position DESC LIMIT 1""",
        (main_server_id,),
    ).fetchone()
    seeded_custom_role_ids = ids(
        f"""SELECT Id FROM ServerRoles
            WHERE ServerId = ? AND IsSystem = 0 AND IsOwner = 0
            AND Name IN ({placeholders(len(SEEDED_ROLE_NAMES))})""",
        (main_server_id, *SEEDED_ROLE_NAMES),
    )
    if member_role_row and seeded_custom_role_ids:
        cur.execute(
            f"UPDATE ServerMembers SET RoleId = ? WHERE ServerId = ? AND RoleId IN ({placeholders(len(seeded_custom_role_ids))})",
            (member_role_row[0], main_server_id, *seeded_custom_role_ids),
        )
    delete_where("ServerMembers", [any_in("UserId", seeded_user_ids), any_in("ServerId", seeded_server_ids)])
    delete_where("ServerRoles", [any_in("Id", seeded_custom_role_ids)])
    if member_role_row:
        cur.execute("UPDATE ServerRoles SET Position = 1 WHERE Id = ?", (member_role_row[0],))

    # 10. The secondary servers, with what hangs off them, and the accounts,
    #     whose sessions, keys, preferences and read markers cascade.
    delete_where("Servers", [any_in("Id", seeded_server_ids)])
    delete_where("Users", [any_in("Id", seeded_user_ids)])

    if files_root is not None:
        for object_key in sorted(object_keys):
            delete_object(files_root, object_key)

    return {
        "removed_users": len(seeded_user_ids),
        "removed_servers": len(seeded_server_ids),
        "removed_channels": len(seeded_channel_ids),
        "removed_messages": removed_messages,
        "removed_attachments": len(seeded_attachment_ids),
    }


def discover_context(cur: sqlite3.Cursor, main_server_id: str, main_created_at_raw: str) -> SeedContext:
    owner_role = cur.execute(
        "SELECT Id FROM ServerRoles WHERE ServerId = ? AND IsOwner = 1 ORDER BY Position ASC LIMIT 1",
        (main_server_id,),
    ).fetchone()
    if not owner_role:
        raise RuntimeError(f"Server {main_server_id} has no owner role.")

    member_role = cur.execute(
        """SELECT Id FROM ServerRoles
           WHERE ServerId = ? AND IsSystem = 1 AND IsOwner = 0
           ORDER BY Position DESC LIMIT 1""",
        (main_server_id,),
    ).fetchone()
    if not member_role:
        raise RuntimeError(f"Server {main_server_id} has no default Member system role.")

    owner_row = cur.execute(
        """SELECT u.Id, u.Username, u.DisplayName, m.JoinedAt
           FROM ServerMembers m
           JOIN Users u ON u.Id = m.UserId
           WHERE m.ServerId = ? AND m.OwnerSlot = 1
           LIMIT 1""",
        (main_server_id,),
    ).fetchone()
    if not owner_row:
        owner_row = cur.execute(
            """SELECT u.Id, u.Username, u.DisplayName, m.JoinedAt
               FROM ServerMembers m
               JOIN Users u ON u.Id = m.UserId
               WHERE m.ServerId = ? AND m.RoleId = ?
               ORDER BY m.JoinedAt ASC LIMIT 1""",
            (main_server_id, owner_role[0]),
        ).fetchone()
    if not owner_row:
        raise RuntimeError(f"Server {main_server_id} has no owner member.")

    other_members = cur.execute(
        """SELECT u.Id, u.Username, u.DisplayName, m.JoinedAt
           FROM ServerMembers m
           JOIN Users u ON u.Id = m.UserId
           WHERE m.ServerId = ? AND m.UserId != ? AND u.IsDisabled = 0
           ORDER BY CASE WHEN u.Username = 'patryk' THEN 0 ELSE 1 END, m.JoinedAt ASC, u.Id ASC""",
        (main_server_id, owner_row[0]),
    ).fetchall()
    viewer_row = other_members[0] if other_members else None

    general_row = cur.execute(
        """SELECT Id FROM Channels
           WHERE ServerId = ? AND Kind = 0
           ORDER BY CASE WHEN Name = 'general' THEN 0 ELSE 1 END, Position ASC LIMIT 1""",
        (main_server_id,),
    ).fetchone()
    if not general_row:
        general_id = new_guid()
        next_pos = (
            cur.execute("SELECT COALESCE(MAX(Position), -1) + 1 FROM Channels WHERE ServerId = ?", (main_server_id,)).fetchone()[0]
        )
        cur.execute(
            """INSERT INTO Channels (
                Id, ServerId, Name, Kind, Position, Topic,
                SlowModeSeconds, HideAfterInactivityHours, UserLimit, SectionId
            ) VALUES (?, ?, 'general', 0, ?, '', 0, 0, 0, NULL)""",
            (general_id, main_server_id, next_pos),
        )
    else:
        general_id = general_row[0]

    voice_row = cur.execute(
        """SELECT Id FROM Channels
           WHERE ServerId = ? AND Kind = 1
           ORDER BY CASE WHEN Name = 'voice' THEN 0 ELSE 1 END, Position ASC LIMIT 1""",
        (main_server_id,),
    ).fetchone()

    next_channel_pos = cur.execute(
        "SELECT COALESCE(MAX(Position), -1) + 1 FROM Channels WHERE ServerId = ?",
        (main_server_id,),
    ).fetchone()[0]
    next_section_pos = cur.execute(
        "SELECT COALESCE(MAX(Position), -1) + 1 FROM Sections WHERE ServerId = ?",
        (main_server_id,),
    ).fetchone()[0]

    return SeedContext(
        main_server_id=main_server_id,
        main_server_created_at=parse_dt(main_created_at_raw),
        owner_role_id=owner_role[0],
        member_role_id=member_role[0],
        owner_id=owner_row[0],
        owner_username=owner_row[1],
        owner_display=owner_row[2],
        owner_joined_at=parse_dt(owner_row[3]),
        viewer_id=viewer_row[0] if viewer_row else None,
        viewer_username=viewer_row[1] if viewer_row else None,
        viewer_display=viewer_row[2] if viewer_row else None,
        viewer_joined_at=parse_dt(viewer_row[3]) if viewer_row else None,
        general_channel_id=general_id,
        voice_channel_id=voice_row[0] if voice_row else None,
        next_channel_pos=next_channel_pos,
        next_section_pos=next_section_pos,
    )


def resolve_actor_and_text(
    actor_key: str,
    text: str,
    ctx: SeedContext,
    user_ids: dict[str, str],
) -> tuple[str, str]:
    """Map symbolic author/mention placeholders (`owner`, `viewer`) onto discovered users."""
    viewer_uname = ctx.viewer_username or ctx.owner_username
    resolved_text = text.replace("__VIEWER__", viewer_uname).replace("__OWNER__", ctx.owner_username)

    if actor_key == "owner":
        return ctx.owner_username, resolved_text
    if actor_key == "viewer":
        return viewer_uname, resolved_text
    if actor_key in user_ids:
        return actor_key, resolved_text
    return "maya", resolved_text


def seed_database(
    db_path: Path,
    *,
    password: str,
    rng_seed: int,
    reset: bool,
    bootstrap_owner: bool,
    owner_username: str,
    data_root: Path,
    include_media: bool,
    include_presence: bool,
) -> dict[str, int]:
    if not db_path.exists():
        raise FileNotFoundError(
            f"Database file not found at '{db_path}'. Start the API once or pass an initialized SQLite DB path."
        )

    rng = random.Random(rng_seed)
    now = datetime.now(timezone.utc)
    pw_hash = hash_password(password)
    files_root = data_root / "files"
    if (include_media or include_presence) and not data_root.is_dir():
        raise FileNotFoundError(
            f"Data folder not found at '{data_root}'. Pass --data-root PATH (the API's DataRoot) "
            "or --no-media --no-presence to seed without attachments and statuses."
        )
    if include_media:
        files_root.mkdir(parents=True, exist_ok=True)

    con = sqlite3.connect(str(db_path))
    try:
        con.execute("PRAGMA foreign_keys = ON;")
        con.execute("PRAGMA journal_mode = WAL;")
        con.execute("PRAGMA busy_timeout = 10000;")
        cur = con.cursor()

        ensure_schema_present(cur)

        with con:
            server_row = find_primary_server_row(cur)
            if not server_row:
                if not bootstrap_owner:
                    raise RuntimeError(
                        "No server found in database. Complete initial setup in the app or pass --bootstrap-owner."
                    )
                bootstrap_empty_server(cur, now, pw_hash, owner_username)
                server_row = find_primary_server_row(cur)
                if not server_row:
                    raise RuntimeError("Failed to bootstrap initial server.")

            main_server_id, _, main_created_at_raw = server_row

            existing_reasons = detect_existing_seed(cur, main_server_id)
            if existing_reasons and not reset:
                raise RuntimeError(
                    "Database already contains seeded records ("
                    + ", ".join(existing_reasons)
                    + "). Re-run with --reset to cleanly replace seeded data."
                )

            removed: dict[str, int] = {}
            if reset:
                removed = reset_seeded_data(cur, main_server_id, files_root if files_root.is_dir() else None)

            ctx = discover_context(cur, main_server_id, main_created_at_raw)

            # 1. Roles on primary server
            cur.execute("UPDATE ServerRoles SET Position = ? WHERE Id = ?", (len(ROLES_SPEC) + 1, ctx.member_role_id))
            role_ids: dict[str, str] = {"Owner": ctx.owner_role_id, "Member": ctx.member_role_id}
            for rname, rcolor, rperm, rpos in ROLES_SPEC:
                rid = new_guid()
                cur.execute(
                    """INSERT INTO ServerRoles (Id, ServerId, Name, Color, Permissions, Position, IsSystem, IsOwner)
                       VALUES (?, ?, ?, ?, ?, ?, 0, 0)""",
                    (rid, main_server_id, rname, rcolor, rperm, rpos),
                )
                role_ids[rname] = rid

            # 2. Seeded Users & primary server membership
            # Ensure every primary member JoinedAt sits strictly after main_server_created_at
            # and strictly before now so secondary servers sort later in BootstrapEndpoints.
            join_floor = ctx.main_server_created_at + timedelta(seconds=10)
            join_ceiling = now - timedelta(minutes=2)
            if join_ceiling <= join_floor:
                join_ceiling = now - timedelta(seconds=10)
                join_floor = min(ctx.main_server_created_at + timedelta(seconds=1), join_ceiling - timedelta(seconds=len(USERS_SPEC) + 1))

            join_span_secs = max(len(USERS_SPEC), int((join_ceiling - join_floor).total_seconds()))
            user_ids: dict[str, str] = {ctx.owner_username: ctx.owner_id}
            user_display: dict[str, str] = {ctx.owner_username: ctx.owner_display}
            if ctx.viewer_id and ctx.viewer_username and ctx.viewer_display:
                user_ids[ctx.viewer_username] = ctx.viewer_id
                user_display[ctx.viewer_username] = ctx.viewer_display

            # Avoid email collisions if an existing account already took `<uname>@example.test`.
            existing_emails = {
                row[0] for row in cur.execute("SELECT NormalizedEmail FROM Users").fetchall()
            }

            user_join_dates: dict[str, datetime] = {}
            max_primary_join = max(
                [ctx.owner_joined_at] + ([ctx.viewer_joined_at] if ctx.viewer_joined_at else [])
            )

            for idx, (uname, dname, pronouns, status, bio, hue) in enumerate(USERS_SPEC):
                joined_at = join_floor + timedelta(
                    seconds=int(join_span_secs * (idx + 1) / (len(USERS_SPEC) + 2))
                )
                if joined_at > max_primary_join:
                    max_primary_join = joined_at
                uid = new_guid()
                email = f"{uname}@example.test"
                if email.upper() in existing_emails:
                    email = f"{uname}.seed@example.test"
                existing_emails.add(email.upper())

                # AvatarUrl and BannerUrl stay NULL: clients draw built-in art
                # from the user id on AvatarColor, as for a new registration.
                cur.execute(
                    """INSERT INTO Users (
                        Id, Username, NormalizedUsername, DisplayName, PasswordHash,
                        Email, NormalizedEmail, ProviderVerifiedEmail, IsDisabled, SessionVersion, SecurityRecoveryRequired,
                        AvatarColor, CreatedAt, AvatarUrl, BannerUrl, CustomStatus, Bio, Pronouns
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, NULL, 0, 0, 0, ?, ?, NULL, NULL, ?, ?, ?)""",
                    (
                        uid, uname, uname.upper(), dname, pw_hash,
                        email, email.upper(), avatar_color(hue), fmt_dt(joined_at),
                        status, bio, pronouns,
                    ),
                )
                user_ids[uname] = uid
                user_display[uname] = dname
                user_join_dates[uname] = joined_at

                if idx < 2:
                    rname = "Admin"
                elif idx < 6:
                    rname = "Moderator"
                elif idx < 12:
                    rname = "Core Maintainer"
                elif idx < 16:
                    rname = "Voice Host"
                else:
                    rname = "Member"

                cur.execute(
                    """INSERT INTO ServerMembers (ServerId, UserId, RoleId, OwnerSlot, JoinedAt)
                       VALUES (?, ?, ?, NULL, ?)""",
                    (main_server_id, uid, role_ids[rname], fmt_dt(joined_at)),
                )

            all_active_users: list[str] = [ctx.owner_username]
            if ctx.viewer_username and ctx.viewer_username not in all_active_users:
                all_active_users.append(ctx.viewer_username)
            all_active_users.extend(u[0] for u in USERS_SPEC)

            # 3. Sections & Channels on primary server. The starter channels
            # belong to the instance and are left as they are.

            chan_pos = ctx.next_channel_pos
            sec_pos = ctx.next_section_pos
            channel_ids: dict[str, str] = {"general": ctx.general_channel_id}
            if ctx.voice_channel_id:
                channel_ids["voice"] = ctx.voice_channel_id
            text_channels: list[tuple[str, str]] = [("general", ctx.general_channel_id)]

            for sec_name, ch_list in SECTIONS_AND_CHANNELS:
                sec_id = new_guid()
                cur.execute(
                    "INSERT INTO Sections (Id, ServerId, Name, Position) VALUES (?, ?, ?, ?)",
                    (sec_id, main_server_id, sec_name, sec_pos),
                )
                sec_pos += 1

                for cname, ckind_str, ctopic in ch_list:
                    ckind = 0 if ckind_str == "text" else 1
                    cid = new_guid()
                    ulimit = 12 if cname == "co-op-lounge" else (0 if ckind == 0 else 16)
                    cur.execute(
                        """INSERT INTO Channels (
                            Id, ServerId, Name, Kind, Position, Topic,
                            SlowModeSeconds, HideAfterInactivityHours, UserLimit, SectionId
                        ) VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?)""",
                        (cid, main_server_id, cname, ckind, chan_pos, ctopic, ulimit, sec_id),
                    )
                    chan_pos += 1
                    channel_ids[cname] = cid
                    if ckind == 0:
                        text_channels.append((cname, cid))

            # 4. System welcome notices in #general for initial seeded joins
            for uname, _, _, _, _, _ in USERS_SPEC[:6]:
                cur.execute(
                    """INSERT INTO Messages (Id, ChannelId, AuthorId, IsSystem, Content, CreatedAt, EditedAt, PinnedAt, ReplyToMessageId)
                       VALUES (?, ?, ?, 1, ?, ?, NULL, NULL, NULL)""",
                    (
                        new_guid(),
                        ctx.general_channel_id,
                        user_ids[uname],
                        welcome_notice(user_display[uname]),
                        fmt_dt(user_join_dates[uname]),
                    ),
                )

            # 5. Conversations across channels
            history_floor = min(ctx.main_server_created_at, now - timedelta(hours=48))
            latest_msg_at: dict[str, datetime] = {}
            mention_targets: list[tuple[str, str, str, str, datetime]] = []

            viewer_uname_for_mentions = ctx.viewer_username or ctx.owner_username
            # Everybody a message on the main server can mention: its members.
            member_ids_by_name = {
                row[0]: row[1]
                for row in cur.execute(
                    """SELECT u.NormalizedUsername, u.Id FROM ServerMembers m
                       JOIN Users u ON u.Id = m.UserId WHERE m.ServerId = ?""",
                    (main_server_id,),
                ).fetchall()
            }

            media_cache: dict[str, bytes] = {}
            poster_cache: dict[str, bytes] = {}

            def read_seed_media(file_name: str, cache: dict[str, bytes]) -> bytes:
                cached = cache.get(file_name)
                if cached is not None:
                    return cached
                path = SEED_MEDIA_DIR / file_name
                if not path.is_file():
                    raise RuntimeError(
                        f"Seed media file '{path}' is missing. Restore scripts/seed-media/ or pass --no-media."
                    )
                data = path.read_bytes()
                cache[file_name] = data
                return data

            def add_attachment(
                message_id: str,
                author_uname: str,
                asset_key: str,
                created_at: datetime,
                *,
                direct: bool = False,
            ) -> None:
                if not include_media:
                    raise RuntimeError("Seed media was requested but --no-media turned attachments off.")
                if author_uname not in SEEDED_USERNAMES:
                    raise RuntimeError(
                        f"Seed attachment '{asset_key}' must belong to a seeded user from USERS_SPEC, not '{author_uname}'."
                    )
                asset = MEDIA_ASSETS[asset_key]
                payload = read_seed_media(asset.file_name, media_cache)

                attachment_id = new_guid()
                stem = attachment_id.replace("-", "").lower()
                month = created_at.astimezone(timezone.utc).strftime("%Y/%m")
                object_key = f"{asset.folder}/{month}/{stem}{asset.extension}"
                write_object(files_root, object_key, payload)

                poster_key = None
                poster_type = None
                poster_size = None
                if asset.poster_file is not None:
                    poster = read_seed_media(asset.poster_file, poster_cache)
                    poster_key = f"posters/{month}/{stem}.png"
                    poster_type = "image/png"
                    poster_size = len(poster)
                    write_object(files_root, poster_key, poster)

                # No preview: a browser sends one only when it saves enough
                # over the original, which these small files never do, and
                # never for an animated GIF. Plain files, so IsEncrypted is 0.
                cur.execute(
                    """INSERT INTO ImageAttachments (
                        Id, UploadedById, ObjectKey, FileName, ContentType, SizeBytes,
                        Width, Height, DurationSeconds, IsVoiceMessage,
                        PosterObjectKey, PosterContentType, PosterSizeBytes,
                        PreviewObjectKey, PreviewContentType, PreviewSizeBytes, Placeholder,
                        IsEncrypted, CreatedAt
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, NULL, NULL, NULL, NULL, 0, ?)""",
                    (
                        attachment_id,
                        user_ids[author_uname],
                        object_key,
                        asset.file_name,
                        asset.content_type,
                        len(payload),
                        asset.width,
                        asset.height,
                        asset.duration_seconds,
                        poster_key,
                        poster_type,
                        poster_size,
                        fmt_dt(created_at),
                    ),
                )
                if direct:
                    cur.execute(
                        "INSERT INTO DirectMessageAttachments (DirectMessageId, AttachmentId) VALUES (?, ?)",
                        (message_id, attachment_id),
                    )
                else:
                    cur.execute(
                        "INSERT INTO MessageAttachments (MessageId, AttachmentId) VALUES (?, ?)",
                        (message_id, attachment_id),
                    )

            def insert_msg(
                cid: str,
                author_uname: str,
                content: str,
                created_at: datetime,
                edited_at: datetime | None = None,
                pinned_at: datetime | None = None,
                reply_to_id: str | None = None,
                track_mentions: bool = True,
                attachment: str | None = None,
            ) -> str:
                mid = new_guid()
                cur.execute(
                    """INSERT INTO Messages (
                        Id, ChannelId, AuthorId, IsSystem, Content, CreatedAt, EditedAt, PinnedAt, ReplyToMessageId
                    ) VALUES (?, ?, ?, 0, ?, ?, ?, ?, ?)""",
                    (
                        mid,
                        cid,
                        user_ids[author_uname],
                        content,
                        fmt_dt(created_at),
                        fmt_dt(edited_at) if edited_at else None,
                        fmt_dt(pinned_at) if pinned_at else None,
                        reply_to_id,
                    ),
                )
                if attachment is not None:
                    add_attachment(mid, author_uname, attachment, created_at)
                prev = latest_msg_at.get(cid)
                if prev is None or created_at > prev:
                    latest_msg_at[cid] = created_at
                if track_mentions:
                    # The inbox entries the API adds for a message: one for
                    # each member it names, never for its author.
                    author_id = user_ids[author_uname]
                    for name in sorted(mentioned_usernames(content)):
                        target_id = member_ids_by_name.get(name)
                        if target_id is not None and target_id != author_id:
                            mention_targets.append((target_id, cid, user_display[author_uname], content, created_at))
                return mid

            def add_reactions(mid: str, created_at: datetime, count: int) -> None:
                for emoji in rng.sample(REACTIONS_POOL, k=min(count, len(REACTIONS_POOL))):
                    for r_uname in rng.sample(all_active_users, k=min(len(all_active_users), rng.randint(1, 5))):
                        cur.execute(
                            """INSERT OR IGNORE INTO MessageReactions (MessageId, UserId, Emoji, CreatedAt)
                               VALUES (?, ?, ?, ?)""",
                            (mid, user_ids[r_uname], emoji, fmt_dt(created_at + timedelta(minutes=rng.randint(1, 35)))),
                        )

            for cname, script in CONVERSATIONS.items():
                cid = channel_ids[cname]
                filler_n = EXTRA_FILLER_TARGETS.get(cname, 10)
                hist_start = history_floor + timedelta(hours=1, minutes=30)
                hist_end = now - timedelta(hours=25)
                if hist_end <= hist_start:
                    hist_start = now - timedelta(hours=46)
                    hist_end = now - timedelta(hours=25)
                hist_span = max(60, int((hist_end - hist_start).total_seconds()))

                prev_ids: list[str] = []
                for f_i in range(filler_n):
                    t = hist_start + timedelta(seconds=int(hist_span * (f_i + 1) / (filler_n + 1)))
                    au = all_active_users[(f_i + len(cname)) % len(all_active_users)]
                    a, b = FILLER_EXCHANGES[(f_i + len(cname)) % len(FILLER_EXCHANGES)]
                    txt = a if f_i % 2 == 0 else b
                    rep = prev_ids[-1] if (f_i > 1 and f_i % 5 == 0) else None
                    ed = (t + timedelta(minutes=3)) if (f_i % 9 == 0) else None
                    mid = insert_msg(cid, au, txt, t, edited_at=ed, reply_to_id=rep, track_mentions=False)
                    prev_ids.append(mid)
                    if f_i % 4 == 0:
                        add_reactions(mid, t, rng.randint(1, 2))

                base_t = now - timedelta(hours=24)
                step = timedelta(minutes=int((23 * 60) / max(len(script), 1)))
                recent_ids: list[str] = []
                for i, item in enumerate(script):
                    raw_author, raw_text = item[0], item[1]
                    attachment_key = item[2] if (include_media and len(item) > 2) else None
                    author_u, text = resolve_actor_and_text(raw_author, raw_text, ctx, user_ids)
                    t = base_t + step * i + timedelta(seconds=rng.randint(10, 45))
                    if t >= now - timedelta(minutes=3):
                        t = now - timedelta(minutes=3 + len(script) - i)
                    reply_id = recent_ids[-2] if (i >= 3 and i % 4 == 2) else None
                    edited = (t + timedelta(minutes=3)) if (i % 5 == 3) else None
                    pinned = (t + timedelta(minutes=5)) if (i == 0 or (len(script) > 6 and i == len(script) - 3)) else None
                    mid = insert_msg(
                        cid,
                        author_u,
                        text,
                        t,
                        edited_at=edited,
                        pinned_at=pinned,
                        reply_to_id=reply_id,
                        attachment=attachment_key,
                    )
                    recent_ids.append(mid)
                    if i % 2 == 0:
                        add_reactions(mid, t, rng.randint(1, 3))

            # #qa-long-history: 180 messages spanning multiple calendar days for scroll/pagination testing
            long_cid = channel_ids["qa-long-history"]
            long_start = min(history_floor + timedelta(hours=1), now - timedelta(hours=47))
            long_span = max(1800, int((now - timedelta(minutes=6) - long_start).total_seconds()))
            long_prev: list[str] = []
            for n in range(1, 181):
                t = long_start + timedelta(seconds=int(long_span * n / 180))
                au = all_active_users[n % len(all_active_users)]
                txt = LONG_HISTORY_LINES[n % len(LONG_HISTORY_LINES)].format(n=n)
                if n == 179:
                    target_tag = ctx.viewer_username or ctx.owner_username
                    if au == target_tag:
                        au = "priya"
                    txt = f"@{target_tag} Check out the latest scroll & pagination test in #qa-long-history (#{n})!"
                rep = long_prev[-3] if (len(long_prev) >= 3 and n % 7 == 0) else None
                pin = (t + timedelta(minutes=1)) if n in (1, 90, 178) else None
                ed = (t + timedelta(minutes=2)) if (n % 17 == 0) else None
                mid = insert_msg(long_cid, au, txt, t, edited_at=ed, pinned_at=pin, reply_to_id=rep)
                long_prev.append(mid)
                if n % 5 == 0:
                    add_reactions(mid, t, rng.randint(1, 2))

            # 6. ChannelReads & UserNotifications
            viewer_targets = [ctx.owner_id]
            if ctx.viewer_id and ctx.viewer_id != ctx.owner_id:
                viewer_targets.append(ctx.viewer_id)

            for vid in viewer_targets:
                for cname, cid in text_channels:
                    last_t = latest_msg_at.get(cid)
                    read_at = (last_t - timedelta(hours=3)) if (cname in UNREAD_CHANNEL_NAMES and last_t) else now
                    # A marker the account already has (in #general) stays,
                    # since --reset could not give the old one back.
                    cur.execute(
                        "INSERT OR IGNORE INTO ChannelReads (ChannelId, UserId, LastReadAt) VALUES (?, ?, ?)",
                        (cid, vid, fmt_dt(read_at)),
                    )

            for uname, _, _, _, _, _ in USERS_SPEC[:8]:
                uid = user_ids[uname]
                for _, cid in text_channels:
                    cur.execute(
                        "INSERT OR IGNORE INTO ChannelReads (ChannelId, UserId, LastReadAt) VALUES (?, ?, ?)",
                        (cid, uid, fmt_dt(now - timedelta(hours=rng.randint(0, 8)))),
                    )

            # The inbox is ordered by insertion, so entries go in by time.
            mention_targets.sort(key=lambda target: target[4])
            for idx, (target_uid, cid, author_disp, content, created_at) in enumerate(mention_targets):
                preview = " ".join(content.split())
                if len(preview) > 220:
                    preview = preview[:217] + "…"
                read_at = fmt_dt(created_at + timedelta(minutes=10)) if idx % 4 == 3 else None
                cur.execute(
                    """INSERT INTO UserNotifications (
                        Id, ServerId, UserId, Source, SourcePackageId, SourcePackageName,
                        SourceModuleId, SourceModuleName, Title, Body, Tone,
                        TargetChannelId, TargetInviteCode, CreatedAt, ReadAt
                    ) VALUES (?, ?, ?, 'system', 'stellarr.mention', 'Stellarr', 'mention', ?, ?, ?, 'info', ?, NULL, ?, ?)""",
                    (
                        new_guid(),
                        main_server_id,
                        target_uid,
                        author_disp,
                        f"{author_disp} mentioned you",
                        preview,
                        cid,
                        fmt_dt(created_at),
                        read_at,
                    ),
                )

            # 7. Friendships, FriendRequests & DirectConversations / DirectMessages
            existing_friend_rows = cur.execute("SELECT UserAId, UserBId FROM Friendships").fetchall()
            friend_pairs: set[tuple[str, str]] = {(r[0], r[1]) for r in existing_friend_rows}

            def add_friendship(u1: str, u2: str, days_ago: int) -> None:
                a, b = pair_ids(user_ids[u1], user_ids[u2])
                if (a, b) in friend_pairs or a == b:
                    return
                friend_pairs.add((a, b))
                cur.execute(
                    "INSERT INTO Friendships (Id, UserAId, UserBId, CreatedAt) VALUES (?, ?, ?, ?)",
                    (new_guid(), a, b, fmt_dt(now - timedelta(days=days_ago))),
                )

            owner_friends = ["maya", "theo", "priya", "jonas", "kenji", "ola", "lena", "mateusz"]
            for i, f in enumerate(owner_friends):
                add_friendship(ctx.owner_username, f, 22 - (i % 15))

            if ctx.viewer_username:
                viewer_friends = ["maya", "theo", "priya", "jonas", "ola", "kuba", "zuzanna", "marek", "kasia", "piotr"]
                for i, f in enumerate(viewer_friends):
                    add_friendship(ctx.viewer_username, f, 20 - (i % 15))

            seeded_names_only = [u[0] for u in USERS_SPEC]
            for _ in range(30):
                u1, u2 = rng.sample(seeded_names_only, 2)
                add_friendship(u1, u2, rng.randint(2, 25))

            pending_reqs = [
                ("viktor", ctx.owner_username),
                ("amara", ctx.owner_username),
                (ctx.owner_username, "claire"),
            ]
            if ctx.viewer_username:
                pending_reqs.extend([
                    ("tomek", ctx.viewer_username),
                    ("natasha", ctx.viewer_username),
                    (ctx.viewer_username, "kenji"),
                    (ctx.viewer_username, "lena"),
                ])
            for sender, receiver in pending_reqs:
                sid, rid = user_ids[sender], user_ids[receiver]
                a, b = pair_ids(sid, rid)
                if (a, b) in friend_pairs or sid == rid:
                    continue
                pk = pair_key(sid, rid)
                existing_req = cur.execute("SELECT 1 FROM FriendRequests WHERE PairKey = ?", (pk,)).fetchone()
                if existing_req:
                    continue
                cur.execute(
                    """INSERT INTO FriendRequests (Id, SenderId, ReceiverId, PairKey, CreatedAt)
                       VALUES (?, ?, ?, ?, ?)""",
                    (new_guid(), sid, rid, pk, fmt_dt(now - timedelta(hours=rng.randint(1, 36)))),
                )

            used_dm_pairs: set[tuple[str, str]] = set()
            for raw_u1, raw_u2, leave_unread_for_first, msgs in DM_SCRIPTS:
                if raw_u1 == "viewer" and not ctx.viewer_username:
                    # Without a separate non-owner viewer, redirect viewer DMs to the owner so inbox stays rich.
                    u1 = ctx.owner_username
                    dm_role_map = {"viewer": ctx.owner_username, "owner": ctx.owner_username}
                else:
                    u1 = ctx.viewer_username if raw_u1 == "viewer" else (ctx.owner_username if raw_u1 == "owner" else raw_u1)
                    dm_role_map = {
                        "viewer": ctx.viewer_username or ctx.owner_username,
                        "owner": ctx.owner_username,
                    }

                u2 = dm_role_map.get(raw_u2, raw_u2)
                if u1 == u2:
                    continue
                uid1, uid2 = user_ids[u1], user_ids[u2]
                a, b = pair_ids(uid1, uid2)
                if (a, b) in used_dm_pairs:
                    continue
                used_dm_pairs.add((a, b))

                conv_id = new_guid()
                # Encryption is off, as for every conversation the API opens:
                # turning it on takes account keys both people make in their
                # browsers, which a seed cannot. Plain messages and plain
                # attachments are what an unencrypted conversation holds.
                cur.execute(
                    """INSERT INTO DirectConversations (
                        Id, UserAId, UserBId, CreatedAt, EncryptionState,
                        EncryptionRequestedById, EncryptionRequestedAt, EncryptionEnabledAt, CurrentEpochId
                    ) VALUES (?, ?, ?, ?, 'Off', NULL, NULL, NULL, NULL)""",
                    (conv_id, a, b, fmt_dt(now - timedelta(days=5))),
                )
                base_t = now - timedelta(hours=8)
                last_dm_t = base_t
                for i, item in enumerate(msgs):
                    raw_author, raw_text = item[0], item[1]
                    attachment_key = item[2] if (include_media and len(item) > 2) else None
                    author_u = dm_role_map.get(raw_author, raw_author)
                    txt = raw_text.replace("__VIEWER__", viewer_uname_for_mentions).replace("__OWNER__", ctx.owner_username)
                    t = base_t + timedelta(minutes=25 * i)
                    if t >= now - timedelta(minutes=2):
                        t = now - timedelta(minutes=2 + len(msgs) - i)
                    last_dm_t = t
                    mid = new_guid()
                    cur.execute(
                        """INSERT INTO DirectMessages (
                            Id, ConversationId, AuthorId, Content, CreatedAt,
                            EncryptedEpochId, EncryptedIv, EncryptedCiphertext, SystemKind
                        ) VALUES (?, ?, ?, ?, ?, NULL, NULL, NULL, NULL)""",
                        (mid, conv_id, user_ids[author_u], txt, fmt_dt(t)),
                    )
                    if attachment_key is not None:
                        add_attachment(mid, author_u, attachment_key, t, direct=True)
                read_t_1 = (last_dm_t - timedelta(minutes=40)) if leave_unread_for_first else now
                for uid_r, rt in [(uid1, read_t_1), (uid2, now)]:
                    cur.execute(
                        """INSERT INTO DirectConversationReads (ConversationId, UserId, LastReadAt)
                           VALUES (?, ?, ?)""",
                        (conv_id, uid_r, fmt_dt(rt)),
                    )

            # 8. Invites & Audit log entries
            for code, ch_name, uses in INVITES_SPEC:
                cur.execute(
                    """INSERT INTO ServerInvites (Id, ServerId, CreatedById, ChannelId, Code, Uses, MaxUses, ExpiresAt, CreatedAt)
                       VALUES (?, ?, ?, ?, ?, ?, 25, ?, ?)""",
                    (
                        new_guid(),
                        main_server_id,
                        ctx.owner_id,
                        channel_ids[ch_name],
                        code,
                        uses,
                        fmt_dt(now + timedelta(days=7)),
                        fmt_dt(now - timedelta(days=2)),
                    ),
                )

            for i, (raw_actor, summary) in enumerate(AUDIT_SPEC):
                actor_u = ctx.owner_username if raw_actor == "owner" else raw_actor
                cur.execute(
                    """INSERT INTO ServerAuditEntries (Id, ServerId, ActorId, Summary, CreatedAt)
                       VALUES (?, ?, ?, ?, ?)""",
                    (
                        new_guid(),
                        main_server_id,
                        user_ids[actor_u],
                        summary,
                        fmt_dt(now - timedelta(hours=36 - i * 3)),
                    ),
                )

            # 9. Compact secondary servers for server-rail switching.
            # Their JoinedAt must strictly exceed max_primary_join so BootstrapEndpoints
            # continues opening the primary server by default.
            extra_base = max(max_primary_join + timedelta(minutes=5), now - timedelta(hours=18))
            if extra_base >= now - timedelta(minutes=1):
                extra_base = max_primary_join + timedelta(seconds=10)

            for s_idx, (sname, sdesc, is_pub, member_keys, sec_defs) in enumerate(EXTRA_SERVERS):
                sid = new_guid()
                screated = extra_base + timedelta(seconds=s_idx * 30)
                cur.execute(
                    """INSERT INTO Servers (Id, Name, Description, IsPublic, IconUrl, CreatedAt, SoundboardEveryoneCanPlay, SoundboardRateLimited)
                       VALUES (?, ?, ?, ?, NULL, ?, 1, 1)""",
                    (sid, sname, sdesc, is_pub, fmt_dt(screated)),
                )
                cur.execute(
                    """INSERT INTO ServerAdminSettings (ServerId, AllMessages, MentionsOnly, VoiceJoinSound, MutedChannelIdsJson, ServerMuted)
                       VALUES (?, 0, 1, 1, '[]', 0)""",
                    (sid,),
                )
                own_rid, mem_rid = new_guid(), new_guid()
                cur.execute(
                    """INSERT INTO ServerRoles (Id, ServerId, Name, Color, Permissions, Position, IsSystem, IsOwner)
                       VALUES (?, ?, 'Owner', 'oklch(0.72 0.16 15)', ?, 0, 1, 1)""",
                    (own_rid, sid, OWNER_PERMISSIONS),
                )
                cur.execute(
                    """INSERT INTO ServerRoles (Id, ServerId, Name, Color, Permissions, Position, IsSystem, IsOwner)
                       VALUES (?, ?, 'Member', 'oklch(0.70 0.02 277)', ?, 1, 1, 0)""",
                    (mem_rid, sid, MEMBER_PERMISSIONS),
                )

                resolved_members: list[str] = []
                for mk in member_keys:
                    if mk == "owner":
                        resolved_members.append(ctx.owner_username)
                    elif mk == "viewer":
                        if ctx.viewer_username and ctx.viewer_username not in resolved_members:
                            resolved_members.append(ctx.viewer_username)
                    elif mk not in resolved_members:
                        resolved_members.append(mk)

                for m_idx, mu in enumerate(resolved_members):
                    is_own = mu == ctx.owner_username
                    cur.execute(
                        """INSERT INTO ServerMembers (ServerId, UserId, RoleId, OwnerSlot, JoinedAt)
                           VALUES (?, ?, ?, ?, ?)""",
                        (
                            sid,
                            user_ids[mu],
                            own_rid if is_own else mem_rid,
                            1 if is_own else None,
                            fmt_dt(screated + timedelta(seconds=m_idx + 1)),
                        ),
                    )

                cpos = 0
                for sec_i, (sec_title, ch_defs) in enumerate(sec_defs):
                    extra_sec_id = new_guid()
                    cur.execute(
                        "INSERT INTO Sections (Id, ServerId, Name, Position) VALUES (?, ?, ?, ?)",
                        (extra_sec_id, sid, sec_title, sec_i),
                    )
                    for cname, ckind_s in ch_defs:
                        cid = new_guid()
                        ckind = 0 if ckind_s == "text" else 1
                        cur.execute(
                            """INSERT INTO Channels (
                                Id, ServerId, Name, Kind, Position, Topic,
                                SlowModeSeconds, HideAfterInactivityHours, UserLimit, SectionId
                            ) VALUES (?, ?, ?, ?, ?, ?, 0, 0, 0, ?)""",
                            (cid, sid, cname, ckind, cpos, f"{sname} — #{cname}", extra_sec_id),
                        )
                        cpos += 1
                        if ckind == 0:
                            for m_i in range(5):
                                au = resolved_members[(cpos + m_i) % len(resolved_members)]
                                t = now - timedelta(hours=10 - m_i)
                                txt = (
                                    f"Welcome to **{sname}** (`#{cname}`)!"
                                    if m_i == 0
                                    else FILLER_EXCHANGES[(cpos + m_i) % len(FILLER_EXCHANGES)][m_i % 2]
                                )
                                mid = insert_msg(cid, au, txt, t, track_mentions=False)
                                if m_i % 2 == 0:
                                    add_reactions(mid, t, 1)
                            for vid in viewer_targets:
                                cur.execute(
                                    "INSERT OR IGNORE INTO ChannelReads (ChannelId, UserId, LastReadAt) VALUES (?, ?, ?)",
                                    (cid, vid, fmt_dt(now)),
                                )

        fk_errors = cur.execute("PRAGMA foreign_key_check;").fetchall()
        if fk_errors:
            raise RuntimeError(f"Foreign key violations detected: {fk_errors[:5]}")
        integrity = cur.execute("PRAGMA integrity_check;").fetchone()[0]
        if integrity != "ok":
            raise RuntimeError(f"SQLite integrity check failed: {integrity}")

        con.execute("PRAGMA wal_checkpoint(TRUNCATE);")

        presence_entries = 0
        if include_presence:
            planned_presence = [
                (user_ids[spec[0]], PRESENCE_CYCLE[index % len(PRESENCE_CYCLE)])
                for index, spec in enumerate(USERS_SPEC)
                if PRESENCE_CYCLE[index % len(PRESENCE_CYCLE)] != "offline"
            ]
            write_presence_seed(data_root, planned_presence)
            presence_entries = len(planned_presence)
        else:
            # Leave no stale document behind that would restore statuses for
            # accounts this run did not create.
            try:
                (data_root / PRESENCE_FILE_NAME).unlink()
            except FileNotFoundError:
                pass

        counts = {
            **removed,
            "users": cur.execute("SELECT COUNT(*) FROM Users").fetchone()[0],
            "servers": cur.execute("SELECT COUNT(*) FROM Servers").fetchone()[0],
            "sections_main": cur.execute("SELECT COUNT(*) FROM Sections WHERE ServerId = ?", (main_server_id,)).fetchone()[0],
            "channels_main": cur.execute("SELECT COUNT(*) FROM Channels WHERE ServerId = ?", (main_server_id,)).fetchone()[0],
            "channels_main_text": cur.execute("SELECT COUNT(*) FROM Channels WHERE ServerId = ? AND Kind = 0", (main_server_id,)).fetchone()[0],
            "channels_main_voice": cur.execute("SELECT COUNT(*) FROM Channels WHERE ServerId = ? AND Kind = 1", (main_server_id,)).fetchone()[0],
            "roles_main": cur.execute("SELECT COUNT(*) FROM ServerRoles WHERE ServerId = ?", (main_server_id,)).fetchone()[0],
            "members_main": cur.execute("SELECT COUNT(*) FROM ServerMembers WHERE ServerId = ?", (main_server_id,)).fetchone()[0],
            "messages_total": cur.execute("SELECT COUNT(*) FROM Messages").fetchone()[0],
            "messages_main": cur.execute(
                "SELECT COUNT(*) FROM Messages m JOIN Channels c ON c.Id = m.ChannelId WHERE c.ServerId = ?",
                (main_server_id,),
            ).fetchone()[0],
            "reactions_total": cur.execute("SELECT COUNT(*) FROM MessageReactions").fetchone()[0],
            "attachments": cur.execute("SELECT COUNT(*) FROM ImageAttachments").fetchone()[0],
            "fts_rows": cur.execute("SELECT COUNT(*) FROM MessageSearch").fetchone()[0],
            "friendships": cur.execute("SELECT COUNT(*) FROM Friendships").fetchone()[0],
            "friend_requests": cur.execute("SELECT COUNT(*) FROM FriendRequests").fetchone()[0],
            "dm_conversations": cur.execute("SELECT COUNT(*) FROM DirectConversations").fetchone()[0],
            "direct_messages": cur.execute("SELECT COUNT(*) FROM DirectMessages").fetchone()[0],
            "notifications": cur.execute("SELECT COUNT(*) FROM UserNotifications").fetchone()[0],
            "seed_presence": presence_entries,
        }
        return counts
    finally:
        con.close()


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Seed Stellarr SQLite development database with realistic users, channels, and conversations."
    )
    parser.add_argument(
        "--db",
        type=Path,
        default=Path(DEFAULT_DB_PATH),
        help=f"Path to Stellarr SQLite database file (default: {DEFAULT_DB_PATH}).",
    )
    parser.add_argument(
        "--password",
        default=DEFAULT_SEED_PASSWORD,
        help=f"Password assigned to seeded accounts (default: {DEFAULT_SEED_PASSWORD}).",
    )
    parser.add_argument(
        "--seed",
        type=int,
        default=DEFAULT_RNG_SEED,
        help=f"Deterministic RNG seed (default: {DEFAULT_RNG_SEED}).",
    )
    parser.add_argument(
        "--reset",
        action="store_true",
        help="Remove previously seeded records before inserting fresh data (preserves non-seeded accounts).",
    )
    parser.add_argument(
        "--bootstrap-owner",
        action="store_true",
        help="If the database schema exists with zero servers, create the initial 'Stellarr' server and owner account.",
    )
    parser.add_argument(
        "--owner-username",
        default="admin",
        help="Owner username created when --bootstrap-owner initializes an empty database (default: admin).",
    )
    parser.add_argument(
        "--data-root",
        type=Path,
        default=None,
        help="The API's DataRoot, where files/ and the presence fixture live (default: derived from --db).",
    )
    parser.add_argument(
        "--no-media",
        action="store_true",
        help="Skip seeded screenshots, GIFs and clips (and their object files).",
    )
    parser.add_argument(
        "--no-presence",
        action="store_true",
        help="Skip the development presence fixture that gives seeded accounts an online/idle/dnd state.",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    data_root = args.data_root if args.data_root is not None else infer_data_root(args.db)
    try:
        counts = seed_database(
            args.db,
            password=args.password,
            rng_seed=args.seed,
            reset=args.reset,
            bootstrap_owner=args.bootstrap_owner,
            owner_username=args.owner_username,
            data_root=data_root,
            include_media=not args.no_media,
            include_presence=not args.no_presence,
        )
    except (FileNotFoundError, RuntimeError, sqlite3.Error) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 2

    print("Seed completed successfully:")
    for key, value in counts.items():
        print(f"  {key}: {value}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
