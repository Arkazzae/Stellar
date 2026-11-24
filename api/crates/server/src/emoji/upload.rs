//! `POST /api/emoji`: a new emoji from a form with a `shortcode` and a `file`.
//!
//! Validation reports the first failure in this order: a form that
//! cannot be read, then who may, then the name, then what the file is (empty,
//! of a type nobody serves, too large), then whether the name is taken, then
//! whether the bytes are what the type says, then whether there is room.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::params;
use stellarr_messages::MessageKey;
use stellarr_unicode::to_lower;

use super::{EmojiDto, announce_changed, may_manage};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Problem, created};
use crate::permissions::audit::{self, AuditEvent};
use crate::permissions::membership::ActiveServer;
use crate::state::AppState;
use crate::uploads::transfer::{read_image_form, reserve, store_file, stored_or_rollback};
use crate::uploads::{Failure, compact, inspect_image, month, problem};
use crate::values::{Id, Timestamp};

const MAX_EMOJI_BYTES: i64 = 512 * 1024;
const MIN_CODE: usize = 2;
const MAX_CODE: usize = 32;

/// The code a name is kept as: without the space around it or the colons
/// around that, in lower case. It is two to 32 lower-case letters, digits or
/// underscores, and nothing after them: not even a line break.
fn shortcode(typed: &str) -> Option<String> {
    let code = to_lower(typed.trim().trim_matches(':'));
    let fits = (MIN_CODE..=MAX_CODE).contains(&code.len())
        && code.bytes().all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'));
    fits.then_some(code)
}

fn refusal(state: &AppState, failure: Failure) -> Result<ApiError, ApiError> {
    match failure {
        // An empty file is reported like an oversized one, as it always was.
        Failure::Empty | Failure::TooLarge => problem(
            state,
            StatusCode::PAYLOAD_TOO_LARGE,
            MessageKey::EmojiTooLarge,
            MessageKey::EmojiMaximum,
        ),
        Failure::UnsupportedType => problem(
            state,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            MessageKey::EmojiTypeUnsupported,
            MessageKey::ImageTypesAllowed,
        ),
        Failure::SignatureMismatch => problem(
            state,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            MessageKey::EmojiTypeMismatch,
            MessageKey::FileContentMismatch,
        ),
    }
}

pub async fn upload(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    request: Request,
) -> ApiResult<Response> {
    let mut received = read_image_form(&state, request, MAX_EMOJI_BYTES, &["shortcode"]).await?;
    let file = received.files.remove(0);
    let typed = match received.fields.get("shortcode") {
        Some(Ok(text)) => text.to_string(),
        // Far too long to be a name.
        Some(Err(_)) => "x".repeat(MAX_CODE + 1),
        None => return Err(ApiError::bad_request()),
    };
    let file = file.ok_or_else(ApiError::bad_request)?;
    let user = caller.user;

    let may = state
        .database()
        .read(move |db| -> Result<bool, DbError> { Ok(may_manage(db, user, server)?) })
        .await?;
    if !may {
        return Err(ApiError::forbidden());
    }
    let Some(code) = shortcode(&typed) else {
        let text = state.messages().get(MessageKey::EmojiShortcodeInvalid, &[])?;
        return Err(Problem::validation(vec![("shortcode".to_string(), vec![text])]).into());
    };
    let inspected = inspect_image(&file, MAX_EMOJI_BYTES);
    let spec = match inspected {
        Ok(spec) => Some(spec),
        // The bytes are looked at after the name, as they always were.
        Err(refused) if refused.failure == Failure::SignatureMismatch => None,
        Err(refused) => return Err(refusal(&state, refused.failure)?),
    };

    let taken = {
        let code = code.clone();
        state
            .database()
            .read(move |db| -> Result<bool, DbError> {
                Ok(db.query_row(
                    "SELECT EXISTS (SELECT 1 FROM \"ServerEmoji\" WHERE \"ServerId\" = ?1 AND \"Shortcode\" = ?2)",
                    params![server, code],
                    |row| row.get(0),
                )?)
            })
            .await?
    };
    if taken {
        return Err(ApiError::conflict());
    }
    let Some(spec) = spec else {
        return Err(refusal(&state, Failure::SignatureMismatch)?);
    };

    let lease = reserve(&state, user, file.length as i64).await?;
    let id = Id::random();
    let key = format!("emoji/{}/{}{}", month(), compact(id), spec.extension);
    store_file(&state, &key, &file).await?;

    let sentence = state
        .messages()
        .get(MessageKey::AuditEmojiAdded, &[("name", &code)])?;
    let row = (key.clone(), spec.content_type.to_string(), file.length as i64);
    let stored_code = code.clone();
    let written = state
        .database()
        .try_write(move |tx| -> Result<(), DbError> {
            tx.execute(
                "INSERT INTO \"ServerEmoji\" (\"Id\", \"ServerId\", \"Shortcode\", \"ObjectKey\", \"ContentType\", \"SizeBytes\", \"UploadedById\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, server, stored_code, row.0, row.1, row.2, user, Timestamp::now()],
            )?;
            audit::record(
                tx,
                server,
                user,
                &AuditEvent {
                    action: "emoji.create".to_string(),
                    sentence,
                    target_type: Some("emoji".to_string()),
                    target_id: Some(id),
                    target_name: Some(stored_code),
                    changes: Vec::new(),
                },
            )?;
            Ok(())
        })
        .await;
    drop(lease);
    // Another upload may have taken the name since the check above.
    let written = written.map_err(ApiError::from).and_then(|row| row.ok_or_else(ApiError::conflict));
    stored_or_rollback(&state, vec![key], written).await?;
    announce_changed(&state, server);
    Ok(created(format!("/api/emoji/{id}"), &EmojiDto::new(id, server, &code)))
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;

    #[test]
    fn a_name_loses_its_space_and_colons_and_its_case() {
        for (typed, kept) in [
            ("mushris", Some("mushris")),
            (" :Mushris: ", Some("mushris")),
            ("::ab::", Some("ab")),
            ("a_1", Some("a_1")),
            ("a", None),
            ("!!", None),
            ("a b", None),
            (&"a".repeat(32), Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")),
            (&"a".repeat(33), None),
            // Not even one line break may follow.
            (":ab\n:", None),
            (&format!(":{}\n:", "a".repeat(32)), None),
            (":ab\n\n:", None),
            (":\nab:", None),
            ("\u{212a}k", Some("kk")),
            ("\u{130}k", None),
        ] {
            assert_eq!(shortcode(typed).as_deref(), kept, "{typed:?}");
        }
    }

    #[test]
    fn the_event_says_nothing_but_the_fact() {
        #[derive(Serialize)]
        struct Nothing {}
        assert_eq!(serde_json::to_string(&Nothing {}).unwrap(), "{}");
    }
}
