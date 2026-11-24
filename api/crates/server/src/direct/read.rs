//! Marking a conversation read.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use super::{peer_in, set_read};
use crate::auth::Caller;
use crate::http::{ApiError, ApiResult, Path};
use crate::social::announce;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

pub async fn mark_read(
    State(state): State<AppState>,
    caller: Caller,
    Path(conversation): Path<Id>,
) -> ApiResult<Response> {
    let me = caller.user;
    state
        .database()
        .write(move |tx| -> Result<(), ApiError> {
            if peer_in(tx, conversation, me)?.is_none() {
                return Err(ApiError::not_found());
            }
            set_read(tx, conversation, me, Timestamp::now())?;
            Ok(())
        })
        .await?;
    announce(&state, &[me]);
    Ok(StatusCode::NO_CONTENT.into_response())
}
