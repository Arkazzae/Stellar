//! Where a person has read up to in each text channel.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::params;
use serde::Serialize;

use super::{Answer, Refuse, path_id};
use crate::auth::Caller;
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::http::{ApiResult, Path, json_response};
use crate::permissions::membership::ActiveServer;
use crate::permissions::visibility::{self, ChannelKind};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// Both the answer and the event.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChannelRead {
    channel_id: Id,
    last_read_at: Timestamp,
}

/// Moves the marker forward to now, and leaves it where it is when it is
/// already later: a slow request must not win against a newer one, and the
/// database decides which is newer. The answer is the time that was offered,
/// which is not necessarily the stored one.
pub async fn mark_read(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(channel_id): Path<String>,
) -> ApiResult<Response> {
    let channel_id = path_id(&channel_id)?;
    let messages = *state.messages();
    let user = caller.user;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Answer<ChannelRead>, DbError> {
            // A channel the caller does not view does not exist for them.
            if visibility::find(tx, user, server, channel_id, Some(ChannelKind::Text))?.is_none() {
                return Ok(Answer::Refused(Refuse::NotFound));
            }
            let read_at = Timestamp::now();
            tx.execute(
                "INSERT INTO \"ChannelReads\" (\"ChannelId\", \"UserId\", \"LastReadAt\")
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (\"ChannelId\", \"UserId\") DO UPDATE
                 SET \"LastReadAt\" = MAX(\"ChannelReads\".\"LastReadAt\", excluded.\"LastReadAt\")",
                params![channel_id, user, read_at],
            )?;
            Ok(Answer::Done(ChannelRead {
                channel_id,
                last_read_at: read_at,
            }))
        })
        .await?;
    match outcome {
        Answer::Refused(refusal) => Err(refusal.into_error(&messages)?),
        Answer::Done(read) => {
            state.events().publish(Event::new(
                Audience::Users(vec![user]),
                "ChannelReadChanged",
                &read,
            ));
            Ok(json_response(StatusCode::OK, &read))
        }
    }
}
