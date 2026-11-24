//! `GET /assets/{id}`, `/assets/{id}/poster` and `/assets/{id}/preview`:
//! stored files, to the people who may read them.
//!
//! A file is streamed from the store as the client takes it, never held whole.
//! The range is asked of the store and not cut from the bytes, so that a
//! browser's seek in a media file costs the slice and not the file. Only the
//! first range of a `Range` header is understood, and only from a start: a
//! form that is not understood is answered with the whole file.

mod access;
pub(crate) mod range;

use std::io;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use stellarr_storage::validation::{UploadKind, spec_for};
use stellarr_storage::{Error as StoreError, ObjectBody, StoredObject};

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Endpoints, Path};
use crate::state::AppState;
use crate::values::Id;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/assets/{id}", download)
        .authorized()
        .get("/assets/{id}/poster", poster)
        .authorized()
        .get("/assets/{id}/preview", preview)
        .authorized()
}

/// What a download needs of a row.
struct Row {
    object_key: String,
    file_name: String,
    content_type: String,
    is_encrypted: bool,
    poster: Option<Still>,
    preview: Option<Still>,
}

struct Still {
    key: String,
    content_type: String,
    size: Option<i64>,
}

/// The attachment, or none when it does not exist or the caller may not read
/// it. Both look the same from outside, so that an address reveals nothing
/// about attachments of other people.
async fn find_readable(state: &AppState, user: Id, id: Id) -> Result<Option<Row>, ApiError> {
    Ok(state
        .database()
        .read(move |db| -> Result<Option<Row>, DbError> {
            let row = db
                .query_row(
                    "SELECT \"ObjectKey\", \"FileName\", \"ContentType\", \"IsEncrypted\",
                            \"PosterObjectKey\", \"PosterContentType\", \"PosterSizeBytes\",
                            \"PreviewObjectKey\", \"PreviewContentType\", \"PreviewSizeBytes\"
                     FROM \"ImageAttachments\" WHERE \"Id\" = ?1",
                    params![id],
                    |row| {
                        let still = |key: usize| -> rusqlite::Result<Option<Still>> {
                            let found: Option<String> = row.get(key)?;
                            Ok(match (found, row.get::<_, Option<String>>(key + 1)?) {
                                (Some(key_text), Some(content_type)) => Some(Still {
                                    key: key_text,
                                    content_type,
                                    size: row.get(key + 2)?,
                                }),
                                _ => None,
                            })
                        };
                        Ok(Row {
                            object_key: row.get(0)?,
                            file_name: row.get(1)?,
                            content_type: row.get(2)?,
                            is_encrypted: row.get(3)?,
                            poster: still(4)?,
                            preview: still(7)?,
                        })
                    },
                )
                .optional()?;
            match row {
                Some(row) if access::can_read(db, user, id)? => Ok(Some(row)),
                _ => Ok(None),
            }
        })
        .await?)
}

async fn download(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let Some(row) = find_readable(&state, caller.user, id).await? else {
        return Err(ApiError::not_found());
    };
    let asked = range::first_range(&headers);
    let stored = match state.storage().get(&row.object_key, asked).await {
        Ok(stored) => stored,
        Err(StoreError::RangeNotSatisfiable(_)) => {
            return Err(ApiError::status(StatusCode::RANGE_NOT_SATISFIABLE));
        }
        Err(StoreError::NotFound(_)) => return Err(ApiError::not_found()),
        Err(error) => return Err(ApiError::internal(error)),
    };
    let mut response = Response::new(Body::empty());
    let out = response.headers_mut();
    // Without this a browser does not even try to ask for a range.
    out.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    set(out, header::CONTENT_TYPE, &row.content_type);
    set(out, header::CONTENT_LENGTH, &stored.length.to_string());
    out.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // Plain files are offered as downloads, and so are encrypted bytes, which
    // only a browser with the key can make anything of. Media is played in
    // the page.
    let generic = row.is_encrypted
        || spec_for(&row.content_type).is_some_and(|spec| spec.kind == UploadKind::File);
    if generic {
        set(
            out,
            header::CONTENT_DISPOSITION,
            &format!(
                "attachment; filename*=UTF-8''{}",
                escape_data_string(&row.file_name)
            ),
        );
    }
    if let Some(served) = stored.range {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        let last = served.to.unwrap_or(stored.total_length.saturating_sub(1));
        set(
            response.headers_mut(),
            header::CONTENT_RANGE,
            &format!("bytes {}-{last}/{}", served.from, stored.total_length),
        );
    }
    *response.body_mut() = stream(stored);
    Ok(response)
}

async fn poster(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let row = find_readable(&state, caller.user, id).await?;
    serve_still(&state, row.and_then(|row| row.poster)).await
}

async fn preview(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let row = find_readable(&state, caller.user, id).await?;
    serve_still(&state, row.and_then(|row| row.preview)).await
}

/// A poster or preview, which is small and always sent whole.
async fn serve_still(state: &AppState, still: Option<Still>) -> ApiResult<Response> {
    let Some(still) = still else {
        return Err(ApiError::not_found());
    };
    let stored = match state.storage().get(&still.key, None).await {
        Ok(stored) => stored,
        Err(StoreError::NotFound(_)) => return Err(ApiError::not_found()),
        Err(error) => return Err(ApiError::internal(error)),
    };
    let mut response = Response::new(Body::empty());
    let out = response.headers_mut();
    out.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    set(out, header::CONTENT_TYPE, &still.content_type);
    set(
        out,
        header::CONTENT_LENGTH,
        &still.size.unwrap_or(stored.length as i64).to_string(),
    );
    *response.body_mut() = stream(stored);
    Ok(response)
}

fn set(headers: &mut HeaderMap, name: header::HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

/// A name as a URL component, as the first server escaped it: letters,
/// digits and `-_.~` stay, every other byte is `%` and two capital digits.
pub(crate) fn escape_data_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Hands the object to the client as it takes it: a few pieces wait for a
/// slow reader, and a client that goes stops the reading.
pub(crate) fn stream(stored: StoredObject) -> Body {
    let (sender, body) = crate::http::stream::channel(2);
    let mut object: ObjectBody = stored.body;
    tokio::spawn(async move {
        loop {
            match object.chunk().await {
                Ok(Some(piece)) => {
                    if sender.send(Ok(piece)).await.is_err() {
                        return;
                    }
                }
                Ok(None) => return,
                Err(error) => {
                    let _ = sender.send(Err(io::Error::other(error.to_string()))).await;
                    return;
                }
            }
        }
    });
    body
}
