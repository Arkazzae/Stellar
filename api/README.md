# The Stellarr API

The server behind the web, desktop and Android clients, written as a Cargo
workspace. One crate per folder of `crates/`:

| Crate | What it is |
| --- | --- |
| `archive` (`stellarr-archive`) | A zip writer and reader for backups: streaming, ZIP64, every checksum checked. |
| `deflate` (`stellarr-deflate`) | DEFLATE, gzip and zlib in both directions, and CRC-32, with no dependencies. |
| `discord` (`stellarr-discord`) | Reads a Discord server, from a template link or with a bot token, into a snapshot in Stellarr's own terms. |
| `embeds` (`stellarr-embeds`) | Link previews: finding the links in a message and fetching what a card for each needs. |
| `hub` (`stellarr-hub`) | The server for the SignalR hub protocol that the web client speaks: negotiation, the JSON protocol over a WebSocket, calls, events to connections and groups, and keep-alive. |
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
| `unicode` (`stellarr-unicode`) | Unicode general categories and simple upper-casing, as the first server read them, and the UTF-16 view of text that message code works in. |
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

```
lib.rs         serve(); groups.list.rs is the list of groups
main.rs        reads the environment, starts logging, runs
settings/      everything read at start
database/      SQLite: pool, schema.sql, errors, the write gate
values/        Id, Timestamp, int_enum!: how a value is stored and written
http/          the pipeline and the tools a group is built from
events.rs      what handlers tell the realtime hub
state.rs       what every handler can reach (AppState)
logging.rs     where the log goes
health/        a group: /health/live and /health/ready
```

Shared plumbing (`settings`, `database`, `values`, `http`, `events`, `state`)
is not edited to add an endpoint. If a group needs something the plumbing does
not have, change the plumbing in its own commit.

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
That is the only shared line a group touches, and the file holds nothing else;
`.gitattributes` makes git keep both sides' lines when two branches each add one. Do not edit `main.rs`, the
pipeline or another group.

`Endpoints` has `get`, `post`, `put`, `patch` and `delete`. Two modifiers apply
to the route added just before them:

- `.json_body()` for a route that reads a JSON body: a request that sends another
  content type is answered 415 before anything counts it.
- `.rate_limit(RateLimitPolicy::AuthWrite)` for a route that counts against one
  of the named limits.

Only the methods a group names are served. `HEAD` is not answered for a `GET`
route; it is a 405.

## The rules every group follows

**Handlers** return `ApiResult<T>`. `?` turns a database error or a message with
a missing value into the 500 problem, which says nothing of the cause and logs
it with the request's trace id. Use `ApiError::not_found()`, `forbidden()`,
`conflict()`, `unauthorized()`, `bad_request()` for the answers that have a
status and no body, and `Problem` for one that has a body:

```rust
Err(Problem::new(StatusCode::UNAUTHORIZED).title(messages.get(MessageKey::SignInFailed, &[])?).into())
Err(Problem::validation(vec![("username".into(), vec![text])]).into())
```

Which of the two a route answers with is part of what the web client sees.
Keep whichever the route has always done.

**Reading a request** goes through `http::Path`, `http::Query` and `http::Json`
and nothing else; axum's own extractors of the same names refuse with a
sentence of plain text. A path segment that is not an id is a 404 with no body
(the route does not exist), a query that does not fit is a 400 with no body,
and a body that is not the right JSON is a 400 with no body. `Json<T>` reads the
whole body into memory, up to the ceiling of its route: 1 MiB, unless the route says
`.json_body_up_to(bytes)`, and never over the 128 MiB of the whole server; a route
that takes an upload streams it instead, through `http::read_all`'s limit, once it
is written.

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
- The schema is `database/schema.sql`, kept as the first server made it. Do not
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
letters and numbers by general category through `stellarr_unicode`. Both have to
agree with the first server letter for letter, because the two are meant to work
on copies of one database: the dotless i is not capitalized, and the long s is. Do
not use the standard library's `to_uppercase`, `to_lowercase`, `is_alphabetic`,
`is_alphanumeric` or `is_numeric` on anything that is stored or compared with what
the first server stored.

**Events** go to the hub through `state.events().publish(Event::new(audience, "Name",
&payload))`, after `write` has returned. A change is committed first and
announced after; a failed change announces nothing.

