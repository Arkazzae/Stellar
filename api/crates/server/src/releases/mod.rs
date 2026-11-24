//! What a client asks before anybody is signed in: which instance this is,
//! and which builds of the desktop and Android clients there are, to download
//! and to update to.
//!
//! The builds are not part of the image. The deploy scripts publish a package
//! beside a JSON manifest under `{data root}/releases/desktop` and
//! `.../android`, and every request reads the manifest again, so a new release
//! is offered without a restart. A manifest that cannot be trusted is a 503,
//! never a 404 or a 204, which would hide a broken publication behind clients
//! that merely never update.
//!
//! `GET /.well-known/stellarr/client`, `GET /api/desktop/release[/arch]`,
//! `GET /api/android/release[/apk]`, `GET /api/client/releases`,
//! `GET /api/desktop/update/{target}/{architecture}/{currentVersion}` and
//! `GET /api/desktop/update/download/{channel}/{bundle}`.
//!
//! None of them is gated by the client gate: it only looks at the calls a
//! session starts with.

mod catalogue;
mod discovery;
mod json;
mod manifest;
mod moment;
mod version;

use axum::Extension;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

pub use catalogue::{
    ANDROID, DESKTOP, Kind, Published, ReleaseDto, UpdateCandidate, current, desktop_version,
    find_update,
};
pub use manifest::Failure;

use crate::http::{
    ApiError, ApiResult, ClientInfo, Endpoints, Json, Path, Problem, Query, json_response,
};
use crate::state::AppState;
use stellarr_messages::MessageKey;

/// The download address stands for whatever release is newest, so it is
/// revalidated soon after a release instead of being cached as immutable.
const CACHE_CONTROL: &str = "public, max-age=60";
/// The one bundle that can replace its own files. A package installed by the
/// system package manager cannot, and offering it an AppImage would be
/// offering an install it has no way to perform.
const SELF_UPDATING_BUNDLE: &str = "appimage";
const LAST_ROLLOUT_BUCKET: i32 = 99;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/.well-known/stellarr/client", discovery::describe)
        .get("/api/desktop/release", describe_desktop)
        .get("/api/desktop/release/arch", download_desktop)
        .get("/api/android/release", describe_android)
        .get("/api/android/release/apk", download_android)
        .get("/api/client/releases", list)
        .get("/api/desktop/update/{target}/{architecture}/{current_version}", check_update)
        .get("/api/desktop/update/download/{channel}/{bundle}", download_update)
}

/// The scheme and host the request was made to, as the proxy in front of the
/// server says them.
fn requested_origin(headers: &HeaderMap, client: Option<&ClientInfo>) -> (String, String) {
    let scheme = client.map_or("http", |client| client.scheme.as_str());
    let host = client
        .and_then(|client| client.host.clone())
        .or_else(|| {
            headers
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        })
        .unwrap_or_default();
    (scheme.to_string(), host)
}

/// A catalogue read, which touches the disk, on a thread that may block.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Failure> + Send + 'static,
) -> Result<T, Failure> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|failure| Failure::Disk(std::io::Error::other(failure)))?
}

/// What a failed read of the catalogue is answered with: the problem that
/// says the release is unavailable, or the 500 of a disk that does not answer.
fn unavailable(state: &AppState, failure: Failure, title: MessageKey) -> ApiError {
    match failure {
        Failure::Untrusted => match state.messages().get(title, &[]) {
            Ok(title) => Problem::new(StatusCode::SERVICE_UNAVAILABLE).title(title).into(),
            Err(missing) => missing.into(),
        },
        Failure::Disk(error) => ApiError::internal(error),
    }
}

async fn published(state: &AppState, kind: &'static Kind) -> ApiResult<Published> {
    let root = state.settings().data_root.clone();
    match blocking(move || current(kind, &root)).await {
        Ok(Some(release)) => Ok(release),
        Ok(None) => Err(ApiError::not_found()),
        Err(failure) => Err(unavailable(state, failure, kind.unavailable)),
    }
}

fn cached(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(CACHE_CONTROL));
    response
}

