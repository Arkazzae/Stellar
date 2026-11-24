//! A conversation's messages, page by page.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, params};
use serde::Serialize;
use stellarr_messages::{MessageKey, Messages};

use super::peer_in;
use crate::auth::Caller;
use crate::http::{ApiError, ApiResult, Path, QueryValues, json_response};
use crate::messages::cursor::{HistoryCursor, Reading};
use crate::messages::field_problem;
use crate::social::conversations::{self, DirectMessage};
use crate::social::is_blocked;
use crate::state::AppState;
use crate::values::{Guid, Id, Timestamp};

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: i32 = 100;

/// One page, oldest message first so that a client can prepend the whole array.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    messages: Vec<DirectMessage>,
    /// Opaque; null once the first message has been reached.
    next_cursor: Option<String>,
}

/// One page of a conversation. A conversation with somebody blocked, in either
/// direction, is as good as absent.
pub async fn history(
    State(state): State<AppState>,
    caller: Caller,
    Path(Guid(conversation)): Path<Guid>,
    values: QueryValues,
) -> ApiResult<Response> {
    let cursor = values.text("cursor");
    let limit = values.int("limit")?;
    let messages = *state.messages();
    if limit.is_some_and(|limit| !(1..=MAX_PAGE_SIZE).contains(&limit)) {
        return Err(field_problem("limit", &messages, MessageKey::PageLimitRange, &[("max", &MAX_PAGE_SIZE)]));
    }
    let me = caller.user;
    let page = state
        .database()
        .read(move |db| read_page(db, &messages, me, conversation, cursor, limit))
        .await?;
    Ok(json_response(StatusCode::OK, &page))
}

fn read_page(
    db: &Connection,
    messages: &Messages,
    me: Id,
    conversation: Id,
    cursor: Option<String>,
    limit: Option<i32>,
) -> Result<Page, ApiError> {
    let Some(peer) = peer_in(db, conversation, me)? else {
        return Err(ApiError::not_found());
    };
    if is_blocked(db, me, peer)? {
        return Err(ApiError::not_found());
    }
    let page = match cursor {
        None => HistoryCursor {
            scope: conversation,
            snapshot: Timestamp::now(),
            before: None,
        },
        Some(text) => match HistoryCursor::decode(&text) {
            Reading::Cursor(cursor) if cursor.scope == conversation => cursor,
            Reading::Fault => return Err(ApiError::internal("a history cursor could not be read")),
            _ => {
                return Err(field_problem("cursor", messages, MessageKey::DirectHistoryCursorInvalid, &[]));
            }
        },
    };
    let size = limit.map_or(DEFAULT_PAGE_SIZE, |limit| limit as usize);
    let fetched = i64::try_from(size + 1).unwrap_or(i64::MAX);
    let rows: Vec<(Id, Timestamp)> = match page.before {
        Some((at, id)) => {
            let mut statement = db.prepare_cached(
                "SELECT \"Id\", \"CreatedAt\" FROM \"DirectMessages\"
                 WHERE \"ConversationId\" = ?1 AND \"CreatedAt\" <= ?2
                   AND (\"CreatedAt\" < ?3 OR (\"CreatedAt\" = ?3 AND \"Id\" < ?4))
                 ORDER BY \"CreatedAt\" DESC, \"Id\" DESC LIMIT ?5",
            )?;
            statement
                .query_map(params![conversation, page.snapshot, at, id, fetched], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        }
        None => {
            let mut statement = db.prepare_cached(
                "SELECT \"Id\", \"CreatedAt\" FROM \"DirectMessages\"
                 WHERE \"ConversationId\" = ?1 AND \"CreatedAt\" <= ?2
                 ORDER BY \"CreatedAt\" DESC, \"Id\" DESC LIMIT ?3",
            )?;
            statement
                .query_map(params![conversation, page.snapshot, fetched], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        }
    };
    let next_cursor = if rows.len() > size {
        let (id, at) = rows[size - 1];
        HistoryCursor { before: Some((at, id)), ..page }.encode()
    } else {
        None
    };
    let mut listed = Vec::with_capacity(size.min(rows.len()));
    for (id, _) in rows.iter().take(size).rev() {
        if let Some((message, _)) = conversations::message(db, *id)? {
            listed.push(message);
        }
    }
    Ok(Page { messages: listed, next_cursor })
}
