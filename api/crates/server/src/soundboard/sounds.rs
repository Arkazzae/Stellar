//! Creating, changing and deleting a sound.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use stellarr_messages::MessageKey;

use super::format::range;
use super::{SoundDto, announce_changed, created, no_content, url_of};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Path, Problem, json_response};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization::{Scope, allows};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_SERVER;
use crate::servers::Object;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const MIN_CLIP_SECONDS: f64 = 0.15;
const MAX_CLIP_SECONDS: f64 = 15.0;
/// Trims come from a browser that measured the clip in floating point.
const TRIM_TOLERANCE: f64 = 0.01;
const MAX_NAME_UNITS: usize = 32;
const MAX_GLYPH_UNITS: usize = 16;

fn nil() -> Id {
    Id::from_bytes([0; 16])
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSound {
    #[serde(default = "nil")]
    attachment_id: Id,
    name: Option<String>,
    glyph: Option<String>,
    #[serde(default)]
    trim_start_seconds: f64,
    #[serde(default)]
    trim_end_seconds: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSound {
    name: Option<String>,
    glyph: Option<String>,
    trim_start_seconds: Option<f64>,
    trim_end_seconds: Option<f64>,
}

/// The name and symbol as they are kept, when the sound and its trim are
/// acceptable. Lengths are counted in UTF-16 code units.
fn validate(
    name: Option<&str>,
    glyph: Option<&str>,
    start: f64,
    end: f64,
    duration: f64,
) -> Option<(String, String)> {
    let name = name.map_or("", str::trim);
    let glyph = glyph.map_or("", str::trim);
    let fits = (1..=MAX_NAME_UNITS).contains(&stellarr_unicode::utf16::length(name))
        && (1..=MAX_GLYPH_UNITS).contains(&stellarr_unicode::utf16::length(glyph))
        && start.is_finite()
        && end.is_finite()
        && start >= 0.0
        && end <= duration + TRIM_TOLERANCE
        && end - start >= MIN_CLIP_SECONDS
        && end - start <= MAX_CLIP_SECONDS + TRIM_TOLERANCE;
    fits.then(|| (name.to_string(), glyph.to_string()))
}

/// A trim as it is stored: to the thousandth.
fn thousandths(value: f64) -> f64 {
    (value * 1000.0).round_ties_even() / 1000.0
}

fn invalid_sound(state: &AppState) -> Result<ApiError, ApiError> {
    let text = state.messages().get(
        MessageKey::SoundInvalid,
        &[("min", &"0.15"), ("max", &"15")],
    )?;
    Ok(Problem::validation(vec![("sound".to_string(), vec![text])]).into())
}

fn invalid(state: &AppState, field: &str, key: MessageKey) -> Result<ApiError, ApiError> {
    let text = state.messages().get(key, &[])?;
    Ok(Problem::invalid_field(field, text))
}

/// What the transaction found, for the handler to answer.
enum Outcome<T> {
    Forbidden,
    Done(T),
    Refused(Refusal),
}

enum Refusal {
    Attachment,
    Taken,
    Sound,
    NotFound,
}

pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(Object(request)): Json<Object<CreateSound>>,
) -> ApiResult<Response> {
    let user = caller.user;
    let kept = {
        let state = state.clone();
        move |tx: &rusqlite::Transaction| -> Result<Outcome<SoundDto>, DbError> {
            if !allows(tx, user, &Scope::server(server), MANAGE_SERVER)? {
                return Ok(Outcome::Forbidden);
            }
            // Only the person's own upload, measured, and audio.
            let found: Option<(String, Option<f64>)> = tx
                .query_row(
                    "SELECT \"ContentType\", \"DurationSeconds\" FROM \"ImageAttachments\" WHERE \"Id\" = ?1 AND \"UploadedById\" = ?2",
                    params![request.attachment_id, user],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let measured_audio = found.filter(|(content_type, _)| {
                content_type.len() >= 6 && content_type.as_bytes()[..6].eq_ignore_ascii_case(b"audio/")
            });
            let Some((_, Some(duration))) = measured_audio else {
                return Ok(Outcome::Refused(Refusal::Attachment));
            };
            let taken: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"SoundboardSounds\" WHERE \"AttachmentId\" = ?1)",
                [request.attachment_id],
                |row| row.get(0),
            )?;
            if taken {
                return Ok(Outcome::Refused(Refusal::Taken));
            }
            let Some((name, glyph)) = validate(
                request.name.as_deref(),
                request.glyph.as_deref(),
                request.trim_start_seconds,
                request.trim_end_seconds,
                duration,
            ) else {
                return Ok(Outcome::Refused(Refusal::Sound));
            };
            let last: Option<i64> = tx.query_row(
                "SELECT MAX(\"Position\") FROM \"SoundboardSounds\" WHERE \"ServerId\" = ?1",
                [server],
                |row| row.get(0),
            )?;
            let id = Id::random();
            let start = thousandths(request.trim_start_seconds);
            let end = thousandths(request.trim_end_seconds);
            tx.execute(
                "INSERT INTO \"SoundboardSounds\" (\"Id\", \"ServerId\", \"AttachmentId\", \"CreatedById\", \"Name\", \"Glyph\",
                    \"TrimStartSeconds\", \"TrimEndSeconds\", \"Position\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    server,
                    request.attachment_id,
                    user,
                    name,
                    glyph,
                    start,
                    end,
                    last.map_or(0, |last| last + 1),
                    Timestamp::now(),
                ],
            )?;
            let sentence = state
                .messages()
                .get(MessageKey::AuditSoundAdded, &[("name", &name)])
                .map_err(|error| DbError::Task(error.to_string()))?;
            audit::record(
                tx,
                server,
                user,
                &AuditEvent {
                    action: "sound.create".to_string(),
                    sentence,
                    target_type: Some("sound".to_string()),
                    target_id: Some(id),
                    target_name: Some(name.clone()),
                    changes: Vec::new(),
                },
            )?;
            Ok(Outcome::Done(SoundDto {
                id,
                name,
                glyph,
                url: url_of(request.attachment_id),
                duration_seconds: duration,
                trim_start_seconds: start,
                trim_end_seconds: end,
            }))
        }
    };
    // Two requests may pick the same clip between the check and the insert:
    // the row that lost is a conflict, as the check would have said.
    let outcome = match state.database().write(kept).await {
        Ok(outcome) => outcome,
        Err(error) if error.is_unique_violation() => Outcome::Refused(Refusal::Taken),
        Err(error) => return Err(error.into()),
    };
    match outcome {
        Outcome::Forbidden => Err(ApiError::forbidden()),
        Outcome::Refused(Refusal::Attachment) => {
            Err(invalid(&state, "attachmentId", MessageKey::SoundAttachmentInvalid)?)
        }
        Outcome::Refused(Refusal::Taken) => Err(ApiError::conflict()),
        Outcome::Refused(Refusal::Sound | Refusal::NotFound) => Err(invalid_sound(&state)?),
        Outcome::Done(sound) => {
            announce_changed(&state, server);
            Ok(created(format!("/api/admin/soundboard/sounds/{}", sound.id), &sound))
        }
    }
}

