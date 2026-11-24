//! The refusals clients act on by their `code`, each with a title they may
//! show and whatever else they need to recover. The extras come first and the
//! code last, as the first server wrote them.

use axum::http::StatusCode;
use serde::Serialize;
use stellarr_messages::MessageKey;

use crate::http::{ApiError, Problem};
use crate::state::AppState;

pub const ACCOUNT_KEY_MISSING: &str = "account-key-missing";
pub const ACCOUNT_KEY_EXISTS: &str = "account-key-exists";
pub const ACCOUNT_KEY_CHANGED: &str = "account-key-changed";
pub const BACKUP_MISSING: &str = "backup-missing";
pub const BACKUP_DELETED: &str = "backup-deleted";
pub const PIN_INCORRECT: &str = "pin-incorrect";
pub const PASSWORD_INCORRECT: &str = "password-incorrect";
pub const DEVICE_MISSING: &str = "device-missing";
pub const DEVICE_LIMIT: &str = "device-limit";
pub const CURRENT_DEVICE: &str = "current-device";
pub const STATE_CONFLICT: &str = "encryption-state";
pub const NOT_ALLOWED: &str = "encryption-not-allowed";
pub const REQUIRED: &str = "encryption-required";
pub const OFF: &str = "encryption-off";
pub const EPOCH_NOT_CURRENT: &str = "epoch-not-current";
pub const EPOCH_OUTDATED: &str = "epoch-outdated";
pub const ATTACHMENT_KIND: &str = "attachment-kind-mismatch";
pub const EPOCH_STALE: &str = "epoch-stale";
pub const EPOCH_COVERAGE: &str = "epoch-coverage";
pub const EPOCH_EXISTS: &str = "epoch-exists";

/// A refusal being put together.
pub struct Refusal(Result<Problem, ApiError>);

impl Refusal {
    pub fn new(state: &AppState, status: StatusCode, title: MessageKey) -> Refusal {
        Refusal(
            state
                .messages()
                .get(title, &[])
                .map(|title| Problem::new(status).title(title))
                .map_err(ApiError::from),
        )
    }

    pub fn with(self, name: &str, value: impl Into<serde_json::Value>) -> Refusal {
        Refusal(self.0.map(|problem| problem.with(name, value)))
    }

    pub fn with_json(self, name: &str, value: &impl Serialize) -> Refusal {
        Refusal(self.0.map(|problem| problem.with_json(name, value)))
    }

    pub fn code(self, code: &str) -> ApiError {
        match self.0 {
            Ok(problem) => problem.with("code", code).into(),
            Err(error) => error,
        }
    }
}

/// A refusal with a code and nothing else.
pub fn coded(state: &AppState, status: StatusCode, code: &str, title: MessageKey) -> ApiError {
    Refusal::new(state, status, title).code(code)
}

/// A refusal of what a field holds, as for any other request.
pub fn invalid(state: &AppState, field: &str, title: MessageKey, values: &[(&str, stellarr_messages::Value<'_>)]) -> ApiError {
    match state.messages().get(title, values) {
        Ok(text) => Problem::validation(vec![(field.to_string(), vec![text])]).into(),
        Err(missing) => missing.into(),
    }
}

pub fn account_key_missing(state: &AppState) -> ApiError {
    coded(state, StatusCode::CONFLICT, ACCOUNT_KEY_MISSING, MessageKey::EncryptionAccountKeyMissing)
}

pub fn backup_missing(state: &AppState) -> ApiError {
    coded(state, StatusCode::NOT_FOUND, BACKUP_MISSING, MessageKey::EncryptionBackupMissing)
}

pub fn password_incorrect(state: &AppState) -> ApiError {
    coded(state, StatusCode::FORBIDDEN, PASSWORD_INCORRECT, MessageKey::EncryptionPasswordIncorrect)
}

/// The refusal of a change for want of a fresh proof, as the route's guard
/// answers it.
pub fn fresh_proof_needed(state: &AppState) -> ApiError {
    match crate::auth::fresh::refusal(state.messages()) {
        Ok(problem) => problem.into(),
        Err(missing) => missing.into(),
    }
}
