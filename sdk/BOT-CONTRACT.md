# Bot contract v1

*Status: proposed for review, checked against `main` at `c120fcaa` (2026-11-27).* This draft defines the contract between a Stellarr instance, a server owner and an external bot. The routes, frames and `@stellarr/sdk/bots` export below describe proposed interfaces. The [activity contract](CONTRACT.md) covers the browser SDK and points here for external bots. The [incoming webhook contract](WEBHOOK-CONTRACT.md) covers channel webhooks and defines the integration-authored message a bot reply is.

> Statements carry a status where it matters: **Decided** (chosen in the review of 2026-11-27), **Proposed** (this revision's recommendation, with the alternative named) or **Open** (a product decision nobody has made yet). Unmarked text is carried over from the original proposal. [Section 9](#9-changes-from-the-first-draft) maps every change to its reason.

## 1. Who runs what

| Participant | Responsibility |
| --- | --- |
| Instance owner | Admits the bot to *Cockpit → Bots*, records its operator, reviews its commands and requested access, and can disable it across the instance. |
| Server owner | Browses admitted bots under *Server settings → Bots*, reviews what leaves the server, selects text channels, installs the bot and can remove it. |
| Server member | Invokes a command in an eligible channel and sees the bot's attributed reply there. |
| Bot operator | Hosts the process, its dependencies, secrets and any application state. This can be the author or a self-hosting instance owner. |
| Stellarr instance | Owns the catalog, installation records, channel permissions, command dispatch, message history and audit trail. |

A catalog entry gets an instance-assigned `appId`. A server installation gets a fresh `installationId` and an author row for its replies ([webhook contract, section 2](WEBHOOK-CONTRACT.md#2-integration-authored-messages)). There is at most one current installation per `(serverId, appId)`. A single process can serve many installations over one connection to the instance. A process serving several instances opens a separately authenticated connection to each.

Installing a bot creates a grant in Stellarr. The operator deploys its code independently, for example as a sibling container on the same VPS. The bot communicates over the public bot protocol and requires neither the Stellarr data volume nor access to the container engine. An approved manifest describes the data and actions Stellarr permits. The operator can change the code behind it, so the operator's identity is part of the approval decision.

## 2. What the owners approve

The author supplies a UTF-8 JSON manifest named `stellarr.bot.json`. The instance owner imports it into Cockpit and pairs the runtime once. Server owners can then install the paired application without handling credentials or deploying software.

```json
{
  "protocol": 1,
  "id": "dice",
  "version": "1.0.0",
  "name": "Dice",
  "description": "Roll dice in a selected channel.",
  "author": { "name": "Dice Club", "url": "https://dice.example" },
  "permissions": ["commands.respond"],
  "commands": [
    {
      "name": "rzut",
      "description": "Roll dice, for example 2d6.",
      "options": [
        { "name": "dice", "description": "Dice expression", "type": "string", "required": true }
      ]
    }
  ]
}
```

- All fields shown are required. An argument-free command uses `"options": []`. The file is limited to 64 KiB. Reject duplicate JSON keys, unknown fields, unsupported protocol versions, permissions and option types before admission.
- `id` is a stable slug unique in the instance bot catalog. Slugs and command/option names match `^[a-z][a-z0-9-]{0,31}$`. Names are unique inside their containing command list or option list. `version` uses SemVer. Cockpit binds each approved version to the SHA-256 digest of the submitted bytes. Resubmitting identical bytes is idempotent. Reusing a version with a different digest is rejected.
- Bot and author names are nonblank, up to 80 UTF-8 bytes. Descriptions are nonblank, up to 240 UTF-8 bytes. The author URL is an HTTPS link shown to both owners. It is informational and carries no credentials.
- **Author and operator (*Proposed*).** The manifest names who wrote the bot (`author`; the first draft called this field `operator`). Who runs the worker is not something the author can state, since section 1 allows the instance owner to self-host it. The instance owner records the operator's name and HTTPS URL in Cockpit at admission, prefilled from `author`, and the install screen shows both when they differ. *Alternative:* keep `operator` in the manifest and accept that a self-hosted bot shows its author as the recipient of the data.
- Declare 1–32 commands, each with up to 8 named options. Option types are `string` (up to 512 UTF-8 bytes), `integer` (JSON safe integer) and `boolean`. The instance rejects missing required values, unexpected arguments and wrong types before dispatch. Omitted optional values are absent from the arguments object.
- The single v1 capability, `commands.respond`, delivers an explicitly invoked command and authorizes one plain-text reply in its channel. The install screen shows the author, the operator, commands, fields sent (section 4), reply visibility and selected channels. Channel selection starts empty. A new installation requires at least one existing text channel on that server.
- **Localization (*Decided*).** The manifest carries one name and one description per item, and command names are ASCII. The language is the author's choice: the instance ships English, Polish and German language packs (`locales/*.json`), and v1 translates nothing a manifest says.
- **Bounds (*Decided*).** The bounds above stand: an `id` of 32 characters, a name of 80 UTF-8 bytes, a description of 240. They are tighter than the activity manifest's (100, 240 and 4,000), because a command list shows each in one line.
- **Import (*Proposed*).** In v1 the instance owner imports a manifest file in *Cockpit → Bots*. Discovery from a GitHub repository is deferred to [Appendix A](#appendix-a-publishing-bots-in-stellarractivities-deferred).

## 3. Approval, updates and revocation

| Action | Effect |
| --- | --- |
| Admit and pair | The instance owner reviews the manifest, records the operator and generates a credential for this application on this instance. The first authenticated worker connection completes pairing. The enabled entry then appears in the catalog and remains visible with an offline status during subsequent disconnects. |
| Install or edit | Only the current server owner can grant access and select channels. Repeating the same install returns the existing installation. Changing channels is an explicit edit. Newly created channels are excluded until selected. |
| Approve an update | Cockpit presents the differences. A change confined to display names, descriptions, the author's details or the version applies to every installation at once. A removed command or option stops being callable at once and needs no consent. Any other change to the manifest (permissions, an added command or option, a changed type or requiredness) leaves each installation on the revision its server owner accepted: it keeps working with that revision's commands until the owner accepts the new revision or removes the installation, or until the limit under *Pinned revisions* runs out. A change of the recorded operator is not pinned and suspends instead (*A changed operator*). |
| Remove a channel or uninstall | Invalidate pending interactions in the affected scope. Uninstall also retires the installation's author row. Reinstall creates a new installation ID and a new author row. Historical replies keep their author and bot label. |
| Disable or remove in Cockpit | Disable hides the entry from new installs, closes its connection and invalidates all pending interactions while preserving grants. Re-enable permits fresh commands under still-valid grants. Removal revokes the credential and retires all installations. |

- **Pinned revisions (*Proposed*).** An installation records the manifest version its owner accepted. The instance validates and dispatches against that version and sends it as `manifestVersion`, so the worker keeps handlers for every version still installed. The differences in the update row are computed per installation, between the revision it is pinned to and the newly approved one, so an installation several versions behind is asked once for everything that changed. The instance keeps a superseded revision while an installation is pinned to it. **A superseded revision stays supported for a number of days that the instance owner sets in Cockpit, 30 by default (*Decided*).** The count starts when the newer revision is approved. When it runs out, the installation exposes no command and is shown to its owner as waiting for the newer revision; accepting it restores the commands. The `ready` frame carries the limit as `revisionGraceDays` (*Proposed*), so an author knows how long this instance needs the old handlers. *Alternative (the first draft):* suspend every installation until its owner accepts, which turns an added optional argument into an outage on every server, and makes `manifestVersion` on the wire redundant.
- **A changed operator (*Decided*).** A change of the recorded operator suspends every installation of the bot at once. It is not pinned: one connection serves every installation, so a pinned installation would send its arguments to a worker whose operator its owner never approved. A suspended installation exposes no command and dispatches nothing, its pending interactions are invalid from that commit on, and it is shown to its owner as waiting for the new operator. Accepting the operator restores what the installation's pinned revision still exposes. The suspension takes effect with no period of grace, and the day count under *Pinned revisions* keeps running through it, so a revision whose limit ran out meanwhile still has to be accepted. *Proposed:* an installation records the operator its owner accepted, as it records the manifest version. Any edit of the recorded name or URL counts as a change, and Cockpit states how many installations the edit suspends before it saves. An edit back to the operator an installation accepted resumes that installation without a new acceptance. *Alternative:* a separate correction edit that suspends nothing, which lets a real hand-over be recorded as a correction. **The change also rotates the credential (*Decided*):** saving it invalidates the old token and closes its connection, as a rotation does (section 5), and Cockpit shows the new credential once for the instance owner to hand to the new operator. Otherwise the previous operator's token would still open the gateway after owners had accepted the new one. An edit back to an earlier operator rotates as well.
- **Empty channel selection (*Proposed*).** When the last selected channel is deleted or deselected the installation stays, exposes no command and is shown to the owner as needing a channel. It is not uninstalled, so its author row and accepted revision survive.
- **Who may manage (*Decided*).** The current server owner only. Holding `manageServer` does not grant it. The same holds for webhooks.
- **Second step (*Proposed*).** Generating or rotating the credential, admitting, changing the recorded operator, disabling and removing in Cockpit, and installing or accepting a revision or a new operator on a server require a fresh second step under the active MFA policy. In Cockpit this is what installing and managing an activity already require (`bind_then_gate(.., true)`, which asks for `FreshInstanceOwner`). A server owner's action asks as the ownership hand-over does, with `fresh::require` in the handler (`administration/ownership.rs`). *Alternative:* no second step for the actions on a server.

Server deletion has the effect of uninstalling every bot there. Ownership transfer gives the new server owner management of existing grants. A channel has no foreign key to its server and deletes cascade through triggers in `database/schema.sql`; the installation, channel-grant and interaction tables need the same triggers. `schema.sql` is the baseline of a new database and is no longer added to: the tables and their triggers are a new entry in `MIGRATIONS` (`database/schema.rs`), applied once per database under `user_version`.

Audit records identify the actor, application, installation where applicable, revision, action and changed channel or permission grants. Cockpit actions go to the operator audit (`OperatorAuditEntries`); installing, editing channels, accepting a revision or a new operator and uninstalling go to the server audit (`ServerAuditEntries`) with the acting member. Command arguments and credentials are excluded from these records.

Restoring a backup restores the catalog, credential verifiers and installations it contains, including ones revoked since (*Proposed*: the post-restore step disables every bot until the instance owner re-enables it).

## 4. Dispatch and reply

1. The member selects a command in a server text channel. The picker shows the bot name and unique catalog slug when two apps declare the same command. Invocation addresses `installationId + command`. The instance derives the actor from the member session. A typed command with an ambiguous name requires a selection. A name shared with one of the client's own commands requires one too (*Decided*, *Client surface* below). Ordinary chat messages do not generate bot events.
2. Before exposing the command, dispatching it, and committing a reply, Stellarr checks the enabled app, current installation, accepted revision and accepted operator, selected channel, and the member's effective `viewChannel` and `sendMessages` in that channel. **The bot holds no permissions of its own (*Proposed*).** The server owner's channel selection is the bot's grant, and the reply is admitted on the invoker's `sendMessages`. A private channel therefore needs only to be selected, and there is no state in which a selected channel refuses the bot's reply. *Alternative (the first draft):* a service identity subject to the base role and section/channel overrides, which today requires an enabled server member ([webhook contract, section 2](WEBHOOK-CONTRACT.md#2-integration-authored-messages)).
3. The runtime gets the IDs, command name, typed arguments and expiry shown below. The actor ID is opaque and stable within that installation. The member's profile, role list, other messages, DMs and voice data are outside this event. Submitting a command discloses its arguments to the displayed operator.
4. The only valid destination for a reply is the original channel, derived by Stellarr from the interaction. The reply is an integration-authored message ([webhook contract, section 2](WEBHOOK-CONTRACT.md#2-integration-authored-messages)): the instance assigns authorship to the installation's author row, labels it as a bot and applies normal channel visibility to the stored message. Reply text cannot trigger mentions, embeds or attachment fetches. Success appears to channel members. Unavailability, denial, a decline by the bot and timeout are reported privately to the caller.
5. **Invoker attribution (*Proposed*).** The bot never learns who the actor is, and audit records exclude arguments, so the instance attributes the reply itself: the stored message records the invoking member and the command name, and the client shows "*member* used `/rzut`" above the reply. Without this, a command that echoes its argument lets a member post through the bot anonymously. **The arguments are neither shown nor stored (*Decided*):** the line names the member and the command only, so what was typed stays out of history and backups.
6. **Slow mode (*Proposed*).** An accepted invocation counts as the invoker's post for the channel's slow mode: it is refused while they must wait and it records their post time, with the same exemption for `manageChannels`. The post-time trigger fires on a stored message and names its author, so the acceptance transaction writes the invoker's post time itself. Slow mode is counted per author, so otherwise a member bypasses it through the bot. *Alternative:* rely on the invocation rate limits in section 5 alone.

**Revocation.** Authorization is checked in the transaction that accepts an invocation and again in the transaction that stores the reply (*Proposed*). SQLite has one writer (the pool keeps a single writer connection, `database/mod.rs`), so a reply commit is ordered against every change to membership, roles, overrides and grants: after a revocation commits, no reply commits in the revoked scope. A member's post now goes through `PostQueue` (`messages/post/batch.rs`), which stores up to four waiting posts in one transaction, each under its own savepoint. That leaves the ordering as it was. A reply has to mark its interaction answered in the transaction that stores the message, so it takes a write of its own and does not join the queue (*Decided*). A previously committed message stays in history. Arguments already sent to the operator cannot be recalled, and a revocation that commits between the acceptance transaction and the socket write does not stop that one frame.

Uninstalling, deselecting or deleting the channel, disabling the app, changing its recorded operator and rotating its credential mark the affected pending interactions invalid in the same transaction, and restoring the grant never revives them. **Changes to membership, roles and overrides invalidate at once (*Decided*):** an interaction whose invoker loses access is invalid from that commit on, and restoring the access inside the 15-second window does not revive it.

**Mechanism (*Proposed*).** Each server has an access generation. Triggers on every table that authorization reads (membership, roles and overrides among them) raise it, so the rule holds whichever code makes the change. An interaction records the generation at acceptance, and its reply commits only while the two are equal. A change of access on a server therefore invalidates every interaction pending there, including those it did not affect; their callers get the private failure and invoke again. *Alternative:* re-evaluate the pending interactions in every access mutation (the call sites `voice::enforce` has). That invalidates only the affected ones, and the list of call sites has to be kept complete by hand.

**Client surface ([Appendix C](#appendix-c-client-surface-preliminary)).** The web client has a command list since `835110bd`. A registry (`web/src/stellarr/components/room/composer/commandRegistry.ts`) holds the client's own commands: file, gif, voice, code, spoiler, pinned, status, kick and ban. The composer's plus opens the list, and so does a slash at the start of the draft. A command with arguments takes the field, one chip per argument, of kind `text` or `person`. Every entry opens something the client already does, and none is sent to the server as a command.

This contract fixes the semantics. Appendix C proposes the routes, the events and the way bot commands enter that registry.

**Names shared with the client's own commands (*Decided*).** A built-in command answers to its name in the reader's language and to its English name, compared without case and without diacritics. A manifest's ASCII name can therefore match one (`status`, `code`, `kod`, `wyrzuc`), and which names match depends on the reader's language. A typed name that matches more than one command, built-in or a bot's, runs nothing by itself: the list shows every match, a bot's under the bot's name and slug, and the member picks one. No manifest is refused for its names, so a language pack or a built-in command added later cannot break an admitted bot.

A slash word that finds no command is sent as typed, so a client released before bot commands sends `/rzut 2d6` as an ordinary message.

## 5. Runtime wire protocol

The bot opens `wss://<instance>/hubs/bots` with WebSocket subprotocol `stellarr.bot.v1` and the HTTP upgrade header `Authorization: Bearer <bot-token>`. Outbound connection from the runtime lets an operator run behind NAT. The instance never connects to a URL supplied in the manifest.

- **Path (*Proposed*).** The bundled Nginx forwards WebSocket upgrades only under `/hubs/`; its `/api/` location is plain HTTP and clears `Connection`. The first draft's `/api/bots/gateway` would need a new proxy location in Nginx and in every operator's own reverse proxy. *Alternative:* keep that path and add the location.
- **No Origin (*Proposed*).** The member hub at `/hubs/app` refuses an upgrade without an allowed `Origin`. The bot endpoint is a separate mount: it authenticates by the bearer credential alone, takes no cookies and ignores `Origin`. A browser page cannot set `Authorization` on a WebSocket upgrade, so the credential requirement already keeps pages out.
- **Liveness.** The instance sends a WebSocket Ping every 20 seconds and closes a connection that has been silent for 45 seconds, the member hub's values. Both fit the proxy's 75-second read timeout.

The instance generates an opaque high-entropy credential, shows it to the instance owner once and stores only its verifier. The owner provisions the bot operator through a private channel. The SDK verifies the instance's TLS identity, sends the credential only to the configured origin and rejects redirects. The credential is valid exclusively on the bot gateway. Rotation immediately invalidates the old token, its connection and outstanding interactions. A change of the recorded operator is a rotation too (section 3). The ordinary chat client and server installation flow never receive this credential.

One authenticated connection per `appId` per instance is active at a time. An invalid or disabled credential fails the upgrade with HTTP 401. **A newer connection supersedes the older one (*Proposed*):** the instance accepts it and closes the old socket with code 4001 (`superseded`). A restarted worker is therefore never locked out by a half-open socket that Ping/Pong has not yet detected, and a rolling deploy needs no gap. The SDK treats 4001 as terminal and does not reconnect, so two misconfigured workers cannot take the slot from each other in a loop. *Alternative (the first draft):* refuse the additional connection with HTTP 409, which then has to be specified as retryable. The server releases the slot on disconnect. The SDK reconnects after transport loss with bounded exponential backoff and jitter. Authentication failures require operator intervention.

Each frame is a UTF-8 JSON object in a WebSocket text message, up to 32 KiB. IDs are opaque strings. Times are UTC RFC 3339 strings assigned by the instance. Each invocation identifies the manifest version the installation is pinned to so a worker can select the corresponding handler. The following exchange shows one complete command.

*Instance → bot*, once after a successful upgrade

```json
{
  "type": "ready",
  "instanceId": "i1",
  "appId": "a7",
  "serverTime": "2026-11-26T11:59:40Z",
  "pingIntervalMs": 20000,
  "maxFrameBytes": 32768,
  "revisionGraceDays": 30
}
```

*Instance → bot*, when a member invokes `/rzut 2d6`

```json
{
  "type": "command.invoke",
  "interactionId": "q9",
  "installationId": "n4",
  "serverId": "s2",
  "channelId": "c8",
  "actorId": "u3",
  "manifestVersion": "1.0.0",
  "name": "rzut",
  "args": { "dice": "2d6" },
  "expiresAt": "2026-11-26T12:00:15Z",
  "ttlMs": 15000
}
```

*Bot → instance*, then *instance → bot* after the message commits

```json
{ "type": "interaction.reply", "interactionId": "q9", "text": "2d6 → 8" }
```

```json
{ "type": "interaction.result", "interactionId": "q9", "messageId": "m5" }
```

*Bot → instance*, instead of a reply, when the command cannot be answered (*Proposed*)

```json
{ "type": "interaction.decline", "interactionId": "q9", "reason": "Use a form like 2d6." }
```

```json
{ "type": "interaction.result", "interactionId": "q9", "declined": true }
```

- **Forward compatibility (*Proposed*).** A bot ignores frame types and fields it does not know; the instance may add either within `stellarr.bot.v1`. The instance is strict towards the bot: unreadable envelopes and unknown frame types close the socket with 1002, oversized messages with 1009. `ttlMs` is the time left when the frame was written, for a worker whose clock differs from the instance's; `expiresAt` is authoritative on the instance.
- A reply contains 1–2,000 UTF-16 units of text that is nonblank after trimming (*Proposed*; the first draft said UTF-8 bytes, which is not the measure ordinary messages use). The cap is this contract's own: an ordinary message holds up to 20,000 units, and up to 100,000 when the excess is fenced code (`messages/mod.rs`). The instance binds each interaction to the authenticated application, installation, channel and invoker. A bot-supplied destination or author is an invalid frame.
- **Decline (*Proposed*).** A decline ends the interaction without a message. The caller sees a private notice at once, instead of waiting out the window or reading an error the whole channel sees. `reason` is optional, up to 200 UTF-16 units, shown only to the caller as literal text attributed to the bot, and never stored. *Alternative:* no decline frame.
- An invocation has a 15-second window starting at acceptance. Each newly accepted invocation is dispatched at most once. Offline or saturated bots fail immediately. Disconnecting does not queue or replay commands. A worker that still has the ID and computed text can reconnect and retry its reply.
- **Interaction record (*Proposed*).** Accepting an invocation stores one row: installation, channel, invoker, request ID, command, expiry and state (`pending`, `answered`, `declined`, `invalid`). An answered row keeps the message ID and a digest of the reply text. Every guarantee below reads this row, which is why they hold across reconnects and instance restarts. Housekeeping deletes rows once their receipt window has passed. *Alternative:* reuse the `clientMessageId` replay of an ordinary send; it forgets the reply when a moderator deletes the message, so a retry would post it again.
- The client submits a unique request ID for each intentional invocation. Within five minutes, a still-authorized repeat by the same actor for the same installation, channel and request ID returns the original invocation status, including after instance restart. A changed command or arguments with that key conflicts. Two intentional rolls use different keys.
- The instance atomically stores the message and marks the interaction answered. While still authorized, an identical reply retry within five minutes of commit returns the same `messageId`, including after reconnect or instance restart, and also when the message has since been deleted. A different text conflicts. A first reply arriving at or after expiry fails. After the receipt window, the ID cannot produce a new message.
- A rejected reply or decline receives `{"type":"interaction.error","interactionId":"q9","code":"expired"}`. Codes are `invalid_reply` (bad text or extra fields), `unknown_interaction` (unknown ID or another app's ID), `access_revoked`, `expired`, `conflict` (a different text, or a reply after a decline) and `rate_limited`. A rate limit includes a positive `retryAfterMs`. A retry is useful only while its applicable window is open.
- Stellarr bounds pending work and invocation rates per member and installation. Capacity exhaustion returns a retry delay to the invoking client and never enqueues work for later delivery. A handler that neither replies nor declines produces a timeout for the caller.
- **Limits (*Decided* for now).** 5 invocations per 10 seconds per member, 64 pending interactions per installation, 256 per application. The numbers are a starting point, to be revisited once there is something to measure.

## 6. Author SDK

The proposed server-side entry point is `@stellarr/sdk/bots`. It maps typed command arguments to an author handler, submits its returned text, and resolves `close()` when the connection is closed. The transport owns reconnection and reply retries. A retry reuses the handler's computed reply. For dice, this preserves the original roll.

**Package (*Decided*).** The entry is a `./bots` export of the existing `@stellarr/sdk`. Version 0.3.0 exports only `.`, the browser client for activities, beside the `stellarr` CLI, so this adds an export and files in `sdk/package.json`.

```js
import { connectBot, decline } from "@stellarr/sdk/bots";
import { rollDice } from "./game.js";

const bot = await connectBot({
  instance: process.env.STELLARR_URL,
  token: process.env.STELLARR_BOT_TOKEN,
  commands: {
    rzut: ({ args }) => rollDice(args.dice) ?? decline("Use a form like 2d6.")
  }
});
```

Here `rollDice(expression)` returns display text, or nothing for an expression it cannot read. A handler may also return a promise of that text. Its context exposes the fields of `command.invoke`. A handler that returns `decline(reason)` sends `interaction.decline`. A handler that throws sends a decline with no reason and reports the error to the operator's log (*Proposed*), so a bug costs the caller no 15-second wait.

The SDK is optional: an operator can implement section 5 directly. Porting an existing Discord bot is described in [Appendix B](#appendix-b-discord-portability).

## 7. Acceptance cases

| Exercise | Observable result |
| --- | --- |
| Admit Dice, install on server A in `#games`, invoke `/rzut 2d6` | One invocation reaches the worker and one attributed reply appears in `#games`, headed by who used `/rzut`. The worker receives only the fields in section 5. |
| Install Dice on server B as well | The same connection serves two distinct installation IDs. Removing A leaves B operational. |
| Try a non-owner install, a command in an unselected channel, or a forged destination | Management and dispatch are denied as applicable. The forged reply produces no message. |
| Remove the grant, delete the channel, or revoke the user's channel access while the handler is running | The late reply cannot commit. Restoring the grant or the access leaves the old interaction invalid. |
| Type `/status` on a server where Dice also declares `status` | Nothing runs. The list shows the client's command and Dice's, and the member picks one. |
| Leave an approved update unaccepted past the limit set in Cockpit | The installation exposes no command and is flagged for its owner. Accepting the revision restores its commands. |
| Invoke in a private channel the owner selected | The reply commits. No role or override names the bot. |
| Reply with `@ada https://example.com` | Ada gets no inbox entry, e-mail or push, and no preview card is stored. |
| Invoke `/rzut` with an expression the bot cannot read | The worker declines. The caller alone sees the reason at once. The channel gets no message. |
| Invoke twice in a slow-mode channel | The second invocation is refused with the wait, as a second message would be. |
| Drop the socket after a reply commits but before the receipt arrives | Retrying the same computed text yields the first message ID. The channel history contains one roll. |
| Delete the reply as a moderator, then retry the same reply | The retry yields the first message ID. Nothing is posted again. |
| Repeat an invocation with the same request ID, restarting the instance in between | The original status is returned. The worker sees one `command.invoke`. |
| Start a second worker with the same credential | It becomes the active connection. The first is closed with 4001 and its SDK does not reconnect. |
| Open the gateway without a credential, with a member's session cookie, or through the bundled Nginx with a valid credential | 401, 401, and a working connection that survives two minutes idle. |
| Approve an update that adds an argument | Server A, which has not accepted it, keeps working with `manifestVersion` 1.0.0. Server B accepts and gets 1.1.0. Both are served over one connection. |
| Approve an update that only changes a description | Every installation shows it at once and stays installed. |
| Delete the last selected channel | The installation remains, exposes no command and is flagged for the owner. |
| Change the recorded operator in Cockpit while a handler is running, with Dice on servers A and B | The connection closes with its credential revoked, and reconnecting with it answers 401. Both installations expose no command and the late reply cannot commit. A worker that connects with the new credential receives nothing yet. A's owner accepts the new operator and gets the commands back. B stays suspended and the worker receives nothing from it. |
| Disable Dice in Cockpit while a handler is running | The connection closes, the late reply cannot commit, and re-enabling does not revive it. |
| Stop the worker and open the command list in `#games` | Dice's commands are absent. They return when the worker reconnects, without a reload. |
| Exceed the invocation rate | The caller gets a retry delay. The worker receives nothing for the refused invocations. |

Further contracts can add proactive messages, passive events, moderation, private responses and voice/music. Each needs an explicit capability, its own data disclosure and revocation rules. The command-only grant above authorizes none of those operations.

## 8. Channel webhooks

Moved to the [incoming webhook contract](WEBHOOK-CONTRACT.md). It is the proposed first use case and has no dependency on this document; this document depends on its section 2.

## 9. Changes from the first draft

| Change | Section | Reason |
| --- | --- | --- |
| Bot identity is an author row, and the bot holds no permissions | 1, 3, 4.2, [webhook 2](WEBHOOK-CONTRACT.md#2-integration-authored-messages) | `Messages.AuthorId` is a `RESTRICT` foreign key to `Users`, and `permissions::authorization` gives nothing to anybody who is not an enabled member. A member-like identity would appear in member lists, mentions, roles, moderation and ownership. |
| Gateway at `/hubs/bots`, no `Origin`, stated ping timing | 5 | `infra/nginx.conf` forwards upgrades only under `/hubs/`; `realtime/origin.rs` requires an `Origin`. |
| Reply shows who invoked which command | 4.5 | The bot cannot name the actor and audit omits arguments, so replies were anonymous. |
| `interaction.decline` | 5, 6 | A bad argument otherwise costs a public error message or a 15-second timeout. |
| Mention, preview and formatting rules live on the message | 4.4, [webhook 2](WEBHOOK-CONTRACT.md#2-integration-authored-messages) | Server and client derive mentions, links and previews from the text alone today. |
| One persisted interaction row; access re-checked at reply commit | 4, 5 | Restart-proof receipts need stored state regardless. Eager invalidation on role and override changes was left *Open* then and is *Decided* now. |
| Installations pinned to the accepted revision | 3, 5 | Suspending on every schema change is an outage per update and made `manifestVersion` redundant. |
| `author` in the manifest, operator recorded in Cockpit | 2 | A self-hosted bot's operator is the instance owner, not the author. |
| A newer connection supersedes the older | 5 | A half-open socket would lock out a restarted worker; 409 was not specified as retryable. |
| Invocations count for slow mode | 4.6 | Slow mode is per author. |
| Unknown frames and fields ignored by the bot; `ttlMs`, `serverTime` | 5 | Additive evolution inside v1; clock skew. |
| Reply length in UTF-16 units | 5 | The measure ordinary messages use. |
| Second step, audit tables, cascade triggers, backups, empty selection | 3 | Unspecified in the first draft. |
| GitHub discovery and Discord portability moved to appendices | A, B | Neither is needed to ship v1 commands. |
| Webhooks moved to their own document | 8 | Independent, and proposed to ship first. |
| New *Open* items: localization, who may manage, shown arguments, limits, client surface | 2, 3, 4, 5 | Decisions the first draft did not name. |

### Checked against `main` at `c120fcaa` (2026-11-27)

The revision above was written against a checkout from before `6b6f4f53`. These statements changed since, or were wrong then:

| Change | Section | What landed |
| --- | --- | --- |
| The client has a command list; names can match its built-in commands | 4.1, 4 *Client surface* | `835110bd` added the composer's command registry. |
| Three language packs, English as the fallback | 2 | `f402225b`, `1923e3a8`. |
| New tables and triggers are a `MIGRATIONS` entry | 3 | `c361fecf` versions the schema with `user_version`. |
| Member posts pass through `PostQueue` | 4 *Revocation* | `de169594` batches waiting posts in one transaction. |
| Cockpit's second step matches activities | 3 | Installing an activity asked for a fresh second step all along; the earlier text said it did not. |
| Reply cap named as this contract's own | 5 | Ordinary messages allow 20,000 units. |
| Link to the activity contract; manifest bounds; SDK package | intro, 2, 6 | The contract is `sdk/CONTRACT.md`, now at 0.3.0 with most of it implemented. |

### Decisions of 2026-11-27

| Question | Section | Decision |
| --- | --- | --- |
| Who may manage installations | 3 | The server owner only. |
| How a reply is stored | 4 | A write of its own, outside `PostQueue`. |
| Localization of the manifest | 2 | One name and one description; the author picks the language. |
| Manifest bounds | 2 | This contract's own: 32, 80 and 240. |
| Life of a superseded revision | 3 | A number of days set in Cockpit, 30 by default. |
| Arguments beside the reply | 4.5 | Not shown and not stored. |
| Membership, role and override changes | 4 | Invalidate pending interactions at once. The mechanism is *Proposed*. |
| Client surface | Appendix C | A first proposal, *Proposed* apart from the three rows at the end of this table. |
| Names shared with built-in commands | 4 | Always a selection from the list. |
| Limits | 5 | The suggested numbers, for now. |
| SDK package | 6 | A `./bots` export of `@stellarr/sdk`. |
| A changed operator | 3 | Suspends every installation at once until its owner accepts, and rotates the credential. What counts as a change is *Proposed*. |
| Commands of an offline bot | C.1 | Hidden from the command list. |
| Typing a `boolean` | C.5 | `true` or `false`, in every language. |
| The pending line after a reload | C.4 | Gone; it lives in the invoking tab only. |

Nothing in this document is *Open* after these. The statements marked *Proposed* still await confirmation.

## Appendix A. Publishing bots in StellarrActivities (deferred)

*Deferred from v1. Text carried over from the first draft, with `operator` read as in section 2.*

The proposed home for Stellarr's own bots is `Arkazzae/StellarrActivities`. Each bot lives under `bots/<id>/` with its manifest, worker source and operator instructions. A release for Dice uses the tag `bots/dice/v1.0.0` and attaches `stellarr.bot.json`. The manifest ID and version must match the tag. Third-party authors can use the same convention in their own repositories.

In *Cockpit → Bots*, the instance owner can paste a GitHub repository URL to discover the newest stable bot release per ID, or import a manifest file directly. Repository discovery considers published, non-prerelease tags matching `bots/<id>/v<semver>` with the manifest attached. Cockpit records the repository, tag and digest for review. An update from a repository must match the catalog entry's recorded source and ID. Publication alone grants no access. The bot operator deploys and pairs the worker separately.

Activity releases keep their existing `<activity-id>/v<version>` tags, ZIPs and `stellarr-release.json` index. A bot release carries no activity index. This lets the same repository URL serve both Cockpit sections, each showing only its own release type.

Acceptance case, when this ships:

| Exercise | Observable result |
| --- | --- |
| Discover a repository containing `synctube/v1.0.0` (activity) and `bots/dice/v1.0.0` (bot) | *Cockpit → Bots* offers Dice and *Cockpit → Activities* offers SyncTube. Discovery neither grants access nor starts a worker. |

## Appendix B. Discord portability

*Text carried over from the first draft.*

The same `rollDice` function can be called by a Discord command handler, which reads its own interaction options and replies via Discord. Each adapter registers its commands, authenticates and delivers replies on its own platform. The operator explicitly enables each connection and keeps the two credentials separate.

| Bot the owner has | v1 integration |
| --- | --- |
| Native Stellarr bot | Its operator supplies the manifest and connects a worker implementing this protocol. The SDK is optional. |
| Discord bot with a cooperating author or self-hostable source | Its author maps supported Discord slash commands and arguments to a Stellarr manifest and calls the shared handlers through the Stellarr adapter. Discord-specific role, member and channel objects require porting. |
| Third-party Discord bot available only through an invite link | The provider must offer a Stellarr integration before it can appear as a working bot in this catalog. A Discord install link grants access in Discord. |

Discord defines its own installation contexts, interaction delivery and Gateway. Binary compatibility with Discord clients and message relay between platforms would be separate projects. [Discord application installs](https://docs.discord.com/developers/resources/application#install-links) · [Interactions](https://docs.discord.com/developers/interactions/overview) · [Gateway](https://docs.discord.com/developers/events/gateway)

Acceptance case:

| Exercise | Observable result |
| --- | --- |
| Run a dual-platform dice bot | One Stellarr invocation produces one Stellarr reply. A Discord invocation produces one Discord reply. Credentials and channel IDs stay on their respective connections. |

## Appendix C. Client surface (preliminary)

*Proposed as a first draft for review, except where a point is marked Decided.* This appendix covers what the web client and the instance say to each other about bot commands. None of it reaches a bot: sections 4 and 5 remain the contract for that.

It follows what the client does today. A post is an HTTP request (`POST /api/channels/{channelId}/messages`), news arrives as an event on `/hubs/app` (`MessageCreated`), and a refusal is a problem document.

### C.1 Listing a channel's commands

`GET /api/channels/{channelId}/commands` answers with the commands the caller may invoke there now.

```json
{
  "commands": [
    {
      "installationId": "n4",
      "bot": { "name": "Dice", "slug": "dice" },
      "name": "rzut",
      "description": "Roll dice, for example 2d6.",
      "options": [
        { "name": "dice", "description": "Dice expression", "type": "string", "required": true }
      ]
    }
  ]
}
```

- A command is listed only when every check of section 4.2 passes for the caller. A channel the caller cannot see answers 404, as its other routes do.
- The commands are those of the revision the installation is pinned to. An installation past its limit, waiting for its owner to accept a changed operator, or with no selected channel, contributes none.
- **A bot that is offline contributes no command (*Decided*).** Its commands are hidden, not shown as blocked, the way the registry hides a built-in command the place does not have (`{ shown: false }`). The catalog a server owner installs from still shows the bot with its offline status (section 3). The client sends a slash word that finds nothing as typed, so `/rzut 2d6` entered while Dice is offline is posted to the channel as text.
- The client reads the list when it opens a text channel, and again on `ChannelCommandsChanged`. The instance sends that event, carrying `{ "serverId": "s2" }` and nothing else, to a server where the set may have changed: after an install, an edit of channels, an accepted revision, a changed or accepted operator, an uninstall, a disable or enable in Cockpit, and when a bot's connection opens or closes.

### C.2 Invoking a command

`POST /api/channels/{channelId}/commands`

```json
{ "installationId": "n4", "name": "rzut", "args": { "dice": "2d6" }, "requestId": "r1" }
```

An accepted invocation answers 202.

```json
{ "interactionId": "q9", "state": "pending", "expiresAt": "2026-11-26T12:00:15Z" }
```

A repeat with the same `requestId` answers with the interaction's current state (section 5), which by then may be `answered` with its `messageId`.

| Refusal | Answer |
| --- | --- |
| Unknown channel, installation or command, or one the caller may not use | 404 |
| A missing required value, an unexpected argument or a wrong type | 400 naming the option |
| The same `requestId` with another command or other arguments | 409 |
| Slow mode | 429, the answer a message gets |
| The invocation rate, the pending limit or a saturated bot | 429 with `Retry-After` |
| The bot went offline after the list was read | 503 with the code `bot_unavailable` |

*Alternative:* a hub method, `InvokeCommand`. The hub's methods today are joins and notices (`JoinChannel`, `SendTyping`). A request that stores a row fits the HTTP pipeline, which already has the rate limits, the JSON body rules and the problem documents.

### C.3 Outcome

The instance sends `CommandFinished` to the caller's own connections and to nobody else.

```json
{ "interactionId": "q9", "channelId": "c8", "outcome": "answered", "messageId": "m5" }
```

- `outcome` is `answered`, `declined`, `timeout` or `invalid` (the access or the grant was revoked). A decline carries the bot's optional `reason`.
- The reply itself arrives as an ordinary `MessageCreated` for everyone who sees the channel. Its integration marker names the invoker and the command.
- A client that was disconnected reads `GET /api/interactions/{interactionId}`, which answers with the same body for as long as the interaction's row exists. Only the invoker may read it; anybody else gets 404. The route parameter is an id, to be listed as open in `http/secret_path.rs`.

### C.4 What the caller sees

- From acceptance to outcome the invoking client shows a pending line in the channel: the command as typed, and the bot's name. The line is local to that client. It is never stored, and nobody else sees it.
- **The line does not survive a reload (*Decided*).** It lives in the memory of the tab that invoked, with the interaction's ID. A reply still arrives as a message. A decline, a timeout or a revocation that lands after a reload inside the 15-second window goes unseen, and the command is not put back in the composer. Keeping the ID in the browser's storage and reading C.3's route after a reload can be added later without a change on the instance.
- `answered` removes the line. The reply stands in the history in its place.
- `declined` turns it into a private notice with the bot's reason as literal text, attributed to the bot. The notice can be dismissed and is gone after a reload.
- `timeout` and `invalid` turn it into a private notice in the instance's words and put the command back in the composer. Another try is a new `requestId`.

### C.5 The registry

- Bot commands form a fourth group after `add`, `message` and `server`. It is drawn only where the list of C.1 has entries, which is a channel's composer. A conversation between people has none.
- An entry's name and summary are the manifest's literal text, where a built-in command has message keys. It has no English alias. Its line shows the bot's name and slug.
- Options become chips: `string` uses the existing `text` kind, and `integer` and `boolean` are two new kinds. `person` is not used, since v1 has no option that names a member.
- **A `boolean` is typed as `true` or `false` (*Decided*),** in every language of the client. The chip offers the two words. Anything else is not a value, and the client says so before it sends.
- A typed name is matched as built-in names are, without case and without diacritics. A typed name that matches more than one command runs nothing by itself (section 4).
- On a phone the group is rows in the sheet.
