# Servers, channels, and features

One Stellarr instance can run your main server alongside extra servers for side groups (for example, a tabletop campaign or a co-op squad). The first server created during `/setup` is the instance server, and its owner also runs [Stellarr Cockpit](cockpit.md).

## Servers, sections, and channels

Create new servers from the rail on the far left. Inside a server, channels can sit at the top level or inside collapsible sections. Members with `manageChannels` reorder channels and sections by dragging them or pressing `↑` and `↓`.

Text channels support a topic line and per-user slow mode. Deleting a channel removes its messages, attachment links, reactions, link cards, pins, FTS5 search rows, and message journal in the same SQLite transaction. Unreferenced files are removed from `/data/stellarr/files/` once Cockpit's orphan retention (`retention.orphans-days`) passes.

## Import from Discord

Open **Create a server → Import from Discord** in the left rail. The wizard creates a private Stellarr server. It copies the selected public text and voice channels and their categories. You can copy channel structure only, all available message history, or history from a date. With a history range selected, you can include public threads and forum posts. Stellarr keeps the names of imported authors on local profiles. Invite members and set up roles and permissions in Stellarr after the import.

### Connect the bot

1. Create an application in the [Discord Developer Portal](https://discord.com/developers/applications) and enable **Guild Install** under **Installation**. Open **Bot** to get its token. For message history, enable **Message Content Intent** on that page. [Discord bot setup](https://docs.discord.com/developers/quick-start/getting-started)
2. Paste the token into the Stellarr wizard and choose **Find servers**. If the server is missing, use **Add bot to a server**, select the Discord server, and authorize **View Channels** and **Read Message History**. Refresh the list in Stellarr.
3. Choose the source server, channels, and history range. Review the scope and start the import. Stellarr lists channels available to all members. History also requires access to read past messages in those channels.

### Copy photos, videos, and other files

After Stellarr copies the history, open the import report and choose **Import missing attachments**. Enter the same bot token and choose **Start copying files**. Stellarr fetches fresh Discord file URLs and stores supported files on the server it already created. It adds them to their original messages and restores messages that consisted of files alone. [Discord attachment URLs](https://docs.discord.com/developers/reference#signed-attachment-cdn-urls)

The report counts attachment-only messages as skipped until their files are copied. The missing-files count includes those messages and messages whose text already came over. The default per-file limits are 10 MiB for images and 100 MiB for videos. Imported files count toward the importing account’s storage quota, which defaults to 2 GiB, and the instance quota. The operator can change the account limit in **Cockpit → Settings → File storage** before resuming. [Uploads and quotas](../install/configuration.md#uploads-and-quotas)

Stellarr builds link previews as readers open the imported history. A provider must make a preview available for its page link to show an image or animation. Discord uploads are handled by **Import missing attachments**.

### Check progress or resume

The Stellarr API runs one Discord import or attachment job at a time. You can close the wizard while it works. Reopen **Create a server → Import from Discord** to see the latest import. If history stopped, choose **Resume import**. If file copying stopped, choose **Resume copying files**. Enter the bot token again to continue the same import. Keep the bot on Discord until you finish copying files.

## Chat, media, and translation

Messages support Markdown, code blocks, inline replies, and side threads (`ThreadPanel`). From the composer you can post multi-option polls, record a voice message, or pick from 50+ built-in astronaut stickers and custom server emojis (`EmojiPane`). When GIPHY is configured in Cockpit, members can search GIFs or turn external GIPHY requests off in their privacy settings.

Channel headers open panels for pinned messages, shared media, posted links, and FTS5 full-text search. Clicking **Translate** on a Polish or English message (or turning on automatic translation in **Settings → Language**) runs Mozilla's Bergamot WebAssembly engine inside a Web Worker. Models download from your own server, verify against SHA-256 digests, and cache in IndexedDB without character limits or external API calls. See [Localization](../localization.md).

## Voice rooms, 4K streams, and soundboards

Click a voice channel to join, or drag your own row (or another participant, with `moderateMembers`) onto a different voice channel to move in one step.

Screen sharing supports four presets: `Economy` (`720p 15 fps`), `Balanced` (`1080p 30 fps`, default), `Sharp` (`1440p 30 fps`), and `Ultra` (`2160p 60 fps`, `12 Mb/s`), plus a separate `160 kb/s` stereo audio track. Changing presets mid-stream needs no window re-selection, and `stellarr-media` forwards video and screen audio only to people watching the stream.

From the microphone menu in the call dock, pick `Auto` voice quality (`128` down to `16 kb/s` as link health changes) or fixed Opus presets up to `256 kb/s` Studio. Input audio passes through manual mic gain (`-12` to `+24 dB`), local WebAssembly noise suppression (`RNNoise`, `GTCRN`, or `Speex`), an `80 ms` lookahead voice gate, and a voiced-only auto-leveler. Listeners can set `0–200%` volume per friend and pick a playback EQ curve (`Off`, `Natural`, `Studio`, `Broadcast`, or 4-band `Custom`). Keyboard shortcuts for mute, deafen, camera, and disconnect live under **Settings → Keybinds**.

Server soundboards hold trimmed clips (`0.15 s` to `15 s`, non-destructive) with configurable display order, `everyoneCanPlay`, and a `5 s` per-user cooldown (`rateLimited`). Each account can also pick a `5 s` arrival clip (`30 s` cooldown) from any server they belong to; muting join/leave sounds in **Settings → Sounds** silences arrival clips locally.

## Roles and the 15 permissions

Every server has an immutable **Owner** role at the top and an implicit **base role** (`Member`) at the bottom. Custom roles sit between them, where lower position numbers outrank higher ones. A member's highest role sets their rank for moderation and delegation checks.

| Group | Permissions | Scope and behavior |
| --- | --- | --- |
| Superuser | `administrator` | Grants every permission on the server and skips overrides. Server-wide only; still bound by role rank. |
| Visibility | `viewChannel` | Controls whether a channel exists for a member. Without it, all permissions in that channel clear to none. |
| Member actions | `sendMessages`, `attachFiles`, `addReactions`, `connect`, `createInvite` | Posting messages, uploading files or voice messages, adding reactions, joining voice, and creating invites. Enabled on a new server's base role. |
| Management | `manageServer`, `manageChannels`, `manageRoles`, `manageMembers`, `manageMessages`, `moderateMembers`, `manageBans`, `viewAuditLog` | Server settings/emojis/soundboard/invites, channel layout, roles and overrides, member roles/kicks, message moderation, voice kicks/moves, bans, and audit log access. |

Overrides apply per role or per member on sections first and channels second:

1. Start with the union of the member's server-wide role permissions.
2. At the section (if any), apply the base role override (`deny`, then `allow`), then all held role overrides together (union of `deny` removed, then union of `allow` added), then the member override (`deny`, then `allow`).
3. Repeat step 2 at the channel. If `viewChannel` is missing at the end, clear all bits to zero.

> [!NOTE]
> You can only edit, assign, or override roles and members ranked below your top role, and only for permission bits you hold yourself. On the primary instance server, role and override edits require a fresh 5-minute second factor when MFA is enabled.

## Private channels

To restrict a channel or section to specific roles or members:

1. Open **Channel settings → Permissions** (or **Section settings → Permissions**).
2. Select the base role (`Member`) and set **View channel** (`viewChannel`) to **Deny**.
3. Add the target role or member and set **View channel** to **Allow**.

For anyone without `viewChannel`, the channel is omitted from bootstrap payloads, the channel rail, search results, `@mention` alerts, and invite previews, and direct requests naming its ID return `404` (or `"Unknown channel."` / `"Unknown voice room."`) before any `403` check. If someone is already seated in a voice channel when a role, override, channel, or membership change removes their `viewChannel` or `connect` right, `voice::enforce` evicts their seat, closes their SFU session, and sends `VoiceChannelKicked`.

## Friends and optional 1:1 end-to-end encryption

Users can add friends by username, block accounts, chat in 1:1 DMs, and start private voice, camera, and screen-share calls.

> [!WARNING]
> Optional 1:1 end-to-end encryption is experimental and has not been independently audited yet.

1. Under **Settings → Encryption**, set a 6-digit PIN. The browser generates an X25519 account key pair, stores it in IndexedDB, and uploads an encrypted key backup derived from the PIN (`PBKDF2-HMAC-SHA256`, 600,000 iterations; 10 wrong PIN attempts delete the server backup).
2. Click **Enable encryption** in a 1:1 DM. Once the other participant confirms (`off` → `requested` → `on`), both browsers wrap a 256-bit epoch key for every active session device on both accounts (up to 10 devices per account).
3. Messages and attachment metadata are encrypted with AES-256-GCM before upload (`POST /api/uploads/encrypted`), and 1:1 calls encrypt Opus, VP8, and VP9 frames in a WebRTC Encoded Transform worker (`SFrame`).
4. Compare the 60-digit safety code (12 groups of 5 digits) in the DM profile panel out of band to verify both account keys. New or revoked sessions rotate the epoch on the next message or call.
