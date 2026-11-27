# Bot contract v1

**Status: proposed for review.** This draft defines the contract between a Stellarr instance, a server owner and an external bot. The routes, frames and `@stellarr/sdk/bots` export below describe proposed interfaces. The [activity contract](CONTRACT.md) covers the browser SDK.

## 1. Who runs what

| Participant | Responsibility |
| --- | --- |
| Instance owner | Admits the bot to **Cockpit → Bots**, reviews its operator, commands and requested access, and can disable it across the instance. |
| Server owner | Browses admitted bots under **Server settings → Bots**, reviews what leaves the server, selects text channels, installs the bot and can remove it. |
| Server member | Invokes a command in an eligible channel and sees the bot's attributed reply there. |
| Bot operator | Hosts the process, its dependencies, secrets and any application state. This can be the author or a self-hosting instance owner. |
| Stellarr instance | Owns the catalog, installation records, channel permissions, command dispatch, message history and audit trail. |

A catalog entry gets an instance-assigned `appId`. A server installation gets a fresh `installationId` and a local bot identity. There is at most one current installation per `(serverId, appId)`. A single process can serve many installations over one connection to the instance. A process serving several instances opens a separately authenticated connection to each.

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
  "operator": { "name": "Dice Club", "url": "https://dice.example" },
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
- Bot and operator names are nonblank, up to 80 UTF-8 bytes. Descriptions are nonblank, up to 240 UTF-8 bytes. The operator URL is an HTTPS link shown to both owners. It is informational and carries no credentials.
- Declare 1–32 commands, each with up to 8 named options. Option types are `string` (up to 512 UTF-8 bytes), `integer` (JSON safe integer) and `boolean`. The instance rejects missing required values, unexpected arguments and wrong types before dispatch. Omitted optional values are absent from the arguments object.
- The single v1 capability, `commands.respond`, delivers an explicitly invoked command and authorizes one plain-text reply in its channel. The install screen shows the operator, commands, fields sent (section 4), reply visibility and selected channels. Channel selection starts empty. A new installation requires at least one existing text channel on that server.

### Publishing bots in StellarrActivities

The proposed home for Stellarr's own bots is `Arkazzae/StellarrActivities`. Each bot lives under `bots/<id>/` with its manifest, worker source and operator instructions. A release for Dice uses the tag `bots/dice/v1.0.0` and attaches `stellarr.bot.json`. The manifest ID and version must match the tag. Third-party authors can use the same convention in their own repositories.

In **Cockpit → Bots**, the instance owner can paste a GitHub repository URL to discover the newest stable bot release per ID, or import a manifest file directly. Repository discovery considers published, non-prerelease tags matching `bots/<id>/v<semver>` with the manifest attached. Cockpit records the repository, tag and digest for review. An update from a repository must match the catalog entry's recorded source and ID. Publication alone grants no access. The bot operator deploys and pairs the worker separately.

Activity releases keep their existing `<activity-id>/v<version>` tags, ZIPs and `stellarr-release.json` index. A bot release carries no activity index. This lets the same repository URL serve both Cockpit sections, each showing only its own release type.

## 3. Approval, updates and revocation

| Action | Effect |
| --- | --- |
| Admit and pair | The instance owner reviews the manifest and generates a credential for this application on this instance. The first authenticated worker connection completes pairing. The enabled entry then appears in the catalog and remains visible with an offline status during subsequent disconnects. |
| Install or edit | Only the current server owner can grant access and select channels. Repeating the same install returns the existing installation. Changing channels is an explicit edit. Newly created channels are excluded until selected. |
| Approve an update | Cockpit presents the differences. A change confined to display names, descriptions or the version keeps existing grants, provided the operator is the same. Changing operator details, permissions, the command list or its argument schema suspends existing installations and invalidates pending commands. Each server owner can accept the current revision or remove the installation. Removed access stops immediately. |
| Remove a channel or uninstall | Invalidate pending interactions in the affected scope. Uninstall also retires its service identity. Reinstall creates a new installation ID. Historical replies keep their author and bot label. |
| Disable or remove in Cockpit | Disable hides the entry from new installs, closes its connection and invalidates all pending interactions while preserving grants. Re-enable permits fresh commands under still-valid grants. Removal revokes the credential and retires all installations. |

Server deletion has the effect of uninstalling every bot there. Ownership transfer gives the new server owner management of existing grants. Audit records identify the actor, application, installation where applicable, revision, action and changed channel or permission grants. Command arguments and credentials are excluded from these records.

## 4. Dispatch and reply

