//! Reads a channel's transcript back, page by page, and what changed in it
//! since.

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use rusqlite::{Connection, params};
use serde::Serialize;
use stellarr_messages::{MessageKey, Messages};

use super::cursor::HistoryCursor;
use super::dto::{self, MessageDto};
use super::field_problem;
use crate::auth::Caller;
use crate::http::{ApiError, ApiResult, Json, Path, QueryValues};
use crate::permissions::membership::ActiveServer;
use crate::permissions::visibility::{self, ChannelKind};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: i32 = 100;
const DEFAULT_SYNC_PAGE_SIZE: usize = 200;
const MAX_SYNC_PAGE_SIZE: i32 = 500;

/// A change of a message in the journal.
const UPSERT: i64 = 0;
const RESET: i64 = 2;

/// What a history request's query string says (see `http::QueryValues`).
#[derive(Debug)]
pub struct HistoryQuery {
    cursor: Option<String>,
    limit: Option<i32>,
}

impl FromRequestParts<AppState> for HistoryQuery {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<HistoryQuery, ApiError> {
        let values = QueryValues::parse(parts.uri.query(), state.messages());
        Ok(HistoryQuery {
            limit: values.int("limit")?,
            cursor: values.text("cursor")?,
        })
    }
}

#[derive(Debug)]
pub struct SyncQuery {
    after: Option<i64>,
    limit: Option<i32>,
}

impl FromRequestParts<AppState> for SyncQuery {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<SyncQuery, ApiError> {
        let values = QueryValues::parse(parts.uri.query(), state.messages());
        Ok(SyncQuery {
            after: values.long("after")?,
            limit: values.int("limit")?,
        })
    }
}

/// One page of a channel's history, oldest message first, so that a client can
/// prepend the whole array without reversing it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub messages: Vec<MessageDto>,
    /// Opaque; null once the beginning of the channel has been reached.
    pub next_cursor: Option<String>,
    /// The journal position the page was read at, where a client resumes
    /// listening for changes.
    pub sync_cursor: String,
}

/// What happened to one message: `upsert` with the message as it is now, or
/// `delete` with none.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncChange {
    pub cursor: String,
    pub kind: &'static str,
    pub message_id: Id,
    pub message: Option<MessageDto>,
}

/// Changes of one channel after a journal position, oldest first.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPage {
    pub changes: Vec<SyncChange>,
    /// Where to ask next; the position asked from when nothing changed.
    pub next_cursor: String,
    pub has_more: bool,
    /// Set when the journal no longer reaches back far enough, and the client
    /// has to load the history again instead of replaying changes.
    pub reset_required: bool,
}

pub async fn history(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(channel): Path<Id>,
    query: HistoryQuery,
) -> ApiResult<Json<HistoryPage>> {
    let messages = *state.messages();
    if query
        .limit
        .is_some_and(|limit| !(1..=MAX_PAGE_SIZE).contains(&limit))
    {
        return Err(field_problem(
            "limit",
            &messages,
            MessageKey::PageLimitRange,
            &[("max", &MAX_PAGE_SIZE)],
        ));
    }
    let page = state
        .database()
        .read(move |db| read_history(db, &messages, caller.user, server, channel, query))
        .await?;
    Ok(Json(page))
}