async fn describe(state: AppState, kind: &'static Kind) -> ApiResult<Response> {
    let release = published(&state, kind).await?;
    Ok(cached(Json(release.dto()).into_response()))
}

async fn download(state: AppState, kind: &'static Kind, headers: HeaderMap) -> ApiResult<Response> {
    let release = published(&state, kind).await?;
    let opened = open(&release.file_path, release.size_bytes).await;
    let Some((file, length)) = opened else {
        return Err(unavailable(&state, Failure::Untrusted, kind.unavailable));
    };
    Ok(cached(
        crate::http::file::answer(
            &headers,
            crate::http::file::Download {
                file,
                length,
                content_type: kind.content_type,
                file_name: release.file_name,
                modified: release.published_at,
            },
        )
        .await,
    ))
}

/// The file, opened once, and measured on the handle that is served from, so
/// that a package replaced after it was checked is not served half old and
/// half new.
async fn open(path: &std::path::Path, declared: i64) -> Option<(tokio::fs::File, u64)> {
    let file = tokio::fs::File::open(path).await.ok()?;
    let length = file.metadata().await.ok()?.len();
    (i64::try_from(length) == Ok(declared)).then_some((file, length))
}

async fn describe_desktop(State(state): State<AppState>) -> ApiResult<Response> {
    describe(state, &DESKTOP).await
}

async fn describe_android(State(state): State<AppState>) -> ApiResult<Response> {
    describe(state, &ANDROID).await
}

async fn download_desktop(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    download(state, &DESKTOP, headers).await
}

async fn download_android(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    download(state, &ANDROID, headers).await
}

/// Every release on offer, the Android one first. A broken manifest hides
/// only its own platform, so that people can still download the other one;
/// the list is unavailable when nothing could be read at all.
async fn list(State(state): State<AppState>) -> ApiResult<Response> {
    let root = state.settings().data_root.clone();
    let (releases, any_broken) = blocking(move || {
        let mut releases = Vec::new();
        let mut any_broken = false;
        for kind in [&ANDROID, &DESKTOP] {
            match current(kind, &root) {
                Ok(Some(release)) => releases.push(release.dto()),
                Ok(None) => {}
                Err(Failure::Untrusted) => any_broken = true,
                Err(failure) => return Err(failure),
            }
        }
        Ok((releases, any_broken))
    })
    .await
    .map_err(|failure| unavailable(&state, failure, MessageKey::ReleaseListUnavailable))?;
    if releases.is_empty() && any_broken {
        return Err(unavailable(&state, Failure::Untrusted, MessageKey::ReleaseListUnavailable));
    }
    Ok(cached(Json(releases).into_response()))
}

/// What the updater inside an installed client expects from a dynamic update
/// server: the names are its own.
#[derive(Serialize)]
struct UpdateResponse {
    version: String,
    pub_date: String,
    url: String,
    signature: String,
    notes: String,
}

