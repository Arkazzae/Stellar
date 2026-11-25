# The Stellarr API

The server behind the web, desktop and Android clients, written as a Cargo
workspace. One crate per folder of `crates/`:

| Crate | What it is |
| --- | --- |
| `archive` (`stellarr-archive`) | A zip writer and reader for backups: streaming, ZIP64, every checksum checked. |
| `cli` (`stellarr-cli`) | The operator's command line, `stellarr`: status, settings, secrets, sessions, account recovery and backups, straight on the data folder. |
| `deflate` (`stellarr-deflate`) | DEFLATE, gzip and zlib in both directions, and CRC-32, with no dependencies. |
| `discord` (`stellarr-discord`) | Reads a Discord server, from a template link or with a bot token, into a snapshot in Stellarr's own terms. |
| `embeds` (`stellarr-embeds`) | Link previews: finding the links in a message and fetching what a card for each needs. |
| `hub` (`stellarr-hub`) | The Stellarr WebSocket protocol for calls, group events and keep-alive. |
| `mail` (`stellarr-mail`) | The mail the server writes: every kind of message, the choice of transport and the Resend request. |
| `mentions` (`stellarr-mentions`) | Which accounts a message names with @username, read the way Stellarr's web client shows them. |
| `messages` (`stellarr-messages`) | Everything the server says to people, in each language of the catalogue. No dependencies. |
| `oidc` (`stellarr-oidc`) | OpenID Connect relying party: requests, state, callbacks and ID token checks, without network or storage. |
| `outbound` (`stellarr-outbound`) | The HTTP client every outbound call of the API goes through: HTTP/1.1 over TCP or rustls, one connection per request. |
| `s3` (`stellarr-s3`) | Requests and answers of the S3 calls Stellarr makes, with Signature Version 4, and no network I/O. |
| `server` (`stellarr-api`) | The server: the library `stellarr_api` and the binary `stellarr-api`: settings, database, HTTP pipeline and the groups of endpoints. |
| `smtp` (`stellarr-smtp`) | The mail the application sends: the message builder and an SMTP client. |
| `storage` (`stellarr-storage`) | Where uploaded files are kept: a folder of the data volume or an S3-compatible bucket, and the checks on what may be uploaded. |
| `timezone` (`stellarr-timezone`) | Time zones from the system's zone database: the offset and abbreviation in force at a moment. |
| `unicode` (`stellarr-unicode`) | Unicode general categories and simple upper-casing, from tables of its own, and the UTF-16 view of text that message code works in. |
| `webauthn` (`stellarr-webauthn`) | WebAuthn: the options a browser is given and the checks on what it sends back, with no I/O and no storage. |
| `webpush` (`stellarr-webpush`) | Web Push without a network: everything a server needs to notify a browser except the HTTP request itself. |
| `websocket` (`stellarr-websocket`) | The server side of WebSocket (RFC 6455) on a connection hyper has upgraded: handshake, frames, ping and pong, closing. |

Run cargo from this folder, with `-p` for the crate you work on:

```sh
cargo fmt -p stellarr-api -- --check
cargo clippy -p stellarr-api --all-targets -- -D warnings
cargo test -p stellarr-api
```

## Layout of `crates/server/src`

One line for each, from the module's own header. The three lists are macro
invocations in files of their own (see below).

```
lib.rs              serve() and run(); the lists groups.list.rs and workers.list.rs
main.rs             reads the environment, starts logging, runs
version.rs          the version the server reports, from the build (see Version)
settings/           everything read at start; a value that cannot be right stops the start
database/           SQLite: pool, schema.sql, errors, the write gate
values/             Id, Timestamp, int_enum!: how a value is stored and written
encoding.rs         base64url and hex, written once
cursor.rs           opaque URL-safe Base64 cursors in standard byte order
seal.rs             sealed values: what the server hands a client or the database to keep
object_store.rs     uploaded file store (files/ or S3) and incoming/ staging
admission.rs        upload reservation leases: quotas, free disk space and concurrency slots
http/               the pipeline and the tools a group is built from (path, extractors,
                    JSON and problems, rate limits, the client gate, answering with a file)
events.rs           what handlers tell the hub, and what they ask it
state.rs            what every handler can reach (AppState)
logging.rs          where the log goes, and what is kept out of the panel buffer
background/         work that runs beside the requests, from start to stop (workers)
auth/               who a request is from: passwords, setup code, sessions, the sign-in flow
instance/           what the instance is made of: accounts, servers, starter channels, invites
permissions/        who may do what on a server, and who sees which channel
panel/              what every route of /api/cockpit stands on: its gate, audit line, settings row
secrets/            the secrets an operator sets in the panel, sealed in the database
mail/               sending mail: settings, one delivery service, the queue of security alerts
realtime/           the hub the web client keeps open: connections, presence, methods, session watcher
accounts/           a group: founding, registering, signing in and out, who is signed in
account_security/   a group: security page, sessions, password and email changes from a session
second_step/        a group: 2FA challenges and enrolment, passkey sign-in, recovery, password reset
identity_providers/ a group: OpenID Connect sign-in, sign-up, re-authentication and linking
desktop_login/      a group: browser-approved sign-in handoff for the desktop client
own_profile/        a group: own profile, preferences and voice arrival sound
client_settings/    a group: per-account client settings sections synced across devices
bootstrap/          a group: /api/bootstrap initial state and runtime integration switches
servers/            a group: listing, discovering, creating and joining servers, invites and profiles
administration/     a group: server invites, audit log, notification defaults, handover and deletion
roles/              a group: server roles, member role assignments, and channel/section overrides
moderation/         a group: kicking and banning members from a server
channels/           a group: channels and sections, ordering, read markers and typing/view hub calls
messages/           a group: posting, editing and deleting channel messages, transcripts and journal
message_views/      a group: message reactions, channel pins, threads and FTS5 channel search
link_previews/      a worker: unfurling message links and storing preview images locally
social/             a group: friend requests, friendships and blocks across the instance
direct/             a group: 1:1 direct conversations, paged messages, sending and read markers
encryption/         a group: server side of 1:1 E2EE: public keys, PIN backup, devices and epochs
uploads/            a group: multipart attachments, profile/server images and encrypted DM blobs
assets/             a group: /assets/{id}, /poster and /preview, streamed with byte ranges
emoji/              a group: server custom emoji upload, listing, deletion and image serving
soundboard/         a group: server soundboard clips, ordering and playback switches
voice/              a group: voice rooms, media server sessions and workers, voice moderation routes
inbox/              a group: /api/notifications, a person's mention and system inbox
delivery/           a group and worker: away notifications via Web Push and mail, subscription routes
backups/            a group and worker: snapshots, full .zip archives, R2 upload, schedule and restore
housekeeping/       a worker: hourly pruning of journals, orphan uploads, deletion queue, backups, logs
cockpit/            a group: /api/cockpit (users, roles, servers, sessions, runtime, logs, settings)
releases/           a group: /.well-known/stellarr/client discovery and desktop/Android builds
health/             a group: /health/live and /health/ready
discord_import/     writes a stellarr-discord snapshot into an existing server, page by page
testing.rs          helpers the unit tests share (not compiled outside tests)
```

Shared plumbing (`settings`, `database`, `values`, `http`, `events`, `state`,
`background`) is not edited to add an endpoint. If a group needs something the
plumbing does not have, change the plumbing in its own commit.

## Adding a group of endpoints

A group is a folder under `src/` with a `mod.rs` that exposes one function.

```rust
// src/channels/mod.rs
use axum::extract::State;
use serde::Serialize;

use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Endpoints, Json, Path};
use crate::state::AppState;
use crate::values::Id;

pub fn routes() -> Endpoints {
    Endpoints::new().get("/api/channels/{id}", read)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChannelDto {
    id: Id,
    name: String,
    topic: Option<String>,
}

async fn read(State(state): State<AppState>, Path(id): Path<Id>) -> ApiResult<Json<ChannelDto>> {
    let found = state
        .database()
        .read(move |db| -> Result<Option<ChannelDto>, DbError> {
            let mut statement = db.prepare("SELECT \"Name\", \"Topic\" FROM \"Channels\" WHERE \"Id\" = ?1")?;
            let mut rows = statement.query([id])?;
            Ok(match rows.next()? {
                Some(row) => Some(ChannelDto { id, name: row.get(0)?, topic: row.get(1)? }),
                None => None,
            })
        })
        .await?;
    found.map(Json).ok_or_else(ApiError::not_found)
}
```

Then write the name of the group on a line of `crates/server/src/groups.list.rs`.
That is the only shared line a group touches. The file is one macro invocation
with a name to a line, and nothing else:

