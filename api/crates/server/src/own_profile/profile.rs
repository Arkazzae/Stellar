//! `PATCH /api/me`: the name, status, bio and pronouns of the own account.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use stellarr_messages::MessageKey;

use crate::accounts::me::Account;
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Problem, json_response};
use crate::instance::users::User;
use crate::servers::Object;
use crate::state::AppState;

const DISPLAY_NAME_MAX: usize = 64;
const CUSTOM_STATUS_MAX: usize = 128;
const BIO_MAX: usize = 2000;
const PRONOUNS_MAX: usize = 32;

/// A property left out, or null, stays as it is.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAccount {
    display_name: Option<String>,
    email: Option<String>,
    custom_status: Option<String>,
    bio: Option<String>,
    pronouns: Option<String>,
}

fn units(text: &str) -> usize {
    text.trim().encode_utf16().count()
}

/// Clearing a field stores nothing rather than an empty string.
fn blank_to_none(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn refusals(state: &AppState, request: &UpdateAccount) -> Result<Vec<(String, Vec<String>)>, ApiError> {
    let messages = state.messages();
    let mut errors = Vec::new();
    let mut refuse = |field: &str, key: MessageKey, max: usize| -> Result<(), ApiError> {
        errors.push((field.to_string(), vec![messages.get(key, &[("max", &max)])?]));
        Ok(())
    };
    if request.display_name.as_deref().is_some_and(|name| !(1..=DISPLAY_NAME_MAX).contains(&units(name))) {
        refuse("displayName", MessageKey::AccountDisplayNameLength, DISPLAY_NAME_MAX)?;
    }
    if request.bio.as_deref().is_some_and(|bio| units(bio) > BIO_MAX) {
        refuse("bio", MessageKey::BioTooLong, BIO_MAX)?;
    }
    if request.custom_status.as_deref().is_some_and(|status| units(status) > CUSTOM_STATUS_MAX) {
        refuse("customStatus", MessageKey::CustomStatusTooLong, CUSTOM_STATUS_MAX)?;
    }
    if request.pronouns.as_deref().is_some_and(|pronouns| units(pronouns) > PRONOUNS_MAX) {
        refuse("pronouns", MessageKey::PronounsTooLong, PRONOUNS_MAX)?;
    }
    Ok(errors)
}

pub async fn update(
    State(state): State<AppState>,
    caller: Caller,
    Json(Object(request)): Json<Object<UpdateAccount>>,
) -> ApiResult<Response> {
    if request.email.is_some() {
        let text = state.messages().get(MessageKey::RecoveryEmailChangeDisabled, &[])?;
        return Err(Problem::validation(vec![("email".to_string(), vec![text])]).into());
    }
    let errors = refusals(&state, &request)?;
    if !errors.is_empty() {
        return Err(Problem::validation(errors).into());
    }
    let user = caller.user;
    let account = state
        .database()
        .write(move |tx| -> Result<Option<Account>, DbError> {
            let Some(before) = User::find(tx, user)? else {
                return Ok(None);
            };
            // Only what changes is written, so that what watches a column
            // (the search text of messages watches the name) is not woken by
            // a save that changes nothing.
            let mut sets: Vec<(&str, Option<String>)> = Vec::new();
            if let Some(name) = request.display_name.as_deref() {
                let name = name.trim();
                if name != before.display_name {
                    sets.push(("DisplayName", Some(name.to_string())));
                }
            }
            for (column, was, sent) in [
                ("CustomStatus", &before.custom_status, &request.custom_status),
                ("Bio", &before.bio, &request.bio),
                ("Pronouns", &before.pronouns, &request.pronouns),
            ] {
                if let Some(text) = sent {
                    let now = blank_to_none(text);
                    if &now != was {
                        sets.push((column, now));
                    }
                }
            }
            if !sets.is_empty() {
                let assignments: Vec<String> = sets
                    .iter()
                    .enumerate()
                    .map(|(index, (column, _))| format!("\"{column}\" = ?{}", index + 1))
                    .collect();
                let mut values: Vec<&dyn rusqlite::ToSql> = Vec::new();
                for (_, value) in &sets {
                    values.push(value);
                }
                values.push(&user);
                tx.execute(
                    &format!("UPDATE \"Users\" SET {} WHERE \"Id\" = ?{}", assignments.join(", "), sets.len() + 1),
                    params_from(&values),
                )?;
            }
            Ok(Account::load(tx, user)?)
        })
        .await?;
    let account = account.ok_or_else(|| ApiError::internal("the signed-in account is not there"))?;
    crate::uploads::broadcast_user_updated(&state, user).await?;
    Ok(json_response(StatusCode::OK, &account))
}

fn params_from<'a>(values: &'a [&'a dyn rusqlite::ToSql]) -> impl rusqlite::Params + 'a {
    rusqlite::params_from_iter(values.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_are_counted_after_trimming_in_utf16_units() {
        assert_eq!(units("  ab  "), 2);
        assert_eq!(units("\u{1f680}"), 2);
        assert_eq!(units("\u{a0}\u{2003}x\u{3000}"), 1);
    }

    #[test]
    fn a_blank_text_is_nothing() {
        assert_eq!(blank_to_none("  \t"), None);
        assert_eq!(blank_to_none(" x "), Some("x".to_string()));
    }
}