/// Answers an installed client asking whether it should replace itself. Every
/// refusal is 204 and never 404: up to date, outside the current stage of the
/// rollout, an unknown channel and a bundle that updates through the package
/// manager all mean "nothing for you" to the updater, and only a failure
/// should read as an error worth reporting. A manifest that cannot be trusted
/// is that failure, and answers 503.
async fn check_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    client: Option<Extension<ClientInfo>>,
    Path((target, architecture, installed)): Path<(String, String, String)>,
    Query(asked): Query<Vec<(String, String)>>,
) -> ApiResult<Response> {
    // A segment of the route is never empty.
    if [&target, &architecture, &installed].iter().any(|part| part.is_empty()) {
        return Err(ApiError::not_found());
    }
    let asked = Asked::of(&asked)?;
    let channel = asked.text("channel").unwrap_or_else(|| "stable".to_string());
    let bundle = asked
        .text("bundle")
        .unwrap_or_else(|| SELF_UPDATING_BUNDLE.to_string());
    let nothing = || Ok(StatusCode::NO_CONTENT.into_response());
    let (true, Some(installed)) = (
        bundle == SELF_UPDATING_BUNDLE,
        version::parse_stable(&installed),
    ) else {
        return nothing();
    };
    let root = state.settings().data_root.clone();
    let (looked_for, looked_in) = (channel.clone(), bundle.clone());
    let candidate = blocking(move || find_update(&root, &looked_for, &target, &architecture, &looked_in))
        .await
        .map_err(|failure| unavailable(&state, failure, MessageKey::DesktopUpdateUnavailable))?;
    let Some(candidate) = candidate.filter(|c| catalogue::is_newer(&c.version, installed)) else {
        return nothing();
    };
    // A staged rollout belongs to the release, not to the client. A client
    // that cannot say which bucket it is in must not be taken for an early
    // adopter, or an install that forgets its identity on every launch would
    // walk into each partial rollout.
    let bucket = match asked.bucket()? {
        Some(bucket @ 0..=LAST_ROLLOUT_BUCKET) => bucket,
        _ => LAST_ROLLOUT_BUCKET,
    };
    if bucket >= candidate.rollout_percent {
        return nothing();
    }
    let (scheme, host) = requested_origin(&headers, client.as_ref().map(|c| &c.0));
    let mut response = json_response(
        StatusCode::OK,
        &UpdateResponse {
            version: candidate.version,
            pub_date: round_trip(candidate.published_at),
            url: format!("{scheme}://{host}/api/desktop/update/download/{channel}/{bundle}"),
            signature: candidate.signature,
            notes: candidate.notes,
        },
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// A moment with all seven digits of the fraction, as the updater is used to
/// reading one: `2026-11-20T10:11:12.5000000Z`.
fn round_trip(moment: crate::values::Timestamp) -> String {
    let text = moment.to_wire_text();
    let body = text.trim_end_matches('Z');
    match body.split_once('.') {
        Some((whole, fraction)) => format!("{whole}.{fraction:0<7}Z"),
        None => format!("{body}.0000000Z"),
    }
}

/// The query of an update check, as the first server's binder read it. Names
/// match in any letter case. A name given more than once has its values
/// joined by commas, leaving out the empty ones, so that `bucket=3&bucket=` is
/// a 3 and `bucket=3&bucket=4` is not a number.
struct Asked<'a>(&'a [(String, String)]);

impl Asked<'_> {
    fn of(pairs: &[(String, String)]) -> ApiResult<Asked<'_>> {
        Ok(Asked(pairs))
    }

    fn text(&self, name: &str) -> Option<String> {
        let values: Vec<&str> = self
            .0
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect();
        (!values.is_empty()).then(|| {
            values
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(",")
        })
    }

    /// The bucket: none when it is not asked for, a bad request when it is
    /// asked for badly.
    fn bucket(&self) -> ApiResult<Option<i32>> {
        match self.text("bucket") {
            None => Ok(None),
            Some(text) => parse_int(&text).map(Some).ok_or_else(ApiError::bad_request),
        }
    }
}

/// A whole number of 32 bits: a sign, digits, and white space before the
/// sign and after the digits, as a number is read from text. Null characters
/// after all of it are let through, as that reader let them.
fn parse_int(text: &str) -> Option<i32> {
    let blank = |c: char| matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ');
    let text = text.trim_start_matches(blank);
    let text = text.trim_end_matches('\0').trim_end_matches(blank);
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.strip_prefix('+').unwrap_or(text).parse().ok()
}

async fn download_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((channel, bundle)): Path<(String, String)>,
) -> ApiResult<Response> {
    let root = state.settings().data_root.clone();
    // Only Linux x86_64 builds publish updater artifacts.
    let candidate = blocking(move || find_update(&root, &channel, "linux", "x86_64", &bundle))
        .await
        .map_err(|failure| unavailable(&state, failure, MessageKey::DesktopUpdateUnavailable))?
        .ok_or_else(ApiError::not_found)?;
    let Some((file, length)) = open(&candidate.file_path, candidate.size_bytes).await else {
        return Err(unavailable(&state, Failure::Untrusted, MessageKey::DesktopUpdateUnavailable));
    };
    Ok(cached(
        crate::http::file::answer(
            &headers,
            crate::http::file::Download {
                file,
                length,
                content_type: "application/octet-stream",
                file_name: candidate.file_name,
                modified: candidate.published_at,
            },
        )
        .await,
    ))
}
