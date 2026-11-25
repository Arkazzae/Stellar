//! The conversations of a channel. A thread has no row of its own: it exists
//! once a message points at a root, and replies are flat, each one answering
//! the root directly.

use axum::extract::State;
use serde::Serialize;

use crate::auth::Caller;
use crate::database::DbError;
use crate::messages::dto::{self, MessageDto, MessageReferenceDto};
use crate::http::{ApiError, ApiResult, Json, Path};
use crate::permissions::membership::ActiveServer;
use crate::permissions::visibility::{self, ChannelKind};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    channel_id: Id,
    root: MessageReferenceDto,
    /// The messages that answer the root directly.
    reply_count: i64,
    /// When the newest of them was sent, which orders the list.
    last_reply_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMessages {
    messages: Vec<MessageDto>,
}

/// Every thread of the channel, the most recently answered first.
pub async fn list(
    Path(channel): Path<Id>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<ThreadSummary>>> {
    let messages = *state.messages();
    let user = caller.user;
    let found = state
        .database()
        .read(move |db| -> Result<Option<Vec<ThreadSummary>>, DbError> {
            // A channel the caller does not view is one that does not exist.
            if visibility::find(db, user, server, channel, Some(ChannelKind::Text))?.is_none() {
                return Ok(None);
            }
            let mut statement = db.prepare(
                "SELECT \"ReplyToMessageId\", COUNT(*), MAX(\"CreatedAt\") FROM \"Messages\"
                 WHERE \"ChannelId\" = ?1 AND \"ReplyToMessageId\" IS NOT NULL
                 GROUP BY \"ReplyToMessageId\"
                 ORDER BY MAX(\"CreatedAt\") DESC, \"ReplyToMessageId\" DESC",
            )?;
            let threads = statement
                .query_map([channel], |row| {
                    Ok((
                        row.get::<_, Id>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Timestamp>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let roots: Vec<Id> = threads.iter().map(|(root, _, _)| *root).collect();
            let mut quotes: std::collections::HashMap<Id, MessageReferenceDto> = dto::references(db, &messages, &roots)?
                .into_iter()
                .map(|quote| (quote.id, quote))
                .collect();
            let mut summaries = Vec::with_capacity(threads.len());
            for (root, reply_count, last_reply_at) in threads {
                if let Some(root) = quotes.remove(&root) {
                    summaries.push(ThreadSummary {
                        channel_id: channel,
                        root,
                        reply_count,
                        last_reply_at,
                    });
                }
            }
            Ok(Some(summaries))
        })
        .await?;
    found.map(Json).ok_or_else(ApiError::not_found)
}

/// A thread in full, read on its own so that opening one does not depend on
/// how far back the transcript has been loaded.
pub async fn read(
    Path((channel, root)): Path<(Id, Id)>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    State(state): State<AppState>,
) -> ApiResult<Json<ThreadMessages>> {
    let messages = *state.messages();
    let user = caller.user;
    let found = state
        .database()
        .read(move |db| -> Result<Option<Vec<MessageDto>>, DbError> {
            if visibility::find(db, user, server, channel, Some(ChannelKind::Text))?.is_none() {
                return Ok(None);
            }
            let mut statement = db.prepare(
                "SELECT \"Id\" FROM \"Messages\"
                 WHERE \"ChannelId\" = ?1 AND (\"Id\" = ?2 OR \"ReplyToMessageId\" = ?2)
                 ORDER BY \"CreatedAt\", \"Id\"",
            )?;
            let ids = statement
                .query_map(rusqlite::params![channel, root], |row| row.get::<_, Id>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if !ids.contains(&root) {
                return Ok(None);
            }
            Ok(Some(dto::load(db, &messages, &ids, Some(user))?))
        })
        .await?;
    let messages = found.ok_or_else(ApiError::not_found)?;
    crate::link_previews::enqueue_imported(&state, channel, &messages).await;
    Ok(Json(ThreadMessages { messages }))
}
