//! `POST /api/uploads/images` and `POST /api/uploads/encrypted`: a file for a message, with the picture that
//! stands in for it before it plays or loads, or the opaque bytes of a file a
//! browser encrypted for an encrypted conversation.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use stellarr_messages::MessageKey;
use stellarr_storage::validation::{UploadKind, UploadSpec};

use super::attachments::{Attachment, Extra, compact, month};
use super::form::{self, Received, Slot, Staged};
use super::inspect::{self, is_placeholder, keep_attachment, keep_image, safe_file_name};
use super::numbers;
use super::transfer::{reserve, store_file, stored_or_rollback};
use super::refusals::{Texts, invalid, problem, refuse};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::multipart::Multipart;
use crate::http::{ApiError, ApiResult, MAX_REQUEST_BODY_BYTES};
use crate::http::created;
use crate::object_store::staging_dir;
use crate::permissions::membership::ActiveServer;
use crate::state::AppState;
use crate::values::Id;

const MAXIMUM_POSTER_BYTES: i64 = 2 * 1024 * 1024;
const MAXIMUM_PREVIEW_BYTES: i64 = 1024 * 1024;
const MAXIMUM_MEDIA_DURATION_SECONDS: f64 = 30.0 * 24.0 * 60.0 * 60.0;
const MAXIMUM_VOICE_MESSAGE_SECONDS: f64 = 10.0 * 60.0;
const MAXIMUM_MEDIA_DIMENSION: i32 = 32_768;
/// What AES-GCM adds to what it encrypts.
const TAG_BYTES: i64 = 16;
const ENCRYPTED_CONTENT_TYPE: &str = "application/octet-stream";

fn texts() -> Texts<'static> {
    Texts {
        unsupported_title: MessageKey::UploadTypeUnsupported,
        unsupported_detail: MessageKey::UploadTypesAllowed,
        too_large_title: MessageKey::UploadTooLarge,
        too_large_detail: &|kind| match kind {
            Some(UploadKind::Image) => MessageKey::UploadImageMaximum,
            Some(UploadKind::Audio) => MessageKey::UploadAudioMaximum,
            Some(UploadKind::Video) => MessageKey::UploadVideoMaximum,
            Some(UploadKind::File) => MessageKey::UploadFileMaximum,
            None => MessageKey::UploadOverLimit,
        },
        mismatch_title: MessageKey::UploadTypeMismatch,
    }
}

