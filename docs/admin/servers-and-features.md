# Servers, channels, and features

A single Stellarr instance can host your primary community server alongside dedicated servers for side groups (such as a tabletop campaign or co-op squad). The first server created during `/setup` is the **instance server**, and its owner manages the deployment through [Stellarr Cockpit](cockpit.md).

## Servers, sections, and channels

Create additional servers from the far-left server rail. Inside each server, text and voice channels sit at the top level or inside collapsible sections. Members holding `manageChannels` reorder channels and sections via drag-and-drop or `↑` / `↓` keyboard controls.

- **Text channels:** Support a channel topic and per-user slow mode.
- **Cascading deletion:** Deleting a channel removes its messages, attachment links, reactions, link cards, pins, FTS5 search entries, and message journal in a single SQLite transaction. Unreferenced files in `/data/stellarr/files/` are swept once Cockpit's orphan retention (`retention.orphans-days`) expires.

## Chat, drafts, and offline translation

- **Rich messaging:** Markdown, syntax-highlighted code blocks, inline replies, side threads (`ThreadPanel`), multi-option polls, voice messages, 50+ built-in astronaut stickers, and custom server emojis (`EmojiPane`). Unsent text drafts and pending file attachments are restored automatically when reopening a channel or DM.
- **Channel sidebars:** Inspect pinned messages, shared media, posted links, and FTS5 full-text search from the channel header.
- **GIF search:** Optional GIPHY integration (when enabled in Cockpit); users can disable browser GIPHY requests individually in **Settings → Privacy**.
- **Offline in-browser translation:** Clicking **Translate** on an English or Polish message (or enabling automatic translation in **Settings → Language**) runs Mozilla's Bergamot WebAssembly translator inside a local Web Worker. Translation models download from your own instance, verify against SHA-256 digests, and cache in IndexedDB with zero external API calls or character limits. See [Localization](../localization.md).

## Installed web app (PWA)

When served over trusted HTTPS, users can install each Stellarr instance as a standalone desktop or mobile app (**Install Stellarr** beneath the server rail, **Share → Add to Home Screen** on iOS/iPadOS, or **File → Add to Dock** on macOS):

- **Unread app badge:** Syncs the OS dock/launcher badge with unread direct messages and `@mention` notifications across accessible channels.
- **Share target:** Sharing photos, videos, or files from the OS share sheet into an installed Stellarr app opens a picker to drop them straight into a channel or direct message.
- **Deep links & shortcuts:** System push notifications and app-icon shortcuts open the target conversation directly after verifying live server access.

## Voice rooms, 4K60 streams, and soundboards

