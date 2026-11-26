//! Authentication for the development activity bridge.
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;

use crate::auth::Caller;
use crate::http::{ApiError, ApiResult, Json, Path};
use crate::instance::users::User;
use crate::permissions::membership::InstanceOwner;
use crate::state::AppState;
use crate::values::Id;

#[derive(Serialize)]
pub(super) struct DevParticipant {
    id: Id,
    name: String,
    username: String,
    avatar: Option<String>,
}

pub(super) async fn authorize_author(
    State(state): State<AppState>,
    _owner: InstanceOwner,
) -> ApiResult<StatusCode> {
    if !state.settings().environment.is_development() {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn participant(
    State(state): State<AppState>,
    caller: Caller,
    Path(channel): Path<Id>,
) -> ApiResult<Json<DevParticipant>> {
    if !state.settings().environment.is_development() {
        return Err(ApiError::not_found());
    }
    let hub = state.realtime().ok_or_else(ApiError::forbidden)?;
    if hub
        .presence()
        .voice_connections_of(caller.user, channel)
        .is_empty()
    {
        return Err(ApiError::forbidden());
    }
    let user = state
        .database()
        .read(move |db| User::find(db, caller.user))
        .await?
        .ok_or_else(ApiError::forbidden)?;
    Ok(Json(DevParticipant {
        id: caller.user,
        name: user.display_name,
        username: user.username,
        avatar: user.avatar_url.filter(|url| url.starts_with("/assets/")),
    }))
}
