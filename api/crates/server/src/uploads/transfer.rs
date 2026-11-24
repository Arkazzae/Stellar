//! Staging and storing files under the upload admission limits.

use axum::extract::Request;

use super::attachments::roll_back;
use super::files::read;
use super::form::{Received, Slot, Staged};
use super::inspect::keep_image;
use super::refusals::no_room;
use crate::admission::Lease;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use crate::values::Id;

/// Reads a form with one image slot. The caller decides when to require
/// that slot and how to validate the other fields.
pub(crate) async fn read_image_form(
    state: &AppState,
    request: Request,
    max_bytes: i64,
    fields: &[&str],
) -> ApiResult<Received> {
    let keeps = |content_type: &str| keep_image(content_type, max_bytes);
    let slots = [Slot { name: "file", keeps: &keeps }];
    read(state, request, &slots, fields).await
}

pub(crate) async fn reserve(state: &AppState, user: Id, bytes: i64) -> ApiResult<Lease> {
    match state.admission().enter(state.database(), user, bytes).await? {
        Ok(lease) => Ok(lease),
        Err(why) => Err(no_room(state, why)?),
    }
}

pub(crate) async fn store_file(state: &AppState, key: &str, staged: &Staged) -> ApiResult<()> {
    let mut source = staged.open().await.map_err(ApiError::internal)?;
    state.storage().put(key, &mut source).await.map_err(ApiError::internal)
}

/// Takes the prepared objects back out of storage if a write refused them.
pub(crate) async fn stored_or_rollback<T>(
    state: &AppState,
    keys: Vec<String>,
    result: ApiResult<T>,
) -> ApiResult<T> {
    if result.is_err() {
        roll_back(state, keys).await;
    }
    result
}