pub async fn update(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(sound_id): Path<Id>,
    Json(Object(request)): Json<Object<UpdateSound>>,
) -> ApiResult<Response> {
    let user = caller.user;
    let work = {
        let state = state.clone();
        move |tx: &rusqlite::Transaction| -> Result<Outcome<SoundDto>, DbError> {
            if !allows(tx, user, &Scope::server(server), MANAGE_SERVER)? {
                return Ok(Outcome::Forbidden);
            }
            let found: Option<(String, String, Id, Option<f64>, f64, f64)> = tx
                .query_row(
                    "SELECT s.\"Name\", s.\"Glyph\", s.\"AttachmentId\", a.\"DurationSeconds\", s.\"TrimStartSeconds\", s.\"TrimEndSeconds\"
                     FROM \"SoundboardSounds\" s JOIN \"ImageAttachments\" a ON a.\"Id\" = s.\"AttachmentId\"
                     WHERE s.\"Id\" = ?1 AND s.\"ServerId\" = ?2",
                    params![sound_id, server],
                    |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
                    },
                )
                .optional()?;
            let Some((was_name, was_glyph, attachment, duration, was_start, was_end)) = found else {
                return Ok(Outcome::Refused(Refusal::NotFound));
            };
            let start = request.trim_start_seconds.unwrap_or(was_start);
            let end = request.trim_end_seconds.unwrap_or(was_end);
            let Some((name, glyph)) = validate(
                Some(request.name.as_deref().unwrap_or(&was_name)),
                Some(request.glyph.as_deref().unwrap_or(&was_glyph)),
                start,
                end,
                duration.unwrap_or(was_end),
            ) else {
                return Ok(Outcome::Refused(Refusal::Sound));
            };
            let start = thousandths(start);
            let end = thousandths(end);
            tx.execute(
                "UPDATE \"SoundboardSounds\" SET \"Name\" = ?1, \"Glyph\" = ?2, \"TrimStartSeconds\" = ?3, \"TrimEndSeconds\" = ?4 WHERE \"Id\" = ?5",
                params![name, glyph, start, end, sound_id],
            )?;
            let trim = (was_start != start || was_end != end).then(|| AuditChange {
                field: "trim".to_string(),
                from: Some(range(was_start, was_end)),
                to: Some(range(start, end)),
                ..AuditChange::default()
            });
            let changes = audit::only([
                AuditChange::changed("name", Some(&was_name), Some(&name)),
                AuditChange::changed("glyph", Some(&was_glyph), Some(&glyph)),
                trim,
            ]);
            if !changes.is_empty() {
                let sentence = state
                    .messages()
                    .get(MessageKey::AuditSoundUpdated, &[("name", &name)])
                    .map_err(|error| DbError::Task(error.to_string()))?;
                audit::record(
                    tx,
                    server,
                    user,
                    &AuditEvent {
                        action: "sound.update".to_string(),
                        sentence,
                        target_type: Some("sound".to_string()),
                        target_id: Some(sound_id),
                        target_name: Some(name.clone()),
                        changes,
                    },
                )?;
            }
            Ok(Outcome::Done(SoundDto {
                id: sound_id,
                name,
                glyph,
                url: url_of(attachment),
                duration_seconds: duration.unwrap_or(end),
                trim_start_seconds: start,
                trim_end_seconds: end,
            }))
        }
    };
    match state.database().write(work).await? {
        Outcome::Forbidden => Err(ApiError::forbidden()),
        Outcome::Refused(Refusal::NotFound) => Err(ApiError::not_found()),
        Outcome::Refused(_) => Err(invalid_sound(&state)?),
        Outcome::Done(sound) => {
            announce_changed(&state, server);
            Ok(json_response(StatusCode::OK, &sound))
        }
    }
}