1. The member selects a command in a server text channel. The picker shows the bot name and unique catalog slug when two apps declare the same command. Invocation addresses `installationId + command`. The instance derives the actor from the member session. A typed command with an ambiguous name requires a selection. Ordinary chat messages do not generate bot events.
2. Before exposing the command, dispatching it, and committing a reply, Stellarr checks the enabled app, current installation and accepted grant, selected channel, and the member and bot's effective `viewChannel` and `sendMessages` in that channel. The bot has an installation-specific service identity subject to the existing base role and section/channel overrides. Its only bot action is the approved capability. Private channels require both channel selection and effective access.
3. The runtime gets the IDs, command name, typed arguments and expiry shown below. The actor ID is opaque and stable within that installation. The member's profile, role list, other messages, DMs and voice data are outside this event. Submitting a command discloses its arguments to the displayed operator.
4. The only valid destination for a reply is the original channel, derived by Stellarr from the interaction. The instance assigns authorship to the bot, labels it as a bot and applies normal channel visibility to the stored message. Reply text cannot trigger mentions, embeds or attachment fetches. Success appears to channel members. Unavailability, denial and timeout are reported privately to the caller.

Changes to membership, roles, channel overrides, or bot grants invalidate outstanding interactions that lose access. The instance orders dispatch and reply commits against these changes. After revocation commits it cannot start a dispatch or commit a reply in the revoked scope. A previously committed message stays in history. Arguments already sent to the operator cannot be recalled. Restoring access never revives an invalidated interaction.

## 5. Runtime wire protocol

The bot opens `wss://<instance>/api/bots/gateway` with WebSocket subprotocol `stellarr.bot.v1` and the HTTP upgrade header `Authorization: Bearer <bot-token>`. Outbound connection from the runtime lets an operator run behind NAT. The instance never connects to a URL supplied in the manifest.

The instance generates an opaque high-entropy credential, shows it to the instance owner once and stores only its verifier. The owner provisions the bot operator through a private channel. The SDK verifies the instance's TLS identity, sends the credential only to the configured origin and rejects redirects. The credential is valid exclusively on the bot gateway. Rotation immediately invalidates the old token, its connection and outstanding interactions. The ordinary chat client and server installation flow never receive this credential.

One authenticated connection per `appId` per instance is active at a time. An invalid or disabled credential fails the upgrade with HTTP 401. An additional live connection receives HTTP 409. The server detects dead peers using WebSocket Ping/Pong and releases the slot on disconnect. The SDK reconnects after transport loss with bounded exponential backoff and jitter. Authentication failures require operator intervention.

Each frame is a UTF-8 JSON object in a WebSocket text message, up to 32 KiB. IDs are opaque strings. Times are UTC RFC 3339 strings assigned by the instance. Each invocation identifies the approved manifest version so a worker can select the corresponding handler. The following exchange shows one complete command.

**Instance → bot**, once after a successful upgrade

```json
{ "type": "ready", "instanceId": "i1", "appId": "a7" }
```

**Instance → bot**, when a member invokes `/rzut 2d6`

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
  "expiresAt": "2026-11-26T12:00:15Z"
}
```

**Bot → instance**, then **instance → bot** after the message commits

```json
{ "type": "interaction.reply", "interactionId": "q9", "text": "2d6 → 8" }
```

```json
{ "type": "interaction.result", "interactionId": "q9", "messageId": "m5" }
```

- A reply contains 1–2,000 UTF-8 bytes of nonblank text. The instance binds each interaction to the authenticated application, installation, channel and invoker. A bot-supplied destination or author is an invalid frame.
- An invocation has a 15-second window starting at acceptance. Each newly accepted invocation is dispatched at most once. Offline or saturated bots fail immediately. Disconnecting does not queue or replay commands. A worker that still has the ID and computed text can reconnect and retry its reply.
- The client submits a unique request ID for each intentional invocation. Within five minutes, a still-authorized repeat by the same actor for the same installation, channel and request ID returns the original invocation status, including after instance restart. A changed command or arguments with that key conflicts. Two intentional rolls use different keys.
- The instance atomically stores the message and its receipt. While still authorized, an identical reply retry within five minutes of commit returns the same `messageId`, including after reconnect or instance restart. A different text conflicts. A first reply arriving at or after expiry fails. After the receipt window, the ID cannot produce a new message.
- A rejected reply receives `{"type":"interaction.error","interactionId":"q9","code":"expired"}`. Codes are `invalid_reply` (bad text or extra fields), `unknown_interaction` (unknown ID or another app's ID), `access_revoked`, `expired`, `conflict` and `rate_limited`. A rate limit includes a positive `retryAfterMs`. A retry is useful only while its applicable window is open. Unreadable envelopes and unknown frame types close the socket with 1002, oversized messages with 1009.
- Stellarr bounds pending work and invocation rates per member and installation. Capacity exhaustion returns a retry delay to the invoking client and never enqueues work for later delivery. A handler that exits without replying produces a timeout for the caller.

## 6. Author SDK and Discord portability

The proposed server-side entry point is `@stellarr/sdk/bots`. It maps typed command arguments to an author handler, submits its returned text, and resolves `close()` when the connection is closed. The transport owns reconnection and reply retries. A retry reuses the handler's computed reply. For dice, this preserves the original roll.

```js
import { connectBot } from "@stellarr/sdk/bots";
import { rollDice } from "./game.js";

