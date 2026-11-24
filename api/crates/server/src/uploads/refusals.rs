//! The answers an upload is refused with, and the store's part in a refusal.

use axum::http::StatusCode;
use stellarr_messages::MessageKey;

use super::inspect::{Failure, Refused};
use crate::admission::Refusal;
use crate::http::{ApiError, Problem};
use crate::state::AppState;

pub fn problem(
    state: &AppState,
    status: StatusCode,
    title: MessageKey,
    detail: MessageKey,
) -> Result<ApiError, ApiError> {
    let messages = state.messages();
    Ok(Problem::new(status)
        .title(messages.get(title, &[])?)
        .detail(messages.get(detail, &[])?)
        .into())
}

/// A field of the form that is not acceptable.
pub fn invalid(state: &AppState, field: &str, key: MessageKey) -> Result<ApiError, ApiError> {
    let text = state.messages().get(key, &[])?;
    Ok(Problem::validation(vec![(field.to_string(), vec![text])]).into())
}

/// No room: a quota, or the disk.
pub fn no_room(state: &AppState, why: Refusal) -> Result<ApiError, ApiError> {
    let detail = match why {
        Refusal::UserQuota => MessageKey::StorageUserQuota,
        Refusal::InstanceQuota => MessageKey::StorageInstanceQuota,
        Refusal::LowDiskSpace => MessageKey::StorageLowDisk,
    };
    problem(
        state,
        StatusCode::INSUFFICIENT_STORAGE,
        MessageKey::StorageFull,
        detail,
    )
}

/// The texts one route says its refusals with.
pub struct Texts<'a> {
    pub unsupported_title: MessageKey,
    pub unsupported_detail: MessageKey,
    pub too_large_title: MessageKey,
    /// What the refusal says of a file over its limit; may depend on its kind.
    pub too_large_detail: &'a dyn Fn(Option<stellarr_storage::validation::UploadKind>) -> MessageKey,
    pub mismatch_title: MessageKey,
}

pub fn refuse(state: &AppState, refused: &Refused, texts: &Texts) -> Result<ApiError, ApiError> {
    match refused.failure {
        Failure::Empty => invalid(state, "file", MessageKey::FileEmpty),
        Failure::UnsupportedType => problem(
            state,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            texts.unsupported_title,
            texts.unsupported_detail,
        ),
        Failure::TooLarge => problem(
            state,
            StatusCode::PAYLOAD_TOO_LARGE,
            texts.too_large_title,
            (texts.too_large_detail)(refused.spec.map(|spec| spec.kind)),
        ),
        Failure::SignatureMismatch => problem(
            state,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            texts.mismatch_title,
            MessageKey::FileContentMismatch,
        ),
    }
}