pub async fn remove(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(sound_id): Path<Id>,
) -> ApiResult<Response> {
    let user = caller.user;
    let work = {
        let state = state.clone();
        move |tx: &rusqlite::Transaction| -> Result<Outcome<Id>, DbError> {
            if !allows(tx, user, &Scope::server(server), MANAGE_SERVER)? {
                return Ok(Outcome::Forbidden);
            }
            let found: Option<(String, Id)> = tx
                .query_row(
                    "SELECT \"Name\", \"AttachmentId\" FROM \"SoundboardSounds\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
                    params![sound_id, server],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            // A sound that is not there is already gone: the answer is the same.
            let Some((name, attachment)) = found else {
                return Ok(Outcome::Refused(Refusal::NotFound));
            };
            tx.execute("DELETE FROM \"SoundboardSounds\" WHERE \"Id\" = ?1", [sound_id])?;
            let sentence = state
                .messages()
                .get(MessageKey::AuditSoundDeleted, &[("name", &name)])
                .map_err(|error| DbError::Task(error.to_string()))?;
            audit::record(
                tx,
                server,
                user,
                &AuditEvent {
                    action: "sound.delete".to_string(),
                    sentence,
                    target_type: Some("sound".to_string()),
                    target_id: Some(sound_id),
                    target_name: Some(name),
                    changes: Vec::new(),
                },
            )?;
            // The clip goes with the sound, so a join sound that names it
            // goes too: left behind, it would be a setting that can only
            // ever mean silence.
            tx.execute(
                "UPDATE \"UserPreferences\" SET \"JoinSoundId\" = NULL WHERE \"JoinSoundId\" = ?1",
                [sound_id],
            )?;
            Ok(Outcome::Done(attachment))
        }
    };
    match state.database().write(work).await? {
        Outcome::Forbidden => Err(ApiError::forbidden()),
        Outcome::Refused(_) => Ok(no_content()),
        Outcome::Done(attachment) => {
            crate::uploads::retire_if_unreferenced(&state, Some(url_of(attachment)), None).await;
            announce_changed(&state, server);
            Ok(no_content())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_and_a_symbol_are_counted_in_utf16_units_after_trimming() {
        let ok = |name: &str, glyph: &str| validate(Some(name), Some(glyph), 0.0, 1.0, 5.0).is_some();
        assert!(ok(&"a".repeat(32), "x"));
        assert!(!ok(&"a".repeat(33), "x"));
        assert!(ok(&format!("  {}  ", "a".repeat(32)), "x"));
        assert!(!ok("   ", "x"));
        assert!(ok("n", &"\u{1f680}".repeat(8)));
        assert!(!ok("n", &"\u{1f680}".repeat(9)));
        assert!(!ok("n", ""));
    }

    #[test]
    fn a_trim_is_inside_the_clip_and_between_the_shortest_and_the_longest() {
        let ok = |start: f64, end: f64| validate(Some("n"), Some("g"), start, end, 30.0).is_some();
        assert!(ok(0.0, 0.15));
        assert!(!ok(0.0, 0.149));
        assert!(ok(0.0, 15.01));
        assert!(!ok(0.0, 15.011));
        assert!(ok(20.0, 30.01));
        assert!(!ok(20.0, 30.011));
        // Sums in floating point, so the edge is where the arithmetic says.
        assert!(!ok(5.0, 20.01));
        assert!(!ok(-0.001, 1.0));
        assert!(!ok(f64::NAN, 1.0));
        assert!(!ok(0.0, f64::INFINITY));
        assert!(validate(None, Some("g"), 0.0, 1.0, 2.0).is_none());
    }

    #[test]
    fn a_trim_is_kept_to_the_thousandth_with_halves_to_the_even() {
        assert_eq!(thousandths(1.25), 1.25);
        assert_eq!(thousandths(0.0005), 0.0);
        assert_eq!(thousandths(0.0015), 0.002);
        assert_eq!(thousandths(2.0), 2.0);
    }
}