Click any voice channel to join, or drag your own row (or another participant's row with `moderateMembers`) onto another voice channel to switch rooms atomically.

| Capability | Presets & controls |
| --- | --- |
| **Screen sharing** | `Economy` (`720p 15 fps`), `Balanced` (`1080p 30 fps`, default), `Sharp` (`1440p 30 fps`), and `Ultra` (`2160p 60 fps`, `12 Mb/s`) plus `160 kb/s` stereo audio. Mid-stream preset switches need no window re-selection, and `stellarr-media` forwards video/audio only to active viewers. |
| **Opus voice quality** | `Auto` (`128` down to `16 kb/s` adaptive) or fixed presets up to `256 kb/s` `Studio` (`MicMenu`). |
| **Microphone DSP** | Manual input gain (`-12` to `+24 dB`), local WASM noise suppression (`RNNoise`, `GTCRN`, `Speex`, or `Browser`), `80 ms` lookahead voice gate, and voiced-only `-20 dBFS` auto-leveler. |
| **Listener mix & shortcuts** | Per-participant volume (`0–200%`), receiver noise gate, 4-band playback EQ (`Off`, `Natural`, `Studio`, `Broadcast`, `Custom`), and customizable mute/deafen/camera/disconnect keybinds (**Settings → Keybinds**). |
| **Soundboard & arrival clips** | Trimmed server clips (`0.15–15 s`, `everyoneCanPlay`, `5 s` cooldown via `rateLimited`) plus a personal `5 s` walk-on clip (`30 s` cooldown; silenced locally when join/leave sounds are muted in **Settings → Sounds**). |

## Activities

Instance owners can install static multiplayer mini-apps and watch-party tools from GitHub releases in **Cockpit → Activities** (served on the instance's existing HTTPS address by default, with optional dedicated wildcard hostnames via `STELLARR_ACTIVITY_DOMAIN`; see [Configuration](../install/configuration.md#address-and-tls) and the [Activity SDK](../sdk/getting-started.md)):

- **Shared voice launch:** Start an activity from the voice call bar or join a live activity directly from the channel bar (one active session per installed activity in a voice room).
- **Personal launch:** Activities supporting `personal` mode can also be opened solo outside calls via **Add a server and more → Solo activities** in the left rail.
- **Independent game volume:** Each participant adjusts activity music and sound effects locally (`0–100%`) via the **Game audio** slider beside call volume without affecting voice audio or other players.

---

## Roles and the 15 permissions

Every server has an immutable **Owner** role at the top and an implicit **Member** base role at the bottom. Custom roles sit between them (lower position numbers outrank higher ones), and a member's highest held role defines their moderation and delegation rank.

| Category | Permission bits | Scope & behavior |
| --- | --- | --- |
| **Superuser** | `administrator` | Grants all permissions server-wide and bypasses channel/section overrides. Server-wide only; still bound by role rank hierarchy. |
| **Visibility** | `viewChannel` | Controls whether a channel exists for a member. Missing `viewChannel` clears all effective permissions at that channel to zero. |
| **Member actions** | `sendMessages`, `attachFiles`, `addReactions`, `connect`, `createInvite` | Enabled by default on a new server's base role (`Member`). Covers posting/replying, file & voice uploads, adding reactions, joining voice seats, and creating invites. |
| **Management** | `manageServer`, `manageChannels`, `manageRoles`, `manageMembers`, `manageMessages`, `moderateMembers`, `manageBans`, `viewAuditLog` | Controls server settings/emojis/soundboard/invites, channel layout, roles & overrides, member roles & kicks, message deletion/pins, voice kicks/moves, bans, and audit logs. |
| **Channel-wide calls** | `mentionEveryone` | `@everyone` / `@wszyscy` notifies every member who can view the channel, `@here` / `@obecni` those of them who are online. Off on the base role, overridable per channel and section, implied by `administrator` and the Owner role. Without it the words are plain text. Such a call reaches the inbox, the in-app sound and pop-up, and push, never e-mail; somebody also named by username is told as for any mention. |

### Override resolution order

Effective channel permissions evaluate in three deterministic layers:

1. **Server roles:** Union of the base role permissions and every role held by the member.
2. **Section overrides** (when the channel sits inside a section): Base role (`deny`, then `allow`) → union of held roles (`deny` union cleared, then `allow` union added) → member override (`deny`, then `allow`).
3. **Channel overrides:** Same three sub-steps at the channel. If `viewChannel` is not set at the end, every permission clears to zero.

> [!NOTE]
> Actors can only edit, assign, or override roles and members ranked strictly below their own top role, and only for permission bits they hold themselves. On the primary instance server, role and override edits require a fresh 5-minute second factor when MFA is active.

## Private channels

1. Open **Channel settings → Permissions** (or **Section settings → Permissions**).
2. Select the base role (**Member**) and set **View channel** (`viewChannel`) to **Deny**.
3. Add each authorized role or member and set **View channel** to **Allow**.

Members without `viewChannel` never receive the channel in `/api/bootstrap`, the channel rail, FTS5 search, `@mention` alerts, or invite previews; direct API or hub calls naming its ID return `404` / `"Unknown channel."` before any `403` check. Whenever a permission or membership change removes `viewChannel` or `connect` from someone already in a voice channel, `voice::enforce` immediately evicts their seat and closes their SFU media session (`VoiceChannelKicked`).

---

## Friends and 1:1 end-to-end encryption

Users can add friends by username, block accounts, message in 1:1 DMs, and start private voice, video, and screen-share calls.

> [!WARNING]
> Optional 1:1 end-to-end encryption is experimental and has not been independently audited yet.

1. **Save your recovery code.** Under **Settings → Encryption**, save the generated code outside Stellarr, for example in your password manager. The browser uses it to protect a backup of your X25519 account key. On a new device, sign in and paste the code to recover your encrypted history. If you lose the code, an unlocked browser can issue a replacement. If you lose both, resetting encryption starts a new history.
2. **Mutual activation:** Click **Enable encryption** in a 1:1 DM. Once the peer confirms, updated clients derive each new conversation key from the two account keys. A newly registered device must recover its account key with the recovery code to read or send encrypted messages and join encrypted calls.
3. **Encrypted messages, files, and calls:** DM text and file metadata are sealed with AES-256-GCM before upload (`POST /api/uploads/encrypted`), and 1:1 calls encrypt Opus, VP8, and VP9 frames inside a WebRTC Encoded Transform worker (`SFrame`).
4. **Verification:** Compare the 60-digit safety code (12 groups of 5 digits) in the DM profile panel out of band. Mark the code as matching on each browser where you compared it. If it changes, sending is blocked until you verify the new code. New or revoked sessions trigger rotation on the next message or call.

If your backup uses a six-digit PIN, open an updated browser and follow the migration prompt. On a new device, enter the old PIN first. Save the generated recovery code, then confirm the change. Your existing messages remain readable. Other browsers will need the new code, and contacts who previously verified you will need to compare the new safety code.

If a pre-migration archive contains PIN backups for both people in a DM, both need to migrate. Someone with that archive and the matching instance key could recover either PIN-era account key and read messages sent while that key was still in use. After the second migration, compare the new safety code outside Stellarr before sending sensitive messages.

---

## Import from Discord

Open **Create a server → Import from Discord** in the left server rail to copy selected public text/voice channels, categories, threads, forum posts, message history, and attachments into a new private Stellarr server while preserving original author display names.

### 1. Connect a Discord bot

1. Create an application in the [Discord Developer Portal](https://discord.com/developers/applications), enable **Guild Install** under **Installation**, copy the token under **Bot**, and enable **Message Content Intent** if importing message history.
2. Paste the bot token into Stellarr's import wizard and click **Find servers**. If your server is not listed, click **Add bot to a server** and grant **View Channels** and **Read Message History**.
3. Pick the source server, public channels, and history range (structure only, from a date, or full history with optional public threads/forums), then start the import.

### 2. Copy attachments

Once message history finishes importing, open the import report, choose **Import missing attachments**, and provide the same bot token. Stellarr refreshes [signed Discord CDN URLs](https://docs.discord.com/developers/reference#signed-attachment-cdn-urls), downloads supported attachments (`≤ 10 MiB` images, `≤ 100 MiB` videos), attaches them to their original messages, and restores attachment-only messages.

Imported files count toward the importing user's storage quota (`2 GiB` default; adjustable beforehand in **Cockpit → Settings → File storage**—see [Uploads and quotas](../install/configuration.md#uploads-and-quotas)). Keep the bot in the Discord server until attachment copying completes.

### 3. Monitor or resume

Stellarr runs one Discord import or attachment job at a time in the background—you can close the dialog at any point. Reopen **Create a server → Import from Discord** to view progress or click **Resume import** / **Resume copying files** with your bot token if interrupted.
