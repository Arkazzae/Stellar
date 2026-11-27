# Incoming webhook contract v1

*Status: proposed for review, checked against `main` at `c120fcaa` (2026-11-27).* This draft defines how an external sender posts a text message to one Stellarr channel with a single HTTPS request. It is split out of the [bot contract](BOT-CONTRACT.md) because it has no dependency on the bot gateway and is the proposed first use case. Section 2 defines the *integration-authored message*, which the bot contract reuses.

> Statements carry a status where it matters: **Decided** (chosen in the review of 2026-11-27), **Proposed** (this draft's recommendation, with the alternative named) or **Open** (a product decision nobody has made yet). Unmarked text is carried over from the original proposal. The routes and tables below describe proposed interfaces.

## 1. Use case and participants

A CI job posts "Version 1.4 deployed" to #releases with one HTTPS request. The instance implements this directly. The sender only needs a secret URL and an HTTP client. There is no bot installation, worker process or SDK dependency. The bot repository can host example senders.

| Participant | Responsibility |
| --- | --- |
| Instance owner | Enables incoming webhooks in Cockpit and can disable every delivery on the instance. |
| Server owner | Creates a named webhook for one text channel under *Server settings → Integrations → Webhooks*, rotates its secret and deletes it. |
| Sender | Holds the secret URL and posts text. It learns nothing about the server beyond the status code. |
| Stellarr instance | Owns the webhook record, derives author and channel from it, stores the message and audits management actions. |

## 2. Integration-authored messages

A webhook post and a bot reply are both messages that no member wrote. This section defines them once.

**Author row (*Proposed*).** `Messages.AuthorId` is a `RESTRICT` foreign key to `Users`, and permissions are evaluated only for enabled members of a server. An integration therefore gets an author row of the kind the Discord import already creates for its placeholders: an account with `Origin` set (`webhook` or `bot`; the only value today is `discord`), disabled, without a password or an address, a member of no server, and found by nobody (a friend request, a conversation, an invitation, a block or an `@mention` answers as for an account that is not there).

- One row per webhook, and one per bot installation. It holds no permission anywhere, so no route that takes a member (roles, overrides, kick, ban, ownership) can name it.
- `Username` is unique on the instance, so it is generated (`webhook-<id>`, `bot-<installationId>`) and never shown. The configured name is the row's `DisplayName`.
- Deleting the webhook or installation deletes the row when it authored nothing. Otherwise the row stays, with its last display name, for as long as a message refers to it. A reinstall or a new webhook gets a new row.
- *Alternative:* a real server member with a bot flag, excluded route by route from member lists, mentions, direct messages, roles and moderation. It costs an exclusion in every such route and a test for each.

**Message marker (*Proposed*).** The stored message carries an integration marker (kind, and for a bot reply the invoker and command, see the bot contract). Server and client both act on it:

| Concern | Behaviour |
| --- | --- |
| Label | The message shows the author's display name with a `webhook` or `bot` label that a member cannot set on their own messages. |
| Mentions | The server writes no inbox entries and queues no e-mail or push. The client does not style `@name` as a mention. |
| Link previews | The message is never queued for unfurling and stores no cards. |
| Formatting | The text goes through the ordinary inline renderer (emphasis, code, bare `http(s)` links). A labelled link `[label](url)` renders as its literal text, so a sender cannot hide a destination behind a label. *Alternative:* render the text fully literal. |
| Length | Measured as ordinary messages are, in UTF-16 units after trimming Unicode white space from both ends. |
| Slow mode | Does not apply to the author row. Post times are written by the `Messages_PostTime_AfterInsert` trigger for every non-system message, so the trigger has to skip marked messages, which takes a migration that drops and recreates it (section 11). |
| Read markers | Counts as unread like any message. Nobody's read marker advances. |
| Edit and delete | Nobody edits it. A member with `manageMessages` in the channel deletes it like any other message. |
| History, sync, search | An ordinary row of `Messages`: the journal and the full-text index see it. |

**Older clients (*Decided*).** The minimum client version is raised when the feature ships, so every admitted client shows the label. The browser build ships with the instance; the client gate has a version floor for the desktop client only, so the Android client needs one too (section 11). The API already sends an account's `origin`, but the web client does not read it, so no client shows a label today. A client released before the marker would show the message as an ordinary one from an ordinary-looking account: a webhook named after a member would be indistinguishable from that member there, and `@name` might be styled as a mention, though nobody is notified.

## 3. Granting access

- The instance owner enables incoming webhooks in Cockpit. The setting is off on a new instance.
- The current server owner creates a named webhook for one existing text channel under *Server settings → Integrations → Webhooks*. Each webhook gets its own id, secret and author row. The name is nonblank, up to 80 UTF-8 bytes.
- **Who may manage (*Decided*).** The current server owner only. Holding `manageServer` does not grant it. The same holds for bot installations.
- **Second step (*Proposed*).** Creating a webhook, rotating its secret and enabling webhooks in Cockpit require a fresh second step under the active MFA policy, as an ownership hand-over does (`fresh::require` in `administration/ownership.rs`) and as installing an activity does in Cockpit. *Alternative:* no second step.
- Ownership transfer gives the new server owner management of existing webhooks. Their URLs keep working.
- **Limit per server (*Decided*).** 20 webhooks.

## 4. Request

`POST /api/webhooks/{id}/{secret}` with `Content-Type: application/json` and a body such as `{"content":"Version 1.4 deployed"}`.

- **Secret in a header (*Proposed*).** `POST /api/webhooks/{id}` with `Authorization: Bearer <secret>` is accepted as an equivalent form, for senders behind a proxy that logs URLs. *Alternative:* URL only.
- `content` is required: a string that is nonblank after trimming and at most 2,000 UTF-16 units (*Proposed*; the original said 2,000 UTF-8 bytes, which is neither the measure ordinary messages use nor Discord's "up to 2000 characters"). The cap follows Discord's: an ordinary message here holds up to 20,000 units.
- The body is at most 16 KiB (*Proposed*). The bundled Nginx admits 128 MiB, so the API enforces this limit itself.
- The request carries no session, no `X-Stellarr-Client` header and no cookies, and the route answers no CORS preflight.

**Discord compatibility (*Decided*).** The route shape and `content` follow Discord's [Execute Webhook](https://docs.discord.com/developers/resources/webhook#execute-webhook). Most existing senders also send fields this contract does not implement, so rejecting every unknown field would refuse them. They are handled as follows:

| Field or parameter | Handling |
| --- | --- |
| `username`, `avatar_url`, `tts`, `allowed_mentions`, `flags` | Accepted and ignored. The author and label always come from the webhook record. |
| `?thread_id`, `?with_components`, `?wait=false` | Accepted and ignored. |
| `?wait=true` | 200 with a minimal message object (`id`, `channel_id`, `content`). Discord answers with the created message, and a sender that parses that body breaks on a 204. |
| `embeds`, `components`, `poll`, `attachments`, `thread_name`, `applied_tags` | 400 naming the field, also when `content` is present, so that nothing a sender meant to show is dropped in silence. |
| `multipart/form-data`, `payload_json`, files | 415. |
| `/slack` and `/github` suffixes | Not routed: 404. |
| Any other field | 400. |

## 5. Response and limits

| Outcome | Answer |
| --- | --- |
| Message committed | 204 with no body. Under `?wait=true`, 200 with the message object of section 4. |
| Invalid JSON, missing or invalid `content`, unsupported field | 400 with a problem document naming the field. |
| Unknown id, wrong secret, deleted or rotated webhook, webhooks disabled in Cockpit | 404, the same answer for each, compared in constant time. |
| Unsupported content type | 415. |
| Body over the limit | 413. |
| Rate limited | 429 with `Retry-After` in whole seconds. |

- Each accepted POST creates one message. The instance derives author and channel from the webhook record. The secret grants only posting to that channel.
- **Idempotency (*Proposed*).** A sender may add `Idempotency-Key` (1–64 visible ASCII characters). Within 5 minutes, a repeat with the same key and the same `content` on the same webhook answers 204 without a second message, including after an instance restart. Under `?wait=true` the repeat answers 200 with the first message's object. The same key with a different `content` answers 409. Without the header every accepted POST is a new message, as in the original. *Alternative:* no idempotency.
- **Rate limits (*Decided*).** 30 posts per minute per webhook, and 30 failed lookups per minute per client address, each refused with 429 and `Retry-After`. The existing limiter counts per client address in a fixed window and sends no `Retry-After`, so this needs a limiter of its own.

## 6. Lifecycle and revocation

- Rotation invalidates the old secret at once. Deleting the webhook or its channel revokes it. Disabling webhooks in Cockpit blocks every delivery on the instance while keeping the records. Check all of these inside the transaction that stores the message.
- A channel has no foreign key to its server and deletes cascade through triggers in `database/schema.sql`. The webhook table needs the same triggers for a deleted channel and a deleted server, so that the rule holds whichever code issues the delete. They are added by a migration, not in `schema.sql` (section 11).
- Stored messages keep their author and label after the webhook is gone (section 2).
- **Backups (*Proposed*).** Restoring a backup restores the webhooks and verifiers it contains, including ones deleted or rotated since. The restore notes say so, and the post-restore step that already invalidates sessions and tokens disables incoming webhooks in Cockpit until the owner re-enables them. *Alternative:* document it only.

## 7. Secrets and logs

- The secret is high-entropy and generated by the instance. Show the URL only on creation or rotation and store only a verifier (a SHA-256 digest, as for the sign-in recovery codes in `RecoveryCodes.CodeHash`; the encryption recovery code that replaced the PIN is a different thing).
- The route parameter is named `secret` and is added to `SECRETS` in `http/secret_path.rs`, so request logs and the panel's buffer show `***`. The test that walks every route fails until the parameter is classified.
- The bundled Nginx writes no access log. A proxy the operator runs in front of the container logs what it logs: the reverse-proxy guide names the path to redact, and section 4's header form avoids the problem.

## 8. Audit

Enabling and disabling webhooks goes to the operator audit (`OperatorAuditEntries`). Creation, rename, rotation and deletion go to the server audit (`ServerAuditEntries`) with the acting member, the webhook id and name, and the channel. Secrets and posted content are never recorded.

## 9. Outgoing webhooks

*Deferred.* Outgoing webhooks would send selected Stellarr events to an external HTTPS endpoint. They need their own event and channel grants, signed deliveries, delivery IDs and retry policy. Enabling incoming webhooks grants no event subscriptions. HTTP delivery of a bot's slash commands is a separate transport decision. The v1 bot connection uses WebSocket.

## 10. Acceptance cases

| Exercise | Observable result |
| --- | --- |
| Post to #releases, rotate the URL, retry the old URL | The channel contains exactly one message. The retry answers 404. |
| Post `@ada https://example.com [x](https://evil.example)` | One labelled message. Ada gets no inbox entry, e-mail or push. No preview card is stored. The labelled link shows as literal text. |
| Post with `username` and `avatar_url` | 204. The message shows the webhook's configured name. |
| Post with `embeds` | 400 naming `embeds`. No message. |
| Post with `content` and `embeds` together | 400 naming `embeds`. No message. |
| Post with `?wait=true` | 200 with `id`, `channel_id` and `content`. One message. |
| Create a twenty-first webhook on a server | Refused. The twenty existing ones keep working. |
| Sign in with a desktop or Android client older than the release that ships the label | The client gate refuses it and names the minimum version. |
| Post twice with the same `Idempotency-Key` and content, restarting the instance in between | One message, two 204 answers. |
| Exceed the per-webhook rate | 429 with `Retry-After`. No message. |
| Delete the channel, or disable webhooks in Cockpit, then post | 404. Re-enabling in Cockpit makes surviving webhooks work again. |
| Post as a signed-in member to the same route without the secret | 404. |
| Read the request log after a post | The secret segment is `***`. |
| Delete the webhook after it posted | The message stays with its author name and `webhook` label. `@webhook-<id>` names nobody. |

## 11. Where this meets the current code

| Area | Today | Needed |
| --- | --- | --- |
| Author | `Messages.AuthorId` → `Users`, `RESTRICT` | Author row per section 2, as `discord_import` makes placeholders. |
| Posting | `messages/post.rs` admits a member with `sendMessages`, then notifies mentions and queues previews. Since `de169594` the handler reaches it through `PostQueue` (`messages/post/batch.rs`), which stores up to four waiting posts in one transaction | A second entry point that takes the webhook record instead of a member and skips mentions, deliveries, previews, slow mode and the read marker. It takes a write of its own and does not join the queue (*Decided*). The checks of section 6 run in that transaction. |
| Rendering | `web/src/stellarr/lib/messages/inline.ts` linkifies, resolves mentions and renders labelled links from text alone; the client ignores `origin` | The label, and an option driven by the marker: no mentions, labelled links literal. |
| Slow mode | `Messages_PostTime_AfterInsert` writes a post time for every non-system message | Skip marked messages. |
| Rate limiting | `http/rate_limit.rs`: per address (uploads per account), fixed window, 429 without `Retry-After` | A per-webhook limiter with `Retry-After`. |
| Secrets in paths | `http/secret_path.rs` lists `asset_key`, `code`, `path` and `request_id` | Add `secret`. |
| Schema | `database/schema.sql` is the baseline of a new database. Since `c361fecf` every later change is an entry in `MIGRATIONS` (`database/schema.rs`), applied once under `user_version`. Cascades are by trigger | One new `MIGRATIONS` entry: tables for webhooks and idempotency keys with their triggers, and `Messages_PostTime_AfterInsert` dropped and recreated to skip marked messages. Housekeeping purges expired keys. |
| Client versions | `http/client_gate.rs` compares a reported version against a floor for the desktop client only | Raise that floor to the release that shows the label, and add a floor for the Android client. |
| Requests without a browser | The client gate covers only the sign-in and session paths (`GATED_PATHS` in `http/client_gate.rs`), and `http/pipeline.rs` refuses a cross-site request only when it carries a session cookie | Nothing: a sender with no cookie and no `X-Stellarr-Client` reaches the route. |

## 12. Decisions of 2026-11-27

| Question | Section | Decision |
| --- | --- | --- |
| Older clients | 2 | The minimum client version is raised when the feature ships. |
| Who may manage webhooks | 3 | The server owner only. |
| Limit per server | 3 | 20 webhooks. |
| Discord compatibility | 4 | Kept, with the table of section 4. |
| `?wait=true` | 4, 5 | 200 with a minimal message object. |
| Unsupported fields such as `embeds` | 4 | 400 naming the field, also beside `content`. |
| Rate limits | 5 | 30 posts a minute per webhook and 30 failed lookups a minute per address, with `Retry-After`. |
| How a post is stored | 11 | A write of its own, outside `PostQueue`. |

Nothing in this document is *Open* after these. The *Proposed* statements still wait for a yes or a no.