**Settings** are read once in `settings/`. To add one: its default in
`defaults.rs`, a row in `variables.rs` if operators set it as `STELLARR_*`, a
field in the section it belongs to and a line in that section's `load`.
Handlers read `state.settings()`.

**Logging** is `tracing`. A request writes one line under the target
`Stellarr.Api.Requests`; health probes write none.

## The pipeline

Every request goes through `http/pipeline.rs`, in this order: count the
response, log one line, turn a panic into a 500, read the `X-Forwarded-*`
headers (the client's address is `Extension<ClientInfo>`), in development
answer the web dev server's origin, count against the rate limit of the route,
the client gate, refuse a JSON route another content type, authenticate, the
handler. Authentication is `route_checks` in `pipeline.rs`: it reads the session
cookie of every request and checks it against the database (see Sessions).

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
its cookie, but not for a connection that is already open: the hub keeps those.
The hub's side has to close the connections of a session when it is told
`SessionEnded` (a sign-out here), and must also look at the database itself, as
the first server's watcher did every second, to close the connections of
sessions that were revoked, expired, reached their absolute end, belong to a
disabled account or no longer meet the second-step policy, whoever changed
them, another process included. `auth::session::authenticate` has the checks to
reuse.

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

## Plugging in the hub

`serve(settings, groups, listening, shutdown)` runs the API with nobody connected.
`serve_with(.., Services { events, presence }, ..)` is the same with the hub's
`EventPublisher` and `Presence`; the hub publishes nothing back, it reads what
handlers publish (and `SessionEnded`, which says that a session was revoked so
its connections close, and is not passed on to clients).

## Tests

Unit tests sit next to the code in `#[cfg(test)] mod tests`.

Endpoint tests are in `crates/server/tests/api/`, one file per group, all
linked into one test binary (`modules.list.rs` lists them, one `mod` line to a
file, merged the same way). Each test starts the real
`stellarr-api` binary on a free port with a temporary data folder, waits for
`/health/ready`, and stops it afterwards. Tests run in parallel and share
nothing.

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

- `Server::start_with(&[("Desktop__MinimumVersion", "0.1.3")])` adds variables
  to the environment; the names are the ones operators use.
- `server.anonymous()` sends no client headers; `server.client_of("web", "2.0.0")`
  sends the ones you name.
- `server.found("owner").await` founds the instance (the setup code is read from
  the binary's log) and returns a `Person` who is signed in; `server.register("maya")`
  makes an invite and registers with it; `server.sign_in(&person)` is a second
  sign-in with a client of its own; `server.login(&client, name, password)` is the
  bare request. Every person has the password `support::PASSWORD`. `server.database()`
  is a connection of the test's own to the server's file, to arrange or to look.
- `server.database_path()` is the database file. A test may open it with
  `rusqlite` to arrange rows or to look at what the server wrote.
- `server.events()` is every event the server published, as JSON.
- `server.log()` is the lines the binary has logged.
- `Server::in_process(routes, &[...])` runs the library in the test's own process
  with extra routes of the test's own, for what the real groups cannot do on
  request: a route that fails, that is rate limited, that reads a body. See
  `tests/api/pipeline.rs`.
- `client.send(Request::new(Method::POST, "/path").json(&json!({...})))` for
  anything beyond `get` and `post_json`.

`tests/api/oracle.rs` holds an ignored test that sends the same requests to this
server and to the first one and prints where the answers differ; its header says
how to run it. `tests/oracle-shared-database.sh` runs both servers on copies of one
database file: an account made by one signs in on the other, and the answers to the
second step are the same.

The Testing environment lifts the rate limits (ten thousand a window) and lets
the session cookie work over plain HTTP. To test a limit, start in Production
with the limit set, as `pipeline.rs` does.

## Known differences from the first server

- Routes are matched case-sensitively; the first server matched them without
  regard to case. The client gate still treats its paths case-insensitively.
- A request body's property names must have the case the client sends; the
  first server matched them without regard to case.
- Responses with a known length carry `Content-Length` where the first server
  sent them chunked, and a response sent before the request body was read may
  carry `Connection: close`.
- Settings are read once at start; the first server read a few again on each
  request.
- A route that reads JSON reads 1 MiB unless it asks for more (`.json_body_up_to`);
  the first server read up to 128 MiB everywhere, which let anybody make it hold
  that much on a route that needs no session. The 128 MiB ceiling of the whole
  server stays.
- A malformed JSON body is a 400 in every environment; in development the first
  server answered 500.
