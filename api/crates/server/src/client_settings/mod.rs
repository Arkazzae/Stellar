//! The settings a person's client keeps on the server, so that the theme, the
//! language, the call shortcuts, the voice processing and what the client may
//! share with outside services follow them to every device.
//!
//! `GET /api/me/settings` and `PUT /api/me/settings/{section}`.
//!
//! Each section is replaced whole, so two devices saving different sections
//! never overwrite each other. What is inside a section belongs to the client,
//! which reads it back as it wrote it; the server keeps it as one compact JSON
//! object of at most [`MAX_LENGTH`] characters and does not look inside.

mod json;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use http_body_util::{BodyExt, LengthLimitError, Limited};
use stellarr_messages::MessageKey;

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Endpoints, Path, Problem};
use crate::own_profile::ensure_preferences;
use crate::state::AppState;

/// Far above what any section needs (the voice section is under a kilobyte),
/// and small enough that a row cannot be used as storage.
pub const MAX_LENGTH: usize = 8192;

/// The sections a client may save, by the name in the route, each with the
/// column that keeps it.
const SECTIONS: [(&str, &str); 5] = [
    ("appearance", "AppearanceSettingsJson"),
    ("language", "LanguageSettingsJson"),
    ("keybinds", "KeybindSettingsJson"),
    ("voice", "VoiceSettingsJson"),
    ("privacy", "PrivacySettingsJson"),
];

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/me/settings", read)
        .authorized()
        .put("/api/me/settings/{section}", save)
        .authorized()
}

async fn read(State(state): State<AppState>, caller: Caller) -> ApiResult<Response> {
    let user = caller.user;
    let stored = state
        .database()
        .read(move |db| -> Result<Vec<Option<String>>, DbError> {
            let found = db.query_row(
                "SELECT \"AppearanceSettingsJson\", \"LanguageSettingsJson\", \"KeybindSettingsJson\", \"VoiceSettingsJson\", \"PrivacySettingsJson\"
                 FROM \"UserPreferences\" WHERE \"UserId\" = ?1",
                [user],
                |row| (0..5).map(|index| row.get::<_, Option<String>>(index)).collect(),
            );
            match found {
                Ok(sections) => Ok(sections),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(vec![None; 5]),
                Err(error) => Err(error.into()),
            }
        })
        .await?;
    // A damaged section reads as never saved instead of failing the answer.
    let mut body = String::from("{");
    for (index, ((name, _), text)) in SECTIONS.iter().zip(stored).enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push('"');
        body.push_str(name);
        body.push_str("\":");
        match text.as_deref().and_then(json::object) {
            Some(object) => body.push_str(&object),
            None => body.push_str("null"),
        }
    }
    body.push('}');
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    Ok(response)
}

async fn save(
    State(state): State<AppState>,
    caller: Caller,
    Path(section): Path<String>,
    request: Request,
) -> ApiResult<Response> {
    let Some((_, column)) = SECTIONS.iter().find(|(name, _)| *name == section) else {
        return Err(ApiError::not_found());
    };
    let (parts, body) = request.into_parts();
    let Some(compact) = body_object(&parts.headers, body).await? else {
        let text = state
            .messages()
            .get(MessageKey::ClientSettingsInvalid, &[("max", &MAX_LENGTH)])?;
        return Err(Problem::validation(vec![(section, vec![text])]).into());
    };
    let user = caller.user;
    let statement = format!("UPDATE \"UserPreferences\" SET \"{column}\" = ?1 WHERE \"UserId\" = ?2");
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            // Only this section's column is written, so devices saving other
            // sections at the same moment do not undo each other. The row of a
            // person who never saved anything is made first.
            ensure_preferences(tx, user)?;
            tx.execute(&statement, rusqlite::params![compact, user])?;
            Ok(())
        })
        .await?;
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    Ok(response)
}

/// What the first server read of a body before it parsed it: at most four
/// times the limit in bytes, and at most that many characters.
const MAX_BODY_CHARACTERS: usize = MAX_LENGTH * 4;

/// The body as compact JSON, when it is one object within the limit, and
/// nothing otherwise. It is read by hand, so that a body that is too large is
/// refused before it is parsed and a value that is no object never reaches the
/// row.
async fn body_object(headers: &axum::http::HeaderMap, body: Body) -> Result<Option<String>, ApiError> {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    if declared.is_some_and(|length| length > MAX_BODY_CHARACTERS as u64) {
        return Ok(None);
    }
    // Three bytes make at most one character, so a body past this is past the
    // limit in characters too.
    let bytes = match Limited::new(body, MAX_BODY_CHARACTERS * 3 + 1).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) if error.downcast_ref::<LengthLimitError>().is_some() => return Ok(None),
        Err(_) => return Err(ApiError::bad_request()),
    };
    let text = json::decode(&bytes);
    if text.encode_utf16().count() > MAX_BODY_CHARACTERS {
        return Ok(None);
    }
    Ok(json::object(&text).filter(|compact| compact.len() <= MAX_LENGTH))
}
