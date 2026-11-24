//! Listing the inbox, reading entries and dismissing them.

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::entry::{COLUMNS, Entry, announce, announce_all_read};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Path, Problem, json_response};
use crate::permissions::membership::ActiveServer;
use crate::social::GuidSegment;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const DEFAULT_PAGE_SIZE: i64 = 30;
const MAX_PAGE_SIZE: i64 = 100;

/// What the inbox lists and counts: the entries of the active server, and the
/// invitations to other servers for as long as the invite can still be used.
/// Parameters: the person, the server, now.
const SHOWN: &str = "n.\"UserId\" = ?1 AND (n.\"ServerId\" = ?2 OR (n.\"Source\" = 'system'
        AND n.\"TargetInviteCode\" IS NOT NULL
        AND EXISTS (SELECT 1 FROM \"ServerInvites\" i
                    WHERE i.\"ServerId\" = n.\"ServerId\" AND i.\"Code\" = n.\"TargetInviteCode\"
                      AND (i.\"ExpiresAt\" IS NULL OR i.\"ExpiresAt\" > ?3)
                      AND (i.\"MaxUses\" IS NULL OR i.\"Uses\" < i.\"MaxUses\"))))";

/// The entry of this person that is theirs to read or dismiss. An invitation
/// stays dismissable after it can no longer be used. Parameters: the entry,
/// the person, the server.
const OWNED: &str = "\"Id\" = ?1 AND \"UserId\" = ?2
    AND (\"ServerId\" = ?3 OR (\"Source\" = 'system' AND \"TargetInviteCode\" IS NOT NULL))";

/// Keeps a private answer out of every cache.
fn private(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn no_such_entry() -> Response {
    private(ApiError::not_found().into_response())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    #[serde(rename = "type")]
    kind: String,
    package_id: String,
    package_name: String,
    module_id: String,
    module_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    #[serde(rename = "type")]
    kind: &'static str,
    channel_id: Option<String>,
    invite_code: Option<String>,
    server_id: Option<String>,
    server_name: Option<String>,
    invited_by_name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Notification {
    id: String,
    sequence: i64,
    title: String,
    body: String,
    tone: String,
    source: Source,
    target: Option<Target>,
    created_at: String,
    read_at: Option<String>,
}

/// An invitation keeps the server's name and the inviter's in the source
/// columns it was delivered with. `just_read` is a moment that was set by this
/// request and not read back from the database, which the first server wrote
/// with its zone.
fn dto(entry: &Entry, just_read: bool) -> Notification {
    let target = match (&entry.target_invite_code, entry.target_channel) {
        (Some(code), _) => Some(Target {
            kind: "server-invite",
            channel_id: None,
            invite_code: Some(code.clone()),
            server_id: Some(entry.server.to_string()),
            server_name: Some(entry.source_package_name.clone()),
            invited_by_name: Some(entry.source_module_name.clone()),
        }),
        (None, Some(channel)) => Some(Target {
            kind: "channel",
            channel_id: Some(channel.to_string()),
            invite_code: None,
            server_id: None,
            server_name: None,
            invited_by_name: None,
        }),
        (None, None) => None,
    };
    Notification {
        id: entry.id.to_string(),
        sequence: entry.sequence,
        title: entry.title.clone(),
        body: entry.body.clone(),
        tone: entry.tone.clone(),
        source: Source {
            kind: entry.source.clone(),
            package_id: entry.source_package_id.clone(),
            package_name: entry.source_package_name.clone(),
            module_id: entry.source_module_id.clone(),
            module_name: entry.source_module_name.clone(),
        },
        target,
        created_at: entry.created_at.to_unspecified_wire_text(),
        read_at: entry.read_at.map(|at| {
            if just_read {
                at.to_wire_text()
            } else {
                at.to_unspecified_wire_text()
            }
        }),
    }
}

/// `limit` and `beforeSequence` from the query, read as the first server read
/// them: names in any case, an empty value as none, values given twice joined
/// with a comma, a number with the spaces and sign a number may have. One that
/// does not fit is a bad request, before anything else is asked.
pub struct PageQuery {
    limit: Option<i32>,
    before_sequence: Option<i64>,
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |i: usize| bytes.get(i).and_then(|b| char::from(*b).to_digit(16));
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' => match (hex(at + 1), hex(at + 2)) {
                (Some(high), Some(low)) => {
                    out.push(u8::try_from(high * 16 + low).unwrap_or(b'%'));
                    at += 2;
                }
                _ => out.push(b'%'),
            },
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_blank(c: char) -> bool {
    matches!(c, '\t'..='\r' | ' ')
}

/// A whole number the way a number is read from a query: digits with an
/// optional sign and spaces around.
fn number<T: std::str::FromStr>(text: &str) -> Result<Option<T>, ()> {
    if text.is_empty() {
        return Ok(None);
    }
    let trimmed = text.trim_matches(is_blank);
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    trimmed.parse::<T>().map(Some).map_err(|_| ())

}

fn query_value(query: &str, name: &str) -> String {
    query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            decode(key).eq_ignore_ascii_case(name).then(|| decode(value))
        })
        .collect::<Vec<_>>()
        .join(",")
}

impl<S: Send + Sync> FromRequestParts<S> for PageQuery {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<PageQuery, ApiError> {
        let query = parts.uri.query().unwrap_or("");
        Ok(PageQuery {
            limit: number(&query_value(query, "limit")).map_err(|()| ApiError::bad_request())?,
            before_sequence: number(&query_value(query, "beforeSequence"))
                .map_err(|()| ApiError::bad_request())?,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    items: Vec<Notification>,
    next_before_sequence: Option<i64>,
    unread_count: i64,
    last_sequence: i64,
}

fn list_page(
    db: &Connection,
    user: Id,
    server: Id,
    size: i64,
    before: Option<i64>,
) -> rusqlite::Result<Page> {
    let now = Timestamp::now();
    let mut statement = db.prepare(&format!(
        "SELECT {COLUMNS} FROM \"UserNotifications\" AS n WHERE {SHOWN} AND (?4 IS NULL OR n.\"Sequence\" < ?4)
         ORDER BY n.\"Sequence\" DESC LIMIT ?5"
    ))?;
    let mut rows: Vec<Entry> = statement
        .query_map(params![user, server, now, before, size + 1], Entry::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let more = rows.len() as i64 > size;
    rows.truncate(size as usize);
    let (unread_count, last_sequence): (i64, Option<i64>) = db.query_row(
        &format!(
            "SELECT COUNT(CASE WHEN n.\"ReadAt\" IS NULL THEN 1 END), MAX(n.\"Sequence\")
             FROM \"UserNotifications\" AS n WHERE {SHOWN}"
        ),
        params![user, server, now],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(Page {
        next_before_sequence: if more { rows.last().map(|e| e.sequence) } else { None },
        items: rows.iter().map(|entry| dto(entry, false)).collect(),
        unread_count,
        last_sequence: last_sequence.unwrap_or(0),
    })
}

pub async fn list(
    State(state): State<AppState>,
    caller: Caller,
    query: PageQuery,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let size = i64::from(query.limit.map_or(DEFAULT_PAGE_SIZE as i32, |l| l));
    if !(1..=MAX_PAGE_SIZE).contains(&size) || query.before_sequence.is_some_and(|b| b <= 0) {
        return Ok(private(
            Problem::validation(vec![(
                "page".into(),
                vec![format!(
                    "limit must be between 1 and {MAX_PAGE_SIZE} and beforeSequence must be positive."
                )],
            )])
            .into_response(),
        ));
    }
    let before = query.before_sequence;
    let page = state
        .database()
        .read(move |db| -> Result<Page, DbError> {
            Ok(list_page(db, caller.user, server, size, before)?)
        })
        .await?;
    Ok(private(json_response(StatusCode::OK, &page)))
}

fn find_owned(
    db: &Connection,
    id: Id,
    user: Id,
    server: Id,
) -> rusqlite::Result<Option<Entry>> {
    db.query_row(
        &format!("SELECT {COLUMNS} FROM \"UserNotifications\" WHERE {OWNED}"),
        params![id, user, server],
        Entry::from_row,
    )
    .optional()
}

pub async fn mark_read(
    State(state): State<AppState>,
    caller: Caller,
    Path(GuidSegment(id)): Path<GuidSegment>,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let user = caller.user;
    let now = Timestamp::now();
    let found = state
        .database()
        .write(move |tx| -> Result<Option<(Entry, bool)>, DbError> {
            let Some(mut entry) = find_owned(tx, id, user, server)? else {
                return Ok(None);
            };
            if entry.read_at.is_some() {
                return Ok(Some((entry, false)));
            }
            tx.execute(
                "UPDATE \"UserNotifications\" SET \"ReadAt\" = ?1 WHERE \"Sequence\" = ?2",
                params![now, entry.sequence],
            )?;
            entry.read_at = Some(now);
            Ok(Some((entry, true)))
        })
        .await?;
    let Some((entry, changed)) = found else {
        return Ok(no_such_entry());
    };
    if changed {
        announce(state.events(), &entry);
    }
    Ok(private(json_response(StatusCode::OK, &dto(&entry, changed))))
}

#[derive(Serialize)]
struct Changed {
    changed: usize,
}

pub async fn mark_all_read(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let user = caller.user;
    let now = Timestamp::now();
    let changed = state
        .database()
        .write(move |tx| -> Result<usize, DbError> {
            Ok(tx.execute(
                &format!(
                    "UPDATE \"UserNotifications\" SET \"ReadAt\" = ?4 WHERE \"ReadAt\" IS NULL AND \"Sequence\" IN
                       (SELECT n.\"Sequence\" FROM \"UserNotifications\" AS n WHERE {SHOWN})"
                ),
                params![user, server, now, now],
            )?)
        })
        .await?;
    if changed > 0 {
        // Entries of other servers may have been among them.
        announce_all_read(state.events(), user, server);
    }
    Ok(private(json_response(StatusCode::OK, &Changed { changed })))
}

pub async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    Path(GuidSegment(id)): Path<GuidSegment>,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let user = caller.user;
    let found = state
        .database()
        .write(move |tx| -> Result<Option<Entry>, DbError> {
            let Some(entry) = find_owned(tx, id, user, server)? else {
                return Ok(None);
            };
            tx.execute(
                "DELETE FROM \"UserNotifications\" WHERE \"Sequence\" = ?1",
                [entry.sequence],
            )?;
            Ok(Some(entry))
        })
        .await?;
    let Some(entry) = found else {
        return Ok(no_such_entry());
    };
    announce(state.events(), &entry);
    Ok(private(StatusCode::NO_CONTENT.into_response()))
}
