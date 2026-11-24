//! `GET /api/emoji/{id}/image`: the picture of an emoji, to the members of
//! its server.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use stellarr_storage::Error as StoreError;

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Path};
use crate::permissions::membership::ActiveServer;
use crate::state::AppState;
use crate::values::Guid;

pub async fn image(
    State(state): State<AppState>,
    _caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(Guid(emoji)): Path<Guid>,
) -> ApiResult<Response> {
    let found: Option<(String, String, i64)> = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            Ok(db
                .query_row(
                    "SELECT \"ObjectKey\", \"ContentType\", \"SizeBytes\" FROM \"ServerEmoji\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
                    params![emoji, server],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?)
        })
        .await?;
    let Some((key, content_type, size)) = found else {
        return Err(ApiError::not_found());
    };
    let stored = match state.storage().get(&key, None).await {
        Ok(stored) => stored,
        Err(StoreError::NotFound(_)) => return Err(ApiError::not_found()),
        Err(error) => return Err(ApiError::internal(error)),
    };
    let mut response = Response::new(Body::empty());
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(size.max(0) as u64));
    // The address changes with the emoji, so the browser may keep it.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400, immutable"),
    );
    *response.status_mut() = StatusCode::OK;
    *response.body_mut() = crate::assets::stream(stored);
    Ok(response)
}