```rust
// crates/server/src/groups.list.rs
groups! {
    accounts,
    health,
    releases,
    channels,
}
```

There is no order: the routes of the groups are joined and the router does not
look at which came first, so a line goes anywhere and two branches that each
add one do not conflict (`.gitattributes` has `merge=union` for the file, which
keeps both sides' lines). The same holds for the other lists:
`src/workers.list.rs` (`workers! { mail::security_alerts, .. }`, module paths),
`src/realtime/methods.list.rs` (`methods! { caller, .. }`) and the test modules in
`tests/api/modules.list.rs` (plain `mod` lines). Do not edit `main.rs`, the
pipeline or another group.

`Endpoints` has `get`, `post`, `put`, `patch` and `delete`. Two modifiers apply
to the route added just before them:

- `.json_body()` for a route that reads a JSON body: a request that sends another
  content type is answered 415 before anything counts it.
- `.rate_limit(RateLimitPolicy::AuthWrite)` for a route that counts against one
  of the named limits.

- `.fresh_second_step()` for a route that could hand an account or the instance
  to somebody else: it needs a session like `.authorized()`, and when the
  account has a second step, one the session passed in the last five minutes.
  A session that did not is answered 403 with the code `fresh-mfa-required`,
  before the body is read. A handler that asks only sometimes calls
  `auth::fresh` itself.

Only the methods a group names are served. `HEAD` is not answered for a `GET`
route; it is a 405.

## The rules every group follows

**Handlers** return `ApiResult<T>`. `?` turns a database error or a message with
a missing value into the 500 problem, which says nothing of the cause and logs
it with the request's trace id. `ApiError::internal(cause)` is the same 500 for
anything else that went wrong on the server's side and can be displayed: the
cause is logged, never sent. For the answers that have a status and no body use
`ApiError::not_found()`, `forbidden()`, `conflict()`, `unauthorized()` and
`bad_request()`; use a `Problem` when the client is meant to read the answer (a
validation, a sign-in that failed, a refusal with a sentence):

```rust
Err(Problem::new(StatusCode::UNAUTHORIZED).title(messages.get(MessageKey::SignInFailed, &[])?).into())
Err(Problem::validation(vec![("username".into(), vec![text])]).into())
```

Which of the two a route answers with is part of what the web client sees.
Keep whichever the route has always done.

**Reading a request.** The path, the query and the body go through
`http::Path`, `http::Query` and `http::Json`; axum's own extractors of the same
names refuse with a sentence of plain text, which the clients do not expect.
A path segment that is not a value of its type (an id that is not an id) is a
404 with no body, because the route does not exist; a query that does not fit
is a 400 with no body; a body that is not the right JSON is a 400 with no body.

- `Path<T>` takes one value, a tuple or a struct, by position or name. A segment
  is never empty, so `/api/x//` reaches no route with `{id}` (404). What a
  segment holds is what the path reader left of it (see Request paths):
  `%2F` and any escape that is not a character stay as written, and everything
  else is decoded.
- **An id is read with `Id::parse`**, in a path (`Path<Id>`), a query, a body, a
  header or a hub argument: the 36-character hyphenated form, in either letter
  case, and nothing else (no braces, no parentheses, no missing hyphens, no
  spaces). It is the one reader; do not write another.
- `Query<T>` is a struct (a missing field, a value that does not parse and a name
  given twice are all 400; a name nobody asked for is ignored), or a
  `Vec<(String, String)>` that keeps every pair in order, repeated names and
  empty values (`?a=1&a=2&c` is `[a 1] [a 2] [c ""]`), for a route that reads it
  by hand. `+` and `%20` are a space.
- `QueryValues` reads the query of a route that wants an optional text, number
  or flag by name: `values.text("cursor")?`, `values.int("limit")?`,
  `values.long("after")?`, `values.flag("keepSignedIn")?`. A name matches as it
  is written, a name given twice is refused, a number is ASCII digits that fit
  its type (no sign, no spaces) and a flag is `true` or `false`. What does not
  read is a 400 with the validation problem for that name. No route takes a
  list. Use `Query<T>` where an empty 400 is as good.
- `Json<T>` reads the whole body into memory, up to the ceiling of its route: 1 MiB,
  unless the route says `.json_body_up_to(bytes)`, and never over the 128 MiB of
  the whole server. A route that takes an upload streams it instead.

A body is read with serde's derive and the exact camelCase names the clients send:
a member in another letter case is a member nobody asked for, a member named
twice, a number in quotes, `NaN` or `Infinity`, Base64 with spaces in it and a
leading byte order mark make the body a 400 with no body. Bytes are
`encoding::Octets` (standard Base64 with padding, one spelling). A route of
the panel binds its body with `panel::bind_then_gate::<T>`, a route whose
second step is asked after the body is read (encryption's reset and new PIN)
asks inside the handler. Where a document is kept as it was written (client
settings) or only passed on (a signal's payload) a reader of its own remains,
without the leniency.

**Headers** are not part of any of those, and a handler needs them. Take what the
request says about itself in the form that fits:

- `headers: axum::http::HeaderMap` for any header; a header that may be written on
  several lines is read with `get_all`. Cookies are read with
  `auth::cookies::read(&headers, name)`.
- `Extension(client): Extension<ClientInfo>` for who is on the other end. The
  pipeline puts it on every request, so a handler may rely on it. `client.address`
  is the `IpAddr` the proxies vouch for (see `http/forwarded.rs`: with none
  listed, the last entry of `X-Forwarded-For`; with some, the first one that is not a
  listed proxy). `client.scheme` is the last entry of `X-Forwarded-Proto`, trimmed, as
  it was written (`gopher` and `HTTPS` stay so), `http` if there is none or it holds
  anything but letters, digits, `+`, `-` and `.`, and `client.is_https()` says
  whether it is `https`, in any letter case. `client.host` is the last entry of
  `X-Forwarded-Host`, if a proxy wrote one. Nothing else is filled in: not the
  port, not the `Host` header (read it from `headers` when no proxy said
  anything, as `releases/` does), not the user agent, not the address of the proxy
  itself.
- `Origin` for the three things a session row and a security event record: the
  address, whether the request came over HTTPS, and the browser's name for
  itself; it also carries every header.
- `Caller` and `MaybeCaller` for the signed-in person (see Sessions).

**Answers that are not JSON** (a file, a download, a stream) are built by the
handler as an `axum::response::Response`. The pipeline sets no caching headers
of its own, so `Cache-Control`, `Last-Modified` and the like are the handler's to
put on it; `ETag`, `Content-Type` and `Content-Disposition` too. For a file on the
disk use `http::file::answer(&headers, Download { .. })`, which the releases group
is built on: it opens nothing itself (the handler opens the `tokio::fs::File`,
which does its disk work on a blocking thread, like everything the reader does
afterwards) and answers `Range`, `If-Range`, `If-Modified-Since` and
`If-Unmodified-Since` the way a browser or a download manager expects, with
`Last-Modified` from the moment you give it, `Accept-Ranges`, a
`Content-Disposition` that names the file, and a body that streams in 64 KiB
chunks, two at a time, so memory does not grow with the file. A stream of your
own is made with `http::stream::channel(buffered)`: a task of your own sends
`Ok(bytes)` (it waits while the client is behind, and fails when the client is
gone: stop then) or `Err(error)` (the body ends abnormally and the connection is
closed, so that no client takes it for a whole one), and dropping the sender ends
the body. Use that and not `http_body_util::channel::Channel`, which loses the
last chunks of a sender that ends while the body is being read: small downloads
came back empty once in about eight thousand. Never the whole content in a `Vec`. Disk or CPU work
of any length goes on a blocking thread (`tokio::task::spawn_blocking`, or
`state.database()` for SQLite), never in the handler's own task.

**JSON** is decided in `http/json.rs`: camelCase names (`#[serde(rename_all =
"camelCase")]`), `null` for a missing value (never `skip_serializing_if`), `Id`
and `Timestamp` from `values`, an enum written as the clients expect it for
that enum, members in the order the struct declares them. Never answer with
`json!` or a `Value`: its objects sort their keys. A request body is read with
the `Json<T>` extractor; a body that is not the right JSON is a 400 with no
body. Read a body no other way, so that the ceilings hold.

**Messages** are in `stellarr-messages`. A text for a person is a `MessageKey`
with a text in both languages, which the compiler insists on:
`state.messages().get(MessageKey::PasswordTooShort, &[("min", &10)])?`.

**The database** is reached through `state.database()`. SQLite is blocking, so
the closure runs on a blocking thread and never on one that serves requests.

- `read(|db| ...)` sees one snapshot for its whole length. A statement in it
  that would change something fails; use `write`.
- `write(|tx| ...)` is one transaction that takes the write lock when it begins
  and waits behind other writers instead of failing. It commits when the
  closure returns `Ok` and rolls back on `Err`.
- `try_write` is a `write` that says `None` when a unique value was taken by
  another request after this one had checked it was free.
- `MutationGate` (`state.mutation_gate().enter().await`) lets one change of a
  server-wide invariant at a time through.
- A closure that only runs statements can return `rusqlite::Result<T>`: a failure
  to get a connection comes back as a `rusqlite::Error` too (it is a refused
  conversion in disguise, so do not read anything into its kind). A closure that
  has outcomes of its own may use an error type of its own as long as it has
  `From<DbError>` and `From<rusqlite::Error>`; `DbError::is_unique_violation()` and
  `is_foreign_key_violation()` tell the refusals apart.
- The initial schema is defined in `database/schema.sql`. Do not
  change it. Use the table and column names as they are, in quotes.
- Ids, moments, booleans and enums go into and out of rows through the types in
  `values`: `Id` (capitals in the database, lower case in JSON), `Timestamp`
  (UTC text with a trimmed fraction), `bool` (0 and 1), `int_enum!`.

**A handler runs to the end** even when the client goes away: the pipeline runs
it as a task of its own, so a dropped connection never stops it between its commit
and its event. That is what guarantees the event after a commit is sent; a handler
may rely on it, and no handler may assume it will be told the client left.

**Names and addresses** are stored and compared in capitals (`NormalizedUsername`,
`NormalizedEmail`) through `instance::names`, and a user name's characters are
letters and numbers by general category through `stellarr_unicode`. Both follow
the crate's own tables, which a database written earlier already depends on: the
dotless i is not capitalized, and the long s is. Do not use the standard library's
`to_uppercase`, `to_lowercase`, `is_alphabetic`, `is_alphanumeric` or `is_numeric`
on anything that is stored or compared with what is already stored.

**Events** go to the hub through `state.events().publish(Event::new(audience, "Name",
&payload))`, after `write` has returned. A change is committed first and
announced after; a failed change announces nothing.

**Uploaded files** are objects in `state.storage()`: the folder `files/` of the
data volume or the bucket named by `Storage:*`, chosen when the state is built and
made ready before the server listens (`object_store::prepare`: a store that is
not ready is asked again every second, thirty times; `Storage:SkipInitialization`
skips it). A file on its way in is staged in `incoming/` of the data root, which
is emptied at start. `state.admission().enter(..)` reserves room for an upload
(per-account and instance quotas, free disk space, and a few slots for uploads
that move bytes at once) and gives a lease that is held until the row is written;
anything that stores a file goes through it.

**Settings** are read once at start in `settings/`, and handlers read
`state.settings()`. To add a setting: its default in `defaults.rs`, a row in
`variables.rs` if operators set it as `STELLARR_*`, a field in the section it
belongs to and a line in that section's `load`. A new section is a struct in
`settings/mod.rs` with a `load(config)` that returns `Result<_, SettingError>`,
a field in `Settings`, and its line in `Settings::from_config`; a section that
nothing reads yet needs only its defaults in the table. **A setting that cannot be
right refuses the start**: `load` returns `Err(SettingError::new(..))` with a
sentence that names the setting (`"Stellarr:InstanceId must be a UUID."`), and
the process prints it and exits, so the operator learns at once and not at the
first request that needs it. Do not fall back to a default for a value that was
given and is wrong; a value that was not given may have one. A test for each
refusal goes next to the others in `settings/tests.rs`
(`Server::refused_to_start` is the same check on the binary).

One exception, on purpose: the Web Push settings (`Push:*`, `PushSettings`).
A key pair or contact that is missing, wrong, or not a pair does not refuse the
start; push is then unavailable (`GET /api/me/notification-delivery` says so, and
every send is a retry) and everything else is served. An installation with a
typo in one variable must still run, and a client can see the difference only as
"push is off". `Push:Enabled` that
is not `true` or `false` counts as off.

**State that belongs to one group** (a gate, a cache, a connection pool the
group alone uses) is a type in the group's own folder, held in a field of
`Inner` in `state.rs` and built in `AppState::new`, with an accessor on
`AppState` (`mail()` and `realtime()` are that; so is the registration gate).
That field and its accessor are the one edit a group makes to `state.rs`,
made in a commit of its own. A worker that alone needs something keeps it in the
closure it is started with. Nothing is global: no `static` that a test could
not replace.

**Logging** is `tracing`. A request writes one line under the target
`Stellarr.Api.Requests`; health probes write none.

## The pipeline

Every request goes through `http/pipeline.rs`, in this order: first the target is
read the way `http/path.rs` reads it (see Request paths);
then count the response, log one line, turn a panic into a 500, read the
`X-Forwarded-*` headers (the client's address is `Extension<ClientInfo>`), in
development answer the web dev server's origin, count against the rate limit of
the route, the client gate, refuse a JSON route another content type,
authenticate, the handler. Authentication is `route_checks` in `pipeline.rs`: it
reads the session cookie of every request and checks it against the database
(see Sessions). The router runs before all of them but the first, so each step
knows which route the request is for (`MatchedPath`) and decides on the rules of
that route, never on a spelling of the path.

**A refusal before the body was read reaches the client.** A route that answers
before it has read the body (a 413 on the declared length, a 401 or 403 before
the body, a bad path, a rate limit, a form refused halfway) leaves what the
client is still writing unread, and a server that then closes the socket makes
the operating system send a reset that can take the answer with it: a client
built on hyper (the desktop app is) saw a broken connection instead of the
refusal in about a third of such requests. So every connection ends the same way
(`http/linger.rs`, the listener the server serves on): once hyper has written
the answer and let the connection go, the server sends its end first, then
reads and discards what the client is still sending until the client closes or
a bound is reached: 128 MiB (the ceiling of the whole server, so a refusal never
costs more to read than an accepted request would), 10 seconds from the moment
the server let go (not from the last byte, so a client that dribbles gains
nothing), and 1024 connections at a time (past that a connection is closed as it
would have been). The rule for groups: refuse as early as you like and answer at once; do
not read the rest of a body only to keep the client from seeing a reset, and do not
send less than the declared length in a test to dodge one. A group that reads a
form on purpose after it has decided to refuse (to refuse in a fixed
order, as `uploads` does) is unaffected: the answer is written when the handler
returns, as before.

## Request paths

The path of a request is read before any other code of the server reads it:
once, outermost, in `http/path.rs`, and everything inside sees only the result:
the rate limit, the client gate, the session rule, the body ceiling, the router
and `Path`. The rules:

- A request line of more than 8192 bytes (the method, the target, the version
  and the line break) is a 414; a `%00` in the path is a 400. Both are answered
  with no body and `Connection: close`, before anything else, and are neither
  counted nor logged as requests. A `%00` in the query is left alone.
- `%XX` in the path is decoded, except `%2F`, which stays as the three characters
  written (the case is kept) and so never separates two segments. A byte of
  0x80 or more is decoded with the ones that complete a UTF-8 character; a
  lone, truncated, overlong or surrogate sequence stays as written, and so does a
  `%` that is not followed by two hex digits. Control characters other than
  null are decoded.
- The dot segments `.` and `..` are removed after that, as RFC 3986 says, so
  `%2e%2e` counts, `..%2f` does not, and `..` cannot go above the root.
- One slash at the end is dropped: `/api/x/` is `/api/x`, and a route is declared
  once, without one. Two are not dropped (`/api/x//` is 404), nor one in the
  middle.
- The query is not touched.
- A forwarded scheme is passed on as the proxy wrote it, if it is made of letters,
  digits, `+`, `-` and `.` (`client.scheme`).

The normalised path is written back into the request in the one encoding that
decodes to it again, so `Path<String>` for `/echo/a%252Fb` is `a%2Fb`, and for
`/echo/a%2Fb` it is also `a%2Fb`. `tests/api/pipeline.rs` holds the table of
spellings and what each becomes. Two choices are made on purpose:

- **Letter case.** Routes are matched with regard to case (`/API/x` is a 404).
  Matching without it would mean a table of the literal parts of every route, in
  every group and in the hub's own router, so that a literal is recognised in
  any case while a parameter (a code, a name) keeps its own; that is a router of
  our own beside axum's, not a cheap change, and what it buys is nothing a client
  sends. The client gate does compare its paths without regard to case, which
  only means it refuses more.
- **The gate and the slash.** The gate, the limit and the router read the same
  path, so `/api/session/` is gated like `/api/session`.

A fragment in the target (`/x#y`) is dropped by the HTTP library and the rest is
served.

## Sealed values

`seal.rs` seals what the server gives a client to keep, or writes to the
database and must be able to trust: `state.sealer().seal(purpose, aad, expires,
payload)` and `.open(purpose, aad, text, now)`. The key is derived from a master
key (`keys/master.key` in the data root, made on the first start with mode 0600
and carried by a backup) for the purpose named, so a value sealed for one
purpose does not open as another, and `aad` (the account an id belongs to, say)
has to be the same on opening. A value may expire inside the seal. Name the purpose
for what the value is (`"session-cookie"`, `"totp-secret"`), once, in the module
that seals it. A part that seals on its own, such as the single sign-on library,
gets `state.sealer().derive_key("oidc")`, 32 bytes.

The second step keeps two kinds of secret sealed, each for one purpose: an
authenticator app's seed (`totp-secret`, bound to the account and the method)
and a code mailed to a person (`email-code`, bound to the account and what the
code is for). A row of either kind that was sealed under another master key
(an older server's, say, or one since replaced) does not open here: the app's
codes are all wrong, and the mailed code uses up its guesses. The person signs in
with another step or a recovery code and sets that one up again. Everything else
the second step stores (recovery codes, remembered devices, passkeys) is a
digest or plain, and does not depend on the key.

## The second step

A route never touches the pieces of the second step. For each thing a person
does there is one call in `auth::ceremony`, which owns the transaction, looks
the challenge and the account up inside it, and answers with an enum that says
what to answer; the enums are `#[must_use]`. Every call that takes a challenge
takes an `Ask`: the request's `Context`, the id and secret in the browser's
challenge cookie (`flow::read_preauth`), and the challenge id the client sent.
A challenge that is consumed, out of date, out of attempts, for another
purpose or of an account that changed comes back as `Invalid(status)`: a 429
for `ChallengeStatus::AttemptLimit`, a 401 for the rest. A wrong proof comes
back as `Refused` with what it cost already kept.

| The person | The call | In the order |
| --- | --- | --- |
| signs in with a password and has a second step | `flow::start` (as before) gives the challenge; `pending` and `cancel` for the page that resumes or leaves it | `answer(Answer::Code / TrustedDevice / Passkey)`, and `send_login_code` first for a mailed code, `passkey_options` first for a passkey |
| proves a second step again in a session | `start_from_session(.., Reverification, ..)` | then `answer`, which gives a new session and leaves the old one |
| sets a second step up (a challenge for it from sign-in, from `start_from_session(.., Enrollment, ..)` or from a recovery credential) | `start_totp` then `confirm_totp`; `send_enrollment_code` then `confirm_email`; `start_passkey` then `confirm_passkey` | each `confirm_*` finishes the sign-in |
| signs in with a passkey alone | `passkey_sign_in_options` then `passkey_sign_in` | a rejected answer is kept as rejected |
| takes a second step away | `remove_method` | |
| asks for new recovery codes / forgets a remembered device | `regenerate_recovery_codes`, `forget_trusted_device` | |
| redeems an operator's credential | `redeem_recovery_credential`, then the enrolment calls | |
| changes the password | `recovery::credentials_changed` in the route's own transaction | |
| gives the password as the proof of a change the route also asks `fresh_second_step()` of | `check_given_password` before the write, then `judge_given_password` in it | an account with no password is judged by its sign-in |

`answer` and the `confirm_*` calls take a closure for the caller's own writes
(linking the identity a sign-in was asked to link). It runs in a savepoint of
the same transaction: if it fails, only its writes are undone, the right proof
stays spent and the sign-in stands, and its error comes back beside the
`Completed` (`hook: Result<R, DbError>`) for the route to set the cookies and
then say what went wrong. A failed link must not tell somebody who holds the
password that a guessed code was right. Only something that forbids the
sign-in after all (the challenge spent by another request, the instance's
requirement no longer met) or a failure of the database undoes the proof. A
sign-in that is completed comes back as `Completed`, with the `Set-Cookie`
lines to send, in order: the session's, the remembered device's if one was
made, and the one that takes the challenge's cookie away. A route reads
neither the account nor the session itself to decide what a proof allows: a
remembered device never makes a session fresh and is never remembered again,
a passkey sign-in is a second step passed.

The calls that change the account's security settings from a session
(`remove_method`, `regenerate_recovery_codes`, `start_from_session`) take the
password as `Option<&str>`, never a closure: it is hashed before the write
transaction, and only when the session's own freshness would not do and the
password could decide; the transaction relies on it only if the stored hash is
still the one it was checked against. `pending` and `cancel` show and give up
the challenge the browser holds. What a route may reach from outside `auth` is
what only reads, `recovery::recover` and `recovery::credentials_changed`
(called with `None` by a reset by mail, which has no session to keep), and
`NewSession::insert` for the first-factor sign-ins; `NewSession`'s second-step
fields, challenge creation and the code, device and recovery-code writes are
private to `auth`.

Ids that arrive as text (the challenge id, a method id) are passed through as
written: the ceremonies read them with `Id::parse`. The passkey
calls take `Option<&RelyingParty>`; none says passkeys are not offered
(`PasskeysUnavailable`), after the challenge (and, for adding one, the name)
was looked at, so a route needs no look at the challenge beforehand. A
mailed code is refused for the challenge first (`Invalid`), then the method
(`MethodUnavailable`), then mail (`MailUnavailable`); `Delivered` carries the
address it went to and the seconds before another may be asked for.

Mail is sent by `send_login_code` and `send_enrollment_code`, in the one order
that is right (reserve and commit, send through `mail::deliver`, then mark the
code delivered or spent); a failed mail still counts toward the cooldown and
the hourly limit. Questions about the account (`fresh::can_change_security`,
`ways_in::remain`) are asked of an account id and read the account inside the
transaction they are given.

## The operator's command line

`crates/cli` builds `stellarr`, the tool an operator runs with `docker exec` (or
against a stopped container's volume) to look at an instance and repair it
without the web client: `status`, `doctor`, `config`, `secrets`, `users`,
`sessions`, `security` and `backup`. `docs/admin/cli.md` is what operators were
promised, and this tool keeps it to the letter: every command and option name,
the text on standard output and standard error, the `--json` documents member by
member and every exit status are the ones that document gives. `cargo test -p
stellarr-cli` runs every command, every option and every wrong use on instances
made on disk and checks what each prints and the rows each leaves.

It is a library, `stellarr_cli::run(args, io, environment, cancelled)`, with a
`main.rs` that gives it the terminal, so that the tests give it an input and read
what it printed. It depends on the server's library for everything that exists
there: the settings row and its validation (`instance::operations`, the same the
operator's panel writes through), the sealed secrets (`secrets`), the recovery of
an account (`auth::recovery::recover`), the security events and the ways in
(`auth::events`, `auth::ways_in`), the master key (`Sealer::open_existing`, which
reads a key and never makes one) and the version (`version::server()`). It writes
nothing of `auth` below its ceremonies: a session is revoked by its own `UPDATE`
and an identity link removed by its own `DELETE`.

What a command may do to the data folder:

- It never makes the database, the keys folder or the master key. A missing
  database is `database_not_found` (3); a file that lacks the tables it needs is
  `schema_incompatible` (4), and no command migrates anything. `secrets set`
  needs the keys folder to exist (the server made it) and fails with
  `data_protection_keys_missing` (4) when it does not. It makes `master.key` in it
  only once the question is answered and the value accepted (the server's own start
  makes the same one), and then says so in a warning with the path and the owning
  uid; the question before it says a key will be made. An existing key is read as it
  is: its mode is never changed, and a mode others can read is said in a warning.
  `secrets clear` needs no key at all.
- Reading opens the database read-only and waits five seconds for a writer; a
  write waits the same for the write lock, takes it before it looks at anything
  it will change (`BEGIN IMMEDIATE`) and changes it and its audit row together.
  What it cannot get in that time is `database_locked` (4) and says so; it never
  leaves half of a change.
- A value it prints is never a secret. A secret is read from `--stdin` or a hidden
  prompt (`stty` switches the echo off), never from an argument, and appears in no
  output and no log. The one thing shown once is a recovery credential. When
  `stty` cannot turn the echo off, nothing is read at the prompt (`secret_value_required`,
  2, with the reason); the echo is put back on every way out, and Ctrl+C at the prompt
  is a cancellation, judged before the value.
- A change that someone else made while a confirmation waited is not made again and
  not recorded: `sessions revoke` and `security unlink` write nothing and answer
  `session_not_found` or `identity_not_found` (3). A command that changed something
  and then cannot write its answer (a closed pipe) ends with status 1 and a line on
  standard error saying the change was made; for `security recover`, that the
  credential was not delivered.

What is particular to this tool:

- `doctor` says of `keys` what this server keeps there, a master key
  (`keys/master.key`).
- A secret in the database that this instance's master key cannot open (the key
  was replaced, or the row was altered) is a damaged value, as it is for the
  server: `secrets list` stops with `secret_unreadable`. `secrets set` writes
  over it and `secrets clear` removes it.
- A setting that is seeded from the host's time zone (`time-zone` on an instance
  that never saved one) is the host's; it is spelled as the host's zone database
  spells it.
- The tool says `database` where the server says `panel`.
- `security unlink` says when it removed an account's last way in (a warning, and
  `lastWayIn` as the last member of its `--json` result); `secrets set` and
  `secrets clear` say in their question when the stored value cannot be opened by
  this server.
- `backup restore` refuses when the free space of the data volume cannot be read
  (`free_space_unknown`, 5).

Backups: `backup verify | restore | cancel | apply` are written against
`stellarr_cli::backup::RestoreLibrary`, whose members are (`check`, `staged_archive_path`, `folder_name`, `broken_marker_name`,
`clear_leftovers`, `apply`, `write_result`). `Runtime::create` gives it
`archives::Archives`, which is those members over `stellarr_api::backups::restore`
and nothing else: a restore is written once, in the server's library. The marker of
a restore that broke is `restore/broken`, as the entrypoint looks for it.

In the image, the launcher (`infra/stellarr-cli.sh`) keeps everything it does and
its last line becomes `exec /opt/stellarr/cli/stellarr "$@"` (the binary built with
`cargo build --release -p stellarr-cli`, with `STELLARR_VERSION` set as for the
server, copied to that path). The tool needs `df` and, to hide a typed secret,
`stty`, which the image has.

```sh
cargo test -p stellarr-cli                       # the commands, on instances made on disk
STELLARR_API=... STELLARR_BIN=... crates/cli/tests/running-server.sh # the tool against a running server
```

## Sessions and who is calling

The session cookie holds a sealed ticket (the account, its session version, the
session row), and the row in `UserSessions` decides: revoked, expired, a bumped
session version, a disabled account or a missing second step end it on the next
request. `auth::session` checks it, writes the last-seen time back at most every
five minutes, and moves the session's end out to a full lifetime when it is
near, never past its absolute end. The cookie's own expiry moves out, by its
original length, when the cookie is past half its life. A cookie that cannot be
opened (not ours, altered, run out) is ignored: nobody is signed in and the
database is not asked. A cookie whose session no longer allows it is not signed
in and is taken away.

What ends a session in the database ends it for the next request that carries
its cookie, but not for a connection the hub already holds open. The hub closes
those in two ways: handlers tell it (`state.events().session_ended(user, session)`
after a sign-out or a revocation), and a worker, `realtime-sessions` in
`realtime/watcher.rs`, looks at the database once a second, so that sessions revoked, expired, past their absolute end, of a
disabled account or no longer meeting the second-step policy close whoever changed
them, another process included. Both use `auth::session::authenticate`.

- `.authorized()` on a route (after `.get(..)` and the like): a request without
  a valid session is answered 401 with no body before the handler.
- A handler on such a route takes `caller: Caller` (`caller.user`,
  `caller.session`). A handler on an open route that answers both takes
  `MaybeCaller(caller)`.
- To sign somebody in, a handler writes the session with `auth::flow::start`
  (the one place every way of signing in goes through: it gives a session, or a
  challenge when a second step or an enrolment is needed, never both) and
  answers with `with_session_cookie(response, &auth::session::sign_in_cookie(..))`.
  To sign out it revokes the row (`auth::session::revoke`) and answers with
  `sign_out_cookie`.
- `Origin` is the address the proxies vouch for, whether the request came over
  HTTPS and the browser's name for itself, for the session row and the security
  event.

## Permissions

`permissions/` holds who may do what on a server and who sees which channel,
(the rules are in `docs/architecture.md`, under
"Server authorization" and "Private channels"). Every decision is the one pure
function in `permissions::evaluator`; `snapshot` loads what it reads;
`authorization` and `visibility` are what a handler calls. Nothing is cached:
every function takes the connection of the `read` or `write` closure it runs in
and asks it, so a change of roles or overrides counts from the next request on.
A handler never decides who may do something by itself, and never opens a
connection for it.

## Mail

A kind of mail asks `mail::setup::find` (or `mail::deliver`, which does the
whole of it for a message a request waits on) for the sender, the instance's
address and the transport the operator chose, builds its message with
`stellarr_mail`, and gives it to `state.mail().send(..)`, which answers with an
`Outcome` (`Sent`, `Refused`, `TryAgain`, `Off`, `NotConfigured`). A request that waits on
mail answers from the outcome; a queue records it on its row and tries again
later, as the worker `mail::security_alerts` does. Nothing logs a password, a
key, a link or the text of a message.

## Secrets

`secrets/` keeps the secrets an operator sets in the panel (the SMTP password,
the Resend key, the keys of the integrations), sealed (see Sealed values) in the
database. A value set there wins over the environment's; values only go in,
nothing gives one back to a client and nothing logs one. A row that cannot be opened
(another master key, altered) is a damaged value: reading the secrets fails with
it, until the operator clears it or restores the key. The functions take the connection or transaction of the caller, so that what
else commits with a change (its audit entry) is the caller's to decide.

## Background workers

Work that runs beside the requests from start to stop (a watcher of ended
sessions, the queue of mail, link previews, housekeeping every hour) is a
`Worker`. A module declares its workers in one function and writes its path on a
line of `src/workers.list.rs`, the one line it shares with the others; the file
merges by itself, as the list of groups does.

```rust
// src/instance/census.rs (this example is compiled as a doc test of `background`)
use std::time::Duration;

use stellarr_api::background::Worker;
use stellarr_api::database::DbError;
use tracing::{info, warn};

pub fn workers() -> Vec<Worker> {
    vec![Worker::new("census", |state, stop| async move {
        // `while stop.sleep(..)` is a timer that ends with the server.
        while stop.sleep(Duration::from_secs(3600)).await {
            let counted = state
                .database()
                .read(|connection| -> Result<i64, DbError> {
                    Ok(connection.query_row("SELECT COUNT(*) FROM \"Users\"", [], |row| row.get(0))?)
                })
                .await;
            // An iteration that fails is logged, and the next one tries again.
            match counted {
                Ok(accounts) => info!("{accounts} accounts"),
                Err(error) => warn!("the census failed: {error}"),
            }
        }
    })]
}
```

```
// src/workers.list.rs
workers! {
    instance::census,
}
```

The rules:

- It starts after the database is open, the state is built and the server
  listens, and it never delays any of them. It is given an `AppState` (the one
  the handlers have) and a `Stop`. Do nothing at the top of the function that
  waits for something slow before the first `.await`; the function is a future.
- It reads and writes through `state.database().read` and `.write`, like a
  handler, and tells the hub through `state.events()`. It does not open a
  connection of its own.
- It stops when told: `stop.stopped().await` resolves when the server is
  stopping (put it in a `select!` with what else the worker waits on),
  `stop.is_stopping()` says it between the steps of a long iteration, and
  `stop.sleep(time)` is a wait that ends early. Do not wait on anything
  that does not end without a timeout.
- The server waits for all the workers together, within the same shutdown
  grace as the requests in flight (30 seconds). A worker still running then is
  abandoned: its task is aborted at its next `.await` and its name is logged as
  an error. Work that never reaches an `.await` cannot be aborted; keep a long
  blocking step inside `database.read` or `.write`, which runs on its own thread.
- An iteration that fails is the worker's own business, as in the first
  server's workers: catch the error, log it, go on. A panic is the net under
  that: it is logged with the worker's name and what it said, the worker is
  started again after a pause that doubles from one second to a minute (and
  starts over after a run of a minute), and nothing else is touched. A restart
  is a new call of the function, so state kept in the old run is gone; what must
  last is in the database. A worker that returns before it was told to stop is
  finished and is not started again, which is how one that a setting turns off
  ends.
- A worker that must not run in tests (a timer that would race them) checks
  `state.settings().environment.is_testing()` itself, or is left out by the
  harness as below.

In tests, `Server::in_process_with(routes, Services { workers, shutdown_grace,
..Services::default() }, &[])` starts a server with exactly the workers given,
and a short grace if one wants to see a worker abandoned; `Server::in_process`
and the library's `serve` have none. The binary the other tests start
(`Server::start`) runs none either: in the Testing environment only the workers
named in `Testing__Workers` (comma-separated) run, so
`Server::start_with(&[("Testing__Workers", "census")])` runs that one.
See `tests/api/workers.rs` for a worker that is told to stop, one that panics
and one that does not stop.

### The notification worker

`delivery` (`notification-delivery`) sends what the outbox owes to people who are
away, by Web Push and by mail, every thirty seconds; `delivery/dispatcher.rs` has
its rules. A batch is up to 25 rows, leased for as long as the batch can take at its
worst (25 rows of 20 seconds) and sent one after another.

Its limits: the rows of a batch are sent one at a
time, and a push service has ten seconds to answer, a mail server fifteen. So one
person's subscriptions that never answer can delay everyone's notifications by up
to 250 seconds a batch. That goes away with a later change to how the server's
queues work, not with this worker. The sender only ever connects to public
addresses; nothing a setting or a request says changes that.
## The administration panel

`panel/` is what the routes under `/api/cockpit` stand on. The gate is two
extractors, and a route is declared `.authorized()` and nothing else:

- `permissions::membership::InstanceOwner(server)`: the owner of the instance
  server (the first one ever made) and nobody else; 403 with no body for
  anybody who is not, and for an owner whose policy asks a second step of the
  panel and whose session has not passed one, 403 with the code
  `cockpit-mfa-required`.
- `panel::FreshInstanceOwner(server)`: the same, and then a second step passed
  in the last five minutes where the account has one (`fresh-mfa-required`).
  Do not mark such a route `.fresh_second_step()`: the pipeline would ask the
  second step of a caller who is not the owner before the owner check, and such
  a caller is answered 403.
- A route that reads a JSON body calls `panel::bind_then_gate` and is marked
  `.json_body()`: the body is bound before any filter, so a body
  that is not the right JSON is a 400 and another content type a 415 for
  anybody, before the gate.
- `panel::record(tx, server, actor, sentence)` writes the audit line of a
  change the operator made (action `operator`, in the transaction of the
  change), and `panel::operations` reads and writes the row of retention and
  clean-up settings that the first change of any setting creates.

Both gates are tested in `tests/api/backups.rs` (a member, nobody, an owner
without and with a second step, a disabled owner, the second step passed at the
bound).

## Backups

`backups/` keeps archives of the instance in `{data root}/backups`: a plain
SQLite copy (`backup-<time>-<token>.db`) or a full archive (`.zip`) of the
database, every object of the store, the keys folder and the instance files
(`secrets/`, `tls/`, `releases/`). The archive is a file an operator keeps, so
its layout is fixed and is not changed: `manifest.json` last,
entries in the order `database`, `objects` (in key order), `keys`, `instance`
(folder by folder, each by the whole path as text), objects stored and the rest
deflated. Archives that follow this layout can be verified and restored.

- The database is copied with SQLite's online backup in one step, so a backup
  taken while people write is a state the database really had
  (`backups/snapshot.rs`; `tests/api/backups.rs` writes throughout one).
- One backup runs at a time (`state.backups()`); it is written under a name
  starting with a dot and ending in `.partial` and renamed when whole. The
  partial files a stop left are removed before the server listens.
- The automatic backup is the worker `automatic-backups` (`backups/schedule.rs`):
  `run_once(state, destinations, clock)` is the whole of a pass and takes the
  clock and the destinations, so the schedule is tried at any moment; the
  worker is a timer around it. It keeps `automatic-backup.json` (PascalCase) so that a failed offsite copy retries the archive
  already made, after a restart too.
- The offsite destination is `backups/offsite.rs`: a bucket of S3 API
  (HEAD first, `sha256` metadata, multipart, nothing overwritten) or the relay
  Worker. Its credentials are the stored secrets (`cloudflare.r2.*`); the
  destination's other settings and the schedule are rows of `InstanceSecrets`
  sealed for their own purposes (`backups.policy`, `cloudflare.r2.policy`). A
  row that cannot be opened is a damaged value, and an error. Nothing here puts a credential, or the text a store sent back, in an
  answer or a log line.
- The routes (all under `/api/cockpit`): `GET backups`, `POST backups`,
  `POST backups/{id}/verify`, `GET backups/{id}/download` (a fresh second step),
  `GET|PUT|DELETE backups/settings` and `GET|PUT|DELETE
  integrations/cloudflare/r2` (the changes ask a fresh second step).

### Restoring

`backups::restore` is what the operator's tool calls (`crates/cli/src/archives.rs`
puts it behind the tool's `RestoreLibrary`): `check(archive, work_folder)`,
`staged_archive_path`, `clear_leftovers`, `apply(staged, data_root, now)` and
`write_result_text`. The tool does the rest itself: asking the operator, copying the
archive to `restore/staged.zip`, dropping it, keeping a refused archive as
`refused-<time>.zip`, and writing the `restore/broken` marker (exit status 4 for a
refusal). `restore::stage`, `cancel` and `write_result` are the library's own
versions of the first three and the tool does not call them.

The entrypoint (`infra/entrypoint.sh`) is what happens at start around a staged
restore: it fails if `restore/broken` exists, and runs `stellarr backup apply
--yes` when `restore/staged.zip` exists, treating exit status 4 as "refused, the
instance starts with its previous data". Nothing in `init/` knows about restores.
`restore::clear_leftovers` is called by `stage` and `apply`.

A restore checks everything before it changes anything (manifest, every entry
name, the database, every byte), refuses an archive that is not a full backup,
and unpacks beside the live folders and swaps them whole, keeping the previous
ones in `restore/previous-<time>`. An entry that is a link, a device or a
folder is refused (a link is never unpacked as a file). Files are only created, never opened over; keys,
secrets and tls files are 0600.

**What a restore needs.** The sealed values (the secrets set in the panel, the
schedule and destination of backups, an authenticator app's seed, a mailed
code) open only under the master key they were sealed with, which a full backup
carries in `keys/`. Restored without it they are damaged values and an error:
clear the secrets with `secrets clear` and set the rest up again.

A restore ends, in the restored database and before it puts it in place, everything that a row made before the archive could use to let somebody in or past a step (`auth::restore::end_what_a_copy_may_bring_back`, one transaction). It ends every running session and every remembered browser (the second step is asked again); every mailed code, password-reset link, mail-change link and operator-printed recovery credential that was not yet used (one used after the archive was made would otherwise be good again); and the period of every confirmed authenticator app up to the current one. It removes open sign-in challenges, passkey sign-in ceremonies, desktop sign-ins with their approvals, and identities waiting to be linked. It leaves the account's own credentials as the archive holds them (password, recovery codes, passkeys, the methods of the second step) and does not raise any account's `SecurityVersion`, because every row that carries one is ended directly. A session or a browser that was ended after the archive was made does not come back. The tool says in a fixed sentence, before it asks, that a restore replaces every account, session, setting and secret and where automatic backups are sent.
The encryption tables (`AccountKeyBackups` and the rest) are encrypted by the
client with the person's PIN and are unaffected. `instance/` in an archive holds
only `secrets/`, `tls/` and `releases/`; `instance-id` is not part of it.

## Housekeeping

`housekeeping/` is the worker `housekeeping`, once an hour after a first wait
(`Maintenance:*` settings). A run, in this order: compacts the message journal
(older than `MessageChangeRetentionDays`, 7 to 365, leaving one reset entry per
affected channel), and then, unless the operator switched automatic cleanup
off: removes the uploads nobody refers to (older than the orphan retention,
`BatchSize` at a time, each in a transaction that looks again whether anything
refers to it), empties the deletion queue (`BatchSize` oldest first; an object
the store cannot delete stays queued), prunes `backups/` and `logs/` by file
time (backup and log retention), and deletes the records of signing in that are
past use (a day after expiry for challenges, mailed codes, recovery credentials
and pending identities; thirty for trusted devices and for used recovery codes
and sent notifications). With cleanup off only the journal and the queue are
touched. The settings are those of the row written last. A delete is made a
thousand rows at a time in transactions of their own (an orphan and a queued
object one at a time), with a pause of a few milliseconds between them for a
request that waits for the write lock to get in, so the one writer is never
held for a whole run. Measured on 300,000 journal rows and 350,000 expired
records (`housekeeping::tests::cost`, ignored), the longest transaction of a
run took 16 ms; a task that wrote one small row every millisecond beside it
waited 0.16 ms at the median, 5 ms at the 99th percentile and 0.43 s at worst, on
a machine busy with other work.
`housekeeping::run(state, now, stopping)` takes the
moment its ages are measured against.

## The realtime hub

`realtime/` is the hub the web client keeps open at `/hubs/app`. `run` starts it
(`Services { realtime: Some(Realtime::new()), .. }`); `serve` alone runs the API
with nobody connected. The web client upgrades directly with the `stellarr.v1`
WebSocket subprotocol. The server checks the session cookie and browser Origin,
then sends `ready`. Each text message contains one JSON object. Clients send
`call` messages addressed by `id` and receive `result` or `error` with the same
`id`. The hub pushes `event` messages with a `name` and `data` and sends `ping`
messages during idle periods. A revoked session closes with code `4401`.
The upgrade requires the `stellarr.v1` subprotocol. Tabs left open from a
release that used SignalR must be refreshed after updating the server.

Connections join the groups of their account, the server they follow
(`?serverId=`, else the first they belong to) and their account on that server.
Presence goes online with the first connection and offline after the last.

Handlers tell the hub through `state.events()`, and read it through
`state.presence()`. An `Audience` says who hears an event, and the handler
works it out, since no group stands for who may see something:

| Audience | Who |
| --- | --- |
| `Server(id)` | every connection that follows the server |
| `Users(ids)` | every connection of each of these people |
| `Followers { server, users }` | the connections of these people that follow the server |
| `Reach { servers, users }` | the connections that follow any of these servers and every connection of these people: a change of status |
| `Everyone` | every connection |
| `Connection(id)` | one connection |

An event goes to all the groups of its audience in one send, so a connection
that is in several of them, or a person named twice, hears it once. The payload
is sent as written. `state.events().session_ended(user, session)` closes the
connections of a session and is not passed on; `server_left(user, server)` takes
a person's connections out of the server's groups. In the Testing environment
every event is also appended to the event file (`server.events()`).

A group adds methods the client calls, and work for a connection that starts
or ends, from its own folder: a function returning `realtime::Registration`, and
its path on a line of `src/realtime/methods.list.rs`, which merges by itself.

```rust
// src/channels/hub.rs
pub fn registration() -> Registration {
    Registration::new()
        .method(Method::new("SendTyping", |cx, arguments| async move {
            let [channel] = arguments::exactly(arguments)?;
            // cx.state(), cx.user(), cx.realtime().throttle(), cx.hub() ...
            Ok(Value::Null)
        }))
        .on_disconnected(|cx, _error| async move { /* free what the connection held */ })
}
```

A call to a method nobody registered is answered as for one that does not
exist. A refusal that the person should read is `HubError::visible(..)`, anything
else is hidden behind a general sentence. The session watcher is a worker,
`realtime-sessions`: run it in a test of the binary with
`Testing__Workers=realtime-sessions`.

## Tests

Unit tests sit next to the code in `#[cfg(test)] mod tests`. They get their
folders and databases from `crate::testing` (`TempDir`, `new_database()`), which
exists only in test builds and names its folders `stellarr-unit-*`.

Endpoint tests are in `crates/server/tests/api/`, one file per group, all
linked into one test binary (`modules.list.rs` lists them, one `mod` line to a
file, merged the same way). Each test starts the real
`stellarr-api` binary on a free port with a temporary data folder, waits for
`/health/ready`, and stops it afterwards. Tests run in parallel and share
nothing.

The data folders are made under the temporary folder, and SQLite syncs every
commit to them. With `TMPDIR` on a filesystem in memory the same tests take
about a fifth less time; CI mounts one for them. It must allow running files,
which Docker's `/dev/shm` does not: the documentation tests are built there
and run from it, and a few tests run a script they wrote there.

```rust
// tests/api/channels.rs
use crate::support::Server;

#[tokio::test]
async fn a_channel_is_read_by_its_id() {
    let server = Server::start().await;            // the binary, in the Testing environment
    let client = server.client();                    // says it is the web build; keeps cookies

    let response = client.get("/api/channels/8f62a4d1-bdcb-4d86-bd1c-6e351900e629").await;

    assert_eq!(response.status, 404);
}
```

The harness is `tests/api/support/`:

- `Server::start()` is the binary; `Server::start_with(&[("Desktop__MinimumVersion",
  "0.1.3")])` adds variables to its environment (the names are the ones operators
  use, and a name the harness sets itself is replaced). In the Testing environment
  no worker runs unless named in `Testing__Workers` (see Background workers).
- `Server::in_process(routes, &[...])` runs the library in the test's own process
  with extra routes of the test's own, for what the real groups cannot do on
  request: a route that fails, that is rate limited, that reads a body.
  `Server::in_process_with(routes, Services { .. }, &[..])` also chooses the workers
  and the shutdown grace. See `tests/api/pipeline.rs` and `workers.rs`. The log of
  such a server is read with `LogCapture::start()`; the log of the binary with
  `server.log()`.
- `server.client()` says it is the web build and keeps cookies; `server.anonymous()`
  sends no client headers; `server.client_of("web", "2.0.0")` sends the ones you
  name. `client.get(path)`, `client.post_json(path, &body)` and
  `client.send(Request::new(Method::POST, "/path").json(&json!({...})))` for
  anything beyond them.
- `server.found("owner").await` founds the instance (the setup code is read from
  the binary's log) and returns a `Person` who is signed in; `server.register("maya")`
  makes an invite and registers with it; `server.sign_in(&person)` is a second
  sign-in with a client of its own; `server.login(&client, name, password)` is the
  bare request. Every person has the password `support::PASSWORD`. `server.database()`
  is a connection of the test's own to the server's file, to arrange or to look;
  `server.database_path()` is the file.
- `server.events()` is every event the server published, as JSON.
- `server.folder()` is the data folder of the server (the files a group reads or
  writes under it, `releases/` for one). `Server::start_in(folder, &[..])` starts
  another binary on a folder that is not its own and is not removed with it, for
  a second start on the same files; `Server::refused_to_start(&[..])` runs the
  binary that is expected not to start and returns what it said on stderr;
  `server.stop().await` sends SIGTERM like the container's manager and says
  whether the process left cleanly; `server.pid()` is the process id (none for
  one run in process), for a test that looks at what the process holds.
- There are two `TempDir`s and they are not interchangeable: `support::TempDir`
  (folders `stellarr-it-*`) is for the integration tests, `crate::testing::TempDir`
  (`stellarr-unit-*`) for the unit tests inside the crate, which cannot see
  `tests/`. Both are removed when dropped.
- `support::raw_get(address, target_bytes, &[(name, value)])`, `raw_send` (any
  method) and `raw_request` (the bytes of the whole request) send what a client
  cannot be made to write: a `%00`, a backslash, a byte that is not UTF-8, a line
  of any length. They answer with an `Answer` (`status`, `headers`, `body`,
  `header(name)`, `text()`); a status of 0 means the connection ended before a
  whole head was read.
- **`serde_json::Value` sorts the keys of an object**, so a test of the order of
  the members of an answer (which the web client may depend on) cannot read it into
  a `Value`: compare the text, or read the order with a reader that keeps it, as
  `releases.rs` does (`member_order`).

The Testing environment lifts the rate limits (ten thousand a window) and lets
the session cookie work over plain HTTP. To test a limit, start in Production
with the limit set, as `pipeline.rs` does.

## Crate features

The server asks for the features it uses in its own `Cargo.toml`, and the workspace
keeps one version of each crate (`[workspace.dependencies]` in `api/Cargo.toml`, no
features there). What is on today: `axum` with `tokio`, `http1`, `matched-path` and
`query` (no `json`, no `form`, no `multipart`, no `ws`: the hub does its own
WebSocket); `tokio` with `rt-multi-thread`, `macros`, `net`, `sync`, `time`, `signal`,
`fs` and `io-util`; `http-body-util` (no features); `hyper` for `Body` and `Frame`; `rusqlite` with `bundled` (which
brings FTS5) and `backup` (the online copy of a database that is written to), and `trace` in tests; `serde` with `derive`; `serde_json` with
`raw_value`; `rustls` with `aws-lc-rs`, `std` and `tls12`; `tracing-subscriber` with
`env-filter`; `aws-lc-rs`, `bytes`. A crate already in the workspace may be asked
for one more feature in the manifest that needs it (a feature is not a new
dependency); a crate that is not in the workspace is not added without
asking.

## Version

The server reports one version, from `version::server()`. It is what the image was
built as: the build sets `STELLARR_VERSION` in the environment of the `cargo build`
that compiles the server (the container build passes its `VERSION` argument), and
without it the version is the workspace's `version` in `api/Cargo.toml`. A plus sign
and what follows it (a revision, `1.4.0+3f9c2e1`) is not part of it. `build.rs`
makes cargo build the crate again when the variable changes. Nothing else reads the
variable, and nothing else spells the version: the discovery document takes it from
here.

## Limits of voice

These are not fixed here, and are written down so that nobody takes them for
guarantees. (A seat that outlives the right to sit there is no longer one of
them: see below.)

- Sessions to close wait in a queue of 4,096. Past that the oldest is dropped
  and nothing reconciles the media server afterwards, so a dropped session lives
  until the media server is reset. The queue is worked from the front, so one
  close that always fails holds up the ones behind it.
- The credentials for the relay are per account, valid for an hour, and cannot be
  withdrawn before that.
- A malformed event from the media server (not an object, or without a type or
  a session) makes the whole answer unreadable: the pump asks again from the same
  place and stalls on it, as long as the media server keeps it in its log.

## Who sits in a voice room

A seat is given where the person may view and connect, and it is taken back when
they may not. A change of access calls `voice::enforce(state, server)` once it is
committed (`voice::unseat` and `Unseated::announce` where the seats must go
first and the room be told last, as for a removal), which asks
`permissions::authorization::held_at` what each seated person holds at the channel
they sit in and takes out whoever may no longer view and connect: the seat and
the media session are gone, the room gets `VoiceParticipantChanged`, and the
person gets `VoiceChannelKicked`, as from a moderator's voice kick. The paths that
call it: a role's edit, deletion, giving and taking, and the overrides of a role
or a member on a channel or a section (`roles::Announcement::publish`), the
panel's edit and giving of a role, a channel moved to another section
(`channels::order`), a channel deleted (`channels::channel::delete`), a kick and
a ban (`moderation::announce_removal`), a server handed over or deleted, by its
owner or from the panel. A section can only be deleted when it holds no channel,
so nobody sits in one that goes. A closed connection frees its seat by itself, so a
disabled account or an ended session needs nothing more. A moderator's move is
no loss of access, but a person moved into a room they could not have joined is
taken out of it by the next change of access on the server, since the rules
count from then on. A seat does not stay until the person leaves: it goes with
the right to sit there.

## Behaviour worth knowing

What this server does where a client, an operator or a reader of the code might
expect otherwise.

- A backup restore refuses an archive entry that is a link, a device or a folder.
  Backups follow no links when they read the instance folders. The backup list
  dates a file by the earlier of its two change times on Linux (a file copied in
  keeps its old date), and by its birth time on other systems.
- The housekeeping deletes in chunks of a thousand rows.
- Routes are matched case-sensitively (see Request paths for why that stays). The
  client gate still treats its paths case-insensitively.
- The client gate, the limits and the router read the same path, so a path with a
  slash at the end (`/api/session/`) is gated like the one without. A fragment in
  a request target (`/x#y`) is dropped and the rest served.
- A request body's property names must have the case the client sends.
- The encryption routes write non-ASCII text as it is (a device label of emoji,
  not `\uD83D\uDE00`); both are the same JSON. A `null` among the envelopes of a
  new epoch is a 400.
- What encryption does not guard: someone who holds a session can try the
  account's password through the two routes that ask for it (a new PIN and a
  reset), at the rate the `Login` policy allows. A wrong try costs what one at
  sign-in costs (`events::record_wrong_password` and
  `password::wrong_password_wait`): a `login-failed` event with the address and
  browser, and a wait of 80 to 179 milliseconds before the answer. Like sign-in,
  it does not lock the account. A session can also use up the account's unlock
  tries and, after ten wrong PINs, delete the backup; and a device is registered
  without the PIN, which the other person learns from the device line and the
  keys-changed event. The owner may decide later to tighten any of that.
- Responses with a known length carry `Content-Length` and are not sent chunked,
  and a response sent before the request body was read may carry
  `Connection: close`.
- Settings are read once at start.
- A route that reads JSON reads 1 MiB unless it asks for more
  (`.json_body_up_to`), so nobody can make the server hold more on a route that
  needs no session. The 128 MiB ceiling of the whole server stays.
- A malformed JSON body is a 400 in every environment.
- A release manifest that names a file with a null character in its name is
  unusable (503).
- A route that takes an upload answers 403 to somebody who is no member of the
  server before it reads their form.
- An upload is staged in `incoming/` of the data root and not in the system's
  temporary folder. A form is read by `http::multipart` within the limits set
  there, and a text field longer than 64 KiB is read as a value too long to be
  anything. A client that sends nothing for six seconds is cut off with a 408.
- The form that approves a desktop sign-in is read up to 1 MiB; a form over that
  is answered 500.
- A poster and a preview are sent with `X-Content-Type-Options: nosniff`.
- The free space of the data volume is asked of `df` (two seconds at most); the
  standard library cannot say, and `unsafe` is not allowed. **The image must
  contain `df`**: where it is missing, fails or is too slow, a warning is logged
  at every upload and the reserve (`Uploads:MinimumFreeDataRootBytes`) is not
  enforced.
- The answer of one poll of the media server's events may weigh up to 64 MiB
  (256 events of up to 128 KiB of session description, with room to spare). A
  longer answer is read as an unreachable media server.
- A voice signal's payload and a media signal's payload are read as JSON by this
  server's reader, which refuses a lone surrogate escape (`"\ud800"`). A client
  that sends one is told "Invalid voice state." (or "Invalid media signal.").
- A signal object that has two members whose names differ only in letter case
  (`kind` and `Kind`) is read by the order of the names, not of the document, so
  the member that wins can be the other one. No client writes such an object.
- A move request's body in the Development environment that is malformed is
  answered 400, as in every environment.
- The operator panel's runtime figures (memory, threads, processor use) are read
  from `/proc` and are zero where there is none (macOS); the "heap" is the
  process's mapped data. The API service's detail says `Rust <version>`.
  Characters outside the basic plane are written raw in JSON. A test mail to an
  address the SMTP client cannot write into a message (a quoted part before the
  @, an address literal, one over 254 bytes) is answered 502 with the reason.

## Single sign-on: what to know

What somebody would notice about the flows of `identity_providers`:

- **An unsigned ID token is refused.** A provider that signs nothing (`alg:
  none`) cannot sign anybody in. Only a provider that is set up to send unsigned
  tokens notices.
- **A return address of over 2,048 characters, or with a character beyond
  ASCII, becomes `/`.** The person lands on the start page instead of the page
  they were headed for.
- **A link answered in another browser, or whose state could not be opened,
  ends on the sign-in page** (`/login?oidc_error=failed`). The sealed state opens
  only for the browser that asked, so it cannot say what the round trip was for;
  that would take showing it to everybody. The person sees the sign-in page with
  an error instead of the link page's. Once the state has opened (no code,
  expired, used), the page is the link page's. The same holds for a fresh
  sign-in answered a second time: the sign-in page (the answer that came first
  was the one that counted).
- **`HEAD` on the callback is a 405.** No browser sends one.
- **The provider's documents have their own timeouts**: ten seconds to
  connect and thirty in all, five redirects for discovery and keys, none for the
  token and profile requests. A provider slower than that fails the sign-in.
- **A secret kept in a file is read once, at start.** A file created later does
  not make the provider appear; the operator restarts.
- **A custom `Oidc:CallbackPath` refuses the start.** The callback is a route
  built at start; an operator who moved it must register the default address
  with the provider.
- **A session that ended before a fresh sign-in is answered** is refused by its
  expiry too.
- **No orphan session** when a provider's profile fails after a linked account
  was found: no session and no event are written before the profile is read.
- **Cookie names**: one cookie for each sign-in under way, `stellarr-oidc-` and a
  hash of its state. Attributes: path of the callback, `Secure`,
  `SameSite=None`, `HttpOnly`.
- **The binding cookie has no `__Host-` prefix**, because it is limited to the
  path of the callback, and that prefix demands the root. What that leaves open:
  a page on a sibling subdomain could plant such a cookie in a browser. It could
  only make that browser's own answer fail, since the state it opens is sealed
  to the value that browser holds.
- **The replay guard lives in memory.** A restart forgets which answers were
  used, so for the minutes a state lives, an answer could be posted twice; the
  provider's code is single-use, and its second redemption fails.
- **An unknown key id fails that sign-in and the next one succeeds**: the
  provider's documents are read again after the failure (once in five
  minutes), not before the retry. A person whose provider rotated its keys
  signs in on the second try.

## Measuring uploads

`tests/uploads-probe.py <base-url> <setup-code> [database-file]` sends one corpus
of requests (every type, names, limits to the byte, forms that lie, ranges,
conditions, who may read what, the pictures of people and servers) to a fresh
server and prints one normalized line per request; the database file is for a
server that cannot make an invite itself. `UPLOADS_PROBE_HEAVY=1` adds the
100 MiB cases. `tests/uploads-real-bucket.sh` runs the corpus against the
folder store and then against real S3-compatible servers (Garage and the
Versity gateway, from images already on the machine) and requires the same
answers. `tests/uploads-memory.py` measures the memory of the server while
uploads pass.