const bot = await connectBot({
  instance: process.env.STELLARR_URL,
  token: process.env.STELLARR_BOT_TOKEN,
  commands: {
    rzut: ({ args }) => rollDice(args.dice)
  }
});
```

Here `rollDice(expression)` returns display text. A handler may also return a promise of that text. Its context exposes the fields of `command.invoke`. The same `rollDice` function can be called by a Discord command handler, which reads its own interaction options and replies via Discord. Each adapter registers its commands, authenticates and delivers replies on its own platform. The operator explicitly enables each connection and keeps the two credentials separate.

| Bot the owner has | v1 integration |
| --- | --- |
| Native Stellarr bot | Its operator supplies the manifest and connects a worker implementing this protocol. The SDK is optional. |
| Discord bot with a cooperating author or self-hostable source | Its author maps supported Discord slash commands and arguments to a Stellarr manifest and calls the shared handlers through the Stellarr adapter. Discord-specific role, member and channel objects require porting. |
| Third-party Discord bot available only through an invite link | The provider must offer a Stellarr integration before it can appear as a working bot in this catalog. A Discord install link grants access in Discord. |

Discord defines its own installation contexts, interaction delivery and Gateway. Binary compatibility with Discord clients and message relay between platforms would be separate projects. [Discord application installs](https://docs.discord.com/developers/resources/application#install-links) · [Interactions](https://docs.discord.com/developers/interactions/overview) · [Gateway](https://docs.discord.com/developers/events/gateway)

## 7. Acceptance cases

| Exercise | Observable result |
| --- | --- |
| Discover a repository containing `synctube/v1.0.0` (activity) and `bots/dice/v1.0.0` (bot) | **Cockpit → Bots** offers Dice and **Cockpit → Activities** offers SyncTube. Discovery neither grants access nor starts a worker. |
| Admit Dice, install on server A in `#games`, invoke `/rzut 2d6` | One invocation reaches the worker and one attributed reply appears in `#games`. The worker receives only the fields in section 5. |
| Install Dice on server B as well | The same connection serves two distinct installation IDs. Removing A leaves B operational. |
| Try a non-owner install, a command in an unselected channel, or a forged destination | Management and dispatch are denied as applicable. The forged reply produces no message. |
| Remove the grant, delete the channel, or revoke the user's or bot's channel access while the handler is running | The late reply cannot commit. Restoring access leaves the old interaction invalid. |
| Drop the socket after a reply commits but before the receipt arrives | Retrying the same computed text yields the first message ID. The channel history contains one roll. |
| Approve a new argument on a command | The updated command becomes callable only after the server owner reviews the revised data disclosure. |
| Run a dual-platform dice bot | One Stellarr invocation produces one Stellarr reply. A Discord invocation produces one Discord reply. Credentials and channel IDs stay on their respective connections. |

Further contracts can add proactive messages, passive events, moderation, private responses and voice/music. Each needs an explicit capability, its own data disclosure and revocation rules. The command-only grant above authorizes none of those operations.

## 8. Channel webhooks

**Proposed first use case.** A CI job posts “Version 1.4 deployed” to `#releases` with one HTTPS request. The instance implements this directly. The sender only needs a secret URL and an HTTP client. There is no bot installation, worker process or SDK dependency. The bot repository can host example senders.

| Decision | Incoming webhook contract |
| --- | --- |
| Who grants access | The instance owner enables incoming webhooks in Cockpit. The current server owner creates a named webhook for one text channel under **Server settings → Integrations → Webhooks**. Each integration gets its own secret. |
| What the sender submits | `POST /api/webhooks/{id}/{secret}` with `Content-Type: application/json` and a body such as `{"content":"Version 1.4 deployed"}`. Accept only a nonblank `content` string up to 2,000 UTF-8 bytes. This supports the text-only subset of Discord's incoming webhook format. Rich embeds, sender overrides and provider-specific payloads need separate support. [Discord webhook format](https://docs.discord.com/developers/resources/webhook#execute-webhook) |
| What the channel receives | One ordinary text message attributed to the configured integration, with a webhook label and the channel's existing visibility rules. The instance derives author and channel from the webhook record. The secret grants only posting to that channel. The post triggers no mentions or link previews. |
| What the sender receives | Return `204` after commit, `400` for unsupported fields or invalid content, `413` for an oversized body, and `429` with `Retry-After` when rate limited. Each accepted POST creates a message. |
| How access ends | Rotation invalidates the old URL. Deleting the webhook or its channel revokes it. Disabling webhooks in Cockpit blocks every delivery on the instance. Check all of these at message commit. Unknown, disabled or revoked URLs return `404`. Stored messages retain their author label. |

Show a new secret URL only on creation or rotation, store only its verifier, and redact the secret in proxy and application logs. Audit creation, rotation and deletion by webhook ID. A basic acceptance check is to post to `#releases`, rotate the URL, then retry the old URL. The channel must still contain exactly one message.

**Outgoing webhooks** would send selected Stellarr events to an external HTTPS endpoint. They need their own event and channel grants, signed deliveries, delivery IDs and retry policy. Enabling incoming webhooks grants no event subscriptions. HTTP delivery of a bot's slash commands is a separate transport decision. The v1 bot connection in section 5 uses WebSocket.