fn read_history(
    db: &Connection,
    messages: &Messages,
    user: Id,
    server: Id,
    channel: Id,
    query: HistoryQuery,
) -> Result<HistoryPage, ApiError> {
    // A channel the caller does not view is answered as one that does not
    // exist.
    if visibility::find(db, user, server, channel, Some(ChannelKind::Text))?.is_none() {
        return Err(ApiError::not_found());
    }

    // New arrivals stay outside the snapshot; a stable message key keeps
    // deletions from shifting subsequent pages.
    let page = match query.cursor {
        None => HistoryCursor {
            scope: channel,
            snapshot: Timestamp::now(),
            before: None,
        },
        Some(text) => match HistoryCursor::decode(&text) {
            Some(cursor) if cursor.scope == channel => cursor,
            _ => {
                return Err(field_problem(
                    "cursor",
                    messages,
                    MessageKey::HistoryCursorInvalid,
                    &[],
                ));
            }
        },
    };

    let page_size = query
        .limit
        .map_or(DEFAULT_PAGE_SIZE, |limit| limit as usize);
    // One snapshot for both reads, so the journal position matches the rows.
    let sync_sequence: Option<i64> = db.query_row(
        "SELECT MAX(\"Sequence\") FROM \"MessageChanges\" WHERE \"ChannelId\" = ?1",
        [channel],
        |row| row.get(0),
    )?;
    let fetched = i64::try_from(page_size + 1).unwrap_or(i64::MAX);
    let mut rows: Vec<(Id, Timestamp)> = Vec::new();
    {
        let mut read = |statement: &mut rusqlite::Statement<'_>,
                        parameters: &[&dyn rusqlite::ToSql]| {
            let found = statement.query_map(parameters, |row| Ok((row.get(0)?, row.get(1)?)))?;
            for row in found {
                rows.push(row?);
            }
            Ok::<(), rusqlite::Error>(())
        };
        match page.before {
            Some((before_at, before_id)) => {
                let mut statement = db.prepare_cached(
                    "SELECT \"Id\", \"CreatedAt\" FROM \"Messages\"
                     WHERE \"ChannelId\" = ?1 AND \"CreatedAt\" <= ?2
                       AND (\"CreatedAt\" < ?3 OR (\"CreatedAt\" = ?3 AND \"Id\" < ?4))
                     ORDER BY \"CreatedAt\" DESC, \"Id\" DESC LIMIT ?5",
                )?;
                read(
                    &mut statement,
                    params![channel, page.snapshot, before_at, before_id, fetched],
                )?;
            }
            None => {
                let mut statement = db.prepare_cached(
                    "SELECT \"Id\", \"CreatedAt\" FROM \"Messages\"
                     WHERE \"ChannelId\" = ?1 AND \"CreatedAt\" <= ?2
                     ORDER BY \"CreatedAt\" DESC, \"Id\" DESC LIMIT ?3",
                )?;
                read(&mut statement, params![channel, page.snapshot, fetched])?;
            }
        }
    }

    let has_more = rows.len() > page_size;
    let next_cursor = if has_more {
        let (last_id, last_at) = rows[page_size - 1];
        HistoryCursor {
            before: Some((last_at, last_id)),
            ..page
        }
        .encode()
    } else {
        None
    };
    let mut ids: Vec<Id> = rows.iter().take(page_size).map(|(id, _)| *id).collect();
    ids.reverse();
    Ok(HistoryPage {
        messages: dto::load(db, messages, &ids, Some(user))?,
        next_cursor,
        sync_cursor: sync_sequence.unwrap_or(0).to_string(),
    })
}

pub async fn sync(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(channel): Path<Id>,
    query: SyncQuery,
) -> ApiResult<Json<SyncPage>> {
    let messages = *state.messages();
    if query.after.is_some_and(|after| after < 0)
        || query
            .limit
            .is_some_and(|limit| !(1..=MAX_SYNC_PAGE_SIZE).contains(&limit))
    {
        return Err(field_problem(
            "cursor",
            &messages,
            MessageKey::SyncCursorInvalid,
            &[],
        ));
    }
    let page = state
        .database()
        .read(move |db| read_sync(db, &messages, caller.user, server, channel, query))
        .await?;
    Ok(Json(page))
}

fn read_sync(
    db: &Connection,
    messages: &Messages,
    user: Id,
    server: Id,
    channel: Id,
    query: SyncQuery,
) -> Result<SyncPage, ApiError> {
    if visibility::find(db, user, server, channel, Some(ChannelKind::Text))?.is_none() {
        return Err(ApiError::not_found());
    }

    let from = query.after.unwrap_or(0);
    let page_size = query
        .limit
        .map_or(DEFAULT_SYNC_PAGE_SIZE, |limit| limit as usize);
    let rows: Vec<(i64, Id, i64)> = {
        let mut statement = db.prepare_cached(
            "SELECT \"Sequence\", \"MessageId\", \"Kind\" FROM \"MessageChanges\"
             WHERE \"ChannelId\" = ?1 AND \"Sequence\" > ?2 ORDER BY \"Sequence\" LIMIT ?3",
        )?;
        statement
            .query_map(
                params![
                    channel,
                    from,
                    i64::try_from(page_size + 1).unwrap_or(i64::MAX)
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?
            .collect::<rusqlite::Result<_>>()?
    };
    let has_more = rows.len() > page_size;
    let changes = &rows[..rows.len().min(page_size)];

    let mut touched: Vec<Id> = Vec::new();
    for (_, message, kind) in changes {
        if *kind == UPSERT && !touched.contains(message) {
            touched.push(*message);
        }
    }
    let mut current: std::collections::HashMap<Id, MessageDto> =
        dto::load(db, messages, &touched, Some(user))?
            .into_iter()
            .map(|message| (message.id, message))
            .collect();

    let mut out = Vec::with_capacity(changes.len());
    for (sequence, message_id, kind) in changes {
        // A message that was upserted and removed since has nothing left to
        // show. One that was upserted twice is shown at both, as it is now.
        let message = if *kind == UPSERT {
            current.get(message_id).cloned()
        } else {
            None
        };
        out.push(SyncChange {
            cursor: sequence.to_string(),
            kind: if message.is_some() {
                "upsert"
            } else {
                "delete"
            },
            message_id: *message_id,
            message,
        });
    }
    current.clear();
    let next = changes.last().map_or(from, |(sequence, _, _)| *sequence);
    Ok(SyncPage {
        changes: out,
        next_cursor: next.to_string(),
        has_more,
        reset_required: changes.iter().any(|(_, _, kind)| *kind == RESET),
    })
}