/// A number the client measured, which may be missing; a value that is not
/// one is a form that cannot be read.
fn measured<T>(
    received: &Received,
    name: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<Option<T>, ApiError> {
    match received.fields.get(name) {
        None => Ok(None),
        Some(Ok(text)) => parse(text).map(Some).ok_or_else(ApiError::bad_request),
        Some(Err(_)) => Err(ApiError::bad_request()),
    }
}

pub async fn upload(
    State(state): State<AppState>,
    caller: Caller,
    _server: ActiveServer,
    request: Request,
) -> ApiResult<Response> {
    let options = state.settings().uploads.options();
    let image_ceiling = (MAXIMUM_POSTER_BYTES, MAXIMUM_PREVIEW_BYTES);
    let keep_file = |content_type: &str| keep_attachment(content_type, &options);
    let keep_poster = |content_type: &str| keep_image(content_type, image_ceiling.0);
    let keep_preview = |content_type: &str| keep_image(content_type, image_ceiling.1);
    let slots = [
        Slot { name: "file", keeps: &keep_file },
        Slot { name: "poster", keeps: &keep_poster },
        Slot { name: "preview", keeps: &keep_preview },
    ];
    let mut received = read(
        &state,
        request,
        &slots,
        &["durationSeconds", "width", "height", "voiceMessage", "placeholder"],
    )
    .await?;
    let duration = measured(&received, "durationSeconds", numbers::decimal)?;
    let width = measured(&received, "width", numbers::integer)?;
    let height = measured(&received, "height", numbers::integer)?;
    let voice = measured(&received, "voiceMessage", numbers::flag)?;
    let placeholder = match received.fields.get("placeholder") {
        None => None,
        Some(Ok(text)) => Some(text.to_string()),
        // Too long to be the shape of a placeholder.
        Some(Err(_)) => Some(String::new()),
    };
    let mut files = received.files.drain(..);
    let file = files.next().flatten().ok_or_else(ApiError::bad_request)?;
    let poster = files.next().flatten();
    let preview = files.next().flatten();

    let spec = match inspect::attachment(&file, &options) {
        Ok(spec) => spec,
        Err(refused) => return Err(refuse(&state, &refused, &texts())?),
    };
    let is_voice = voice == Some(true);
    if let Some(key) = metadata_problem(spec.kind, is_voice, duration, width, height) {
        return Err(match key {
            Metadata::Voice => {
                let minutes = (MAXIMUM_VOICE_MESSAGE_SECONDS / 60.0) as i64;
                let text = state
                    .messages()
                    .get(MessageKey::VoiceMessageInvalid, &[("minutes", &minutes)])?;
                crate::http::Problem::validation(vec![("voiceMessage".to_string(), vec![text])])
                    .into()
            }
            Metadata::Duration => invalid(&state, "durationSeconds", MessageKey::MediaDurationInvalid)?,
            Metadata::Dimensions => invalid(&state, "dimensions", MessageKey::MediaDimensionsInvalid)?,
        });
    }

    let id = Id::random();
    let month = month();
    let poster = match poster {
        None => None,
        Some(staged) => match inspect::image(&staged, MAXIMUM_POSTER_BYTES) {
            Ok(poster_spec) if spec.kind == UploadKind::Video => Some((staged, poster_spec)),
            _ => return Err(invalid(&state, "poster", MessageKey::UploadPosterInvalid)?),
        },
    };
    let preview = match preview {
        None => None,
        Some(staged) => match inspect::image(&staged, MAXIMUM_PREVIEW_BYTES) {
            Ok(preview_spec)
                if spec.kind == UploadKind::Image
                    && matches!(preview_spec.content_type, "image/webp" | "image/jpeg") =>
            {
                Some((staged, preview_spec))
            }
            _ => return Err(invalid(&state, "preview", MessageKey::UploadPreviewInvalid)?),
        },
    };
    if let Some(text) = &placeholder
        && (spec.kind != UploadKind::Image || !is_placeholder(text))
    {
        return Err(invalid(&state, "placeholder", MessageKey::UploadPlaceholderInvalid)?);
    }

    let bytes = file.length
        + poster.as_ref().map_or(0, |(staged, _)| staged.length)
        + preview.as_ref().map_or(0, |(staged, _)| staged.length);
    let lease = reserve(&state, caller.user, i64::try_from(bytes).unwrap_or(i64::MAX)).await?;

    let n = compact(id);
    let key = format!("{}/{month}/{n}{}", spec.folder, spec.extension);
    let extra = |kind: &str, found: &Option<(Staged, UploadSpec)>| {
        found.as_ref().map(|(staged, spec)| Extra {
            key: format!("{kind}s/{month}/{n}{}", spec.extension),
            content_type: spec.content_type.to_string(),
            size: staged.length as i64,
        })
    };
    let attachment = Attachment {
        id,
        uploaded_by: caller.user,
        object_key: key,
        file_name: safe_file_name(&file.file_name, &spec),
        content_type: spec.content_type.to_string(),
        size: file.length as i64,
        width,
        height,
        duration_seconds: duration.map(|seconds| (seconds * 1000.0).round_ties_even() / 1000.0),
        is_voice_message: is_voice,
        poster: extra("poster", &poster),
        preview: extra("preview", &preview),
        placeholder,
        is_encrypted: false,
    };
    let mut stored = vec![(attachment.object_key.clone(), &file)];
    if let (Some(extra), Some((staged, _))) = (&attachment.poster, &poster) {
        stored.push((extra.key.clone(), staged));
    }
    if let (Some(extra), Some((staged, _))) = (&attachment.preview, &preview) {
        stored.push((extra.key.clone(), staged));
    }
    let response = store_and_record(&state, &stored, attachment).await;
    drop(lease);
    response
}

enum Metadata {
    Voice,
    Duration,
    Dimensions,
}

/// Checks the properties the client measured: duration, dimensions and
/// whether the file is a recorded voice message.
fn metadata_problem(
    kind: UploadKind,
    is_voice: bool,
    duration: Option<f64>,
    width: Option<i32>,
    height: Option<i32>,
) -> Option<Metadata> {
    if is_voice
        && (kind != UploadKind::Audio
            || !duration.is_some_and(|seconds| {
                seconds.is_finite() && seconds > 0.0 && seconds <= MAXIMUM_VOICE_MESSAGE_SECONDS
            }))
    {
        return Some(Metadata::Voice);
    }
    if let Some(seconds) = duration
        && (matches!(kind, UploadKind::Image | UploadKind::File)
            || !seconds.is_finite()
            || seconds <= 0.0
            || seconds > MAXIMUM_MEDIA_DURATION_SECONDS)
    {
        return Some(Metadata::Duration);
    }
    let out_of_range = |value: Option<i32>| value.is_some_and(|v| v <= 0 || v > MAXIMUM_MEDIA_DIMENSION);
    if width.is_some() != height.is_some()
        || out_of_range(width)
        || out_of_range(height)
        || (width.is_some() && matches!(kind, UploadKind::Audio | UploadKind::File))
    {
        return Some(Metadata::Dimensions);
    }
    None
}

/// Writes each staged file to the store under its key, then the row. A file
/// that did not make it, with its siblings, is taken back out of the store.
async fn store_and_record(
    state: &AppState,
    stored: &[(String, &Staged)],
    attachment: Attachment,
) -> ApiResult<Response> {
    let keys: Vec<String> = stored.iter().map(|(key, _)| key.clone()).collect();
    for (key, staged) in stored {
        if let Err(error) = store_file(state, key, staged).await {
            return stored_or_rollback(state, keys, Err(error)).await;
        }
    }
    let dto = attachment.dto();
    let location = dto_url(&attachment);
    let written = state
        .database()
        .write(move |tx| -> Result<(), DbError> { Ok(attachment.insert(tx)?) })
        .await;
    stored_or_rollback(state, keys, written.map_err(ApiError::from)).await?;
    Ok(created(location, &dto))
}

fn dto_url(attachment: &Attachment) -> String {
    format!("/assets/{}", attachment.id)
}

pub async fn encrypted(
    State(state): State<AppState>,
    caller: Caller,
    _server: ActiveServer,
    request: Request,
) -> ApiResult<Response> {
    let options = state.settings().uploads.options();
    let largest = [
        options.max_image_bytes,
        options.max_audio_bytes,
        options.max_video_bytes,
        options.max_file_bytes,
    ]
    .into_iter()
    .max()
    .unwrap_or(0)
    .saturating_add(TAG_BYTES);
    let keeps = |_: &str| Some(largest.max(0) as u64);
    let slots = [Slot { name: "file", keeps: &keeps }];
    let mut received = read(&state, request, &slots, &[]).await?;
    let file = received
        .files
        .remove(0)
        .ok_or_else(ApiError::bad_request)?;
    if file.length == 0 {
        return Err(invalid(&state, "file", MessageKey::FileEmpty)?);
    }
    if i64::try_from(file.length).map_or(true, |length| length > largest) {
        return Err(problem(
            &state,
            StatusCode::PAYLOAD_TOO_LARGE,
            MessageKey::UploadTooLarge,
            MessageKey::UploadOverLimit,
        )?);
    }
    let lease = reserve(&state, caller.user, file.length as i64).await?;
    let id = Id::random();
    let attachment = Attachment {
        id,
        uploaded_by: caller.user,
        object_key: format!("encrypted/{}/{}.bin", month(), compact(id)),
        file_name: "encrypted.bin".to_string(),
        content_type: ENCRYPTED_CONTENT_TYPE.to_string(),
        size: file.length as i64,
        width: None,
        height: None,
        duration_seconds: None,
        is_voice_message: false,
        poster: None,
        preview: None,
        placeholder: None,
        is_encrypted: true,
    };
    let stored = vec![(attachment.object_key.clone(), &file)];
    let response = store_and_record(&state, &stored, attachment).await;
    drop(lease);
    response
}

/// Reads the form of a request into the slots a route asks for.
pub async fn read(
    state: &AppState,
    request: Request,
    slots: &[Slot<'_>],
    fields: &[&str],
) -> Result<Received, ApiError> {
    let (parts, body) = request.into_parts();
    let mut reader = Multipart::open(&parts.headers, body, MAX_REQUEST_BODY_BYTES)?;
    form::read(&mut reader, &staging_dir(state.settings()), slots, fields).await
}
