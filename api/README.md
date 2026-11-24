# The Stellarr API

The server behind the web, desktop and Android clients, written as a Cargo
workspace. One crate per folder of `crates/`:

| Crate | What it is |
| --- | --- |
| `messages` (`stellarr-messages`) | Everything the server says to people, in each language. No dependencies. |
| `server` (`stellarr-api`) | The library `stellarr_api` and the binary `stellarr-api`: settings, database, HTTP pipeline and the groups of endpoints. |

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
whole body into memory, up to the 128 MiB ceiling; a route that takes an upload
streams it instead, through `http::read_all`'s limit, once it is written.

**JSON** is decided in `http/json.rs`: camelCase names (`#[serde(rename_all =
"camelCase")]`), `null` for a missing value (never `skip_serializing_if`), `Id`
and `Timestamp` from `values`, an enum written as the clients expect it for
that enum, members in the order the struct declares them. Never answer with
`json!` or a `Value`: its objects sort their keys. A request body is read with
the `Json<T>` extractor; a body that is not the right JSON is a 400 with no
body. Read a body no other way, so that the 128 MiB ceiling holds.

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
- The closure's error type may be your own as long as it has `From<DbError>` and
  `From<rusqlite::Error>`; `DbError::is_unique_violation()` and
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
handler. Authentication has its place at the end of `content_type` in
`pipeline.rs`; it is not written yet.

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
- A malformed JSON body is a 400 in every environment; in development the first
  server answered 500.
