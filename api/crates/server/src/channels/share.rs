//! Only offer destinations that admit a message with files for this member.
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiResult, json_response, no_store};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::{ATTACH_FILES, SEND_MESSAGES, VIEW_CHANNEL, contains};
use crate::permissions::snapshot::{Snapshot, Targets};
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let ids = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            let channels = super::channels_of(db, server)?;
            let targets = Targets {
                sections: channels.iter().filter_map(|c| c.section_id).collect(),
                channels: channels.iter().map(|c| c.id).collect(),
            };
            let snapshot = Snapshot::load(db, server, Some(&[caller.user]), targets)?;
            let mut allowed = Vec::new();
            if let Some(subject) = snapshot.subject(caller.user) {
                let required = VIEW_CHANNEL | SEND_MESSAGES | ATTACH_FILES;
                for channel in channels.into_iter().filter(|c| c.is_text()) {
                    let held = snapshot.evaluate(subject, channel.section_id, Some(channel.id))?;
                    if contains(held, required) {
                        allowed.push(channel.id);
                    }
                }
            }
            Ok(allowed)
        })
        .await?;
    Ok(no_store(json_response(StatusCode::OK, &ids)))
}
