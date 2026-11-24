//! Admits only clients that belong to this project.
//!
//! This is an allow-list rather than a block-list. The browser build, the
//! operator panel, the desktop client and the Android client announce
//! themselves with `X-Stellarr-Client` and a version. Anything that does not
//! is an abandoned build of an earlier product, or code written against an
//! API it has no agreement with. Naming every bad build is impossible; naming
//! the good ones is not.
//!
//! Only signing in and the calls a session starts with are gated. Refusing
//! those ends the session of a client that is not supported, and the rest of
//! the API is unreachable without one.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rusqlite::OptionalExtension;
use stellarr_messages::MessageKey;

use super::error::ApiError;
use super::pipeline::Pipeline;
use super::problem::Problem;
use crate::database::DbError;

pub const KIND_HEADER: &str = "X-Stellarr-Client";
pub const VERSION_HEADER: &str = "X-Stellarr-Client-Version";

const DESKTOP_KIND: &str = "desktop-linux";
const UNSUPPORTED_CLIENT: &str = "unsupported-client";

const GATED_PATHS: [&str; 7] = [
    "/api/auth/login",
    "/api/auth/passkey/login",
    "/api/auth/register",
    "/api/auth/desktop/browser/start",
    "/api/auth/desktop/browser/exchange",
    "/api/session",
    "/api/bootstrap",
];

fn is_gated(path: &str) -> bool {
    GATED_PATHS
        .iter()
        .any(|gated| path.eq_ignore_ascii_case(gated))
}

/// A version of two to four numbers, `1.4` or `0.1.3.2`. A number that is
/// left out ranks below zero, so `0.1` is older than `0.1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientVersion {
    parts: [i32; 4],
    count: usize,
}

impl ClientVersion {
    /// Each number is digits with an optional sign in front, and fits in 31
    /// bits; a negative one is refused.
    pub fn parse(text: &str) -> Option<ClientVersion> {
        let pieces: Vec<&str> = text.trim().split('.').collect();
        if !(2..=4).contains(&pieces.len()) {
            return None;
        }
        let mut parts = [-1; 4];
        for (part, piece) in parts.iter_mut().zip(&pieces) {
            let digits = piece.strip_prefix(['+', '-']).unwrap_or(piece);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let value: i64 = piece.parse().ok()?;
            *part = i32::try_from(value).ok().filter(|value| *value >= 0)?;
        }
        Some(ClientVersion {
            parts,
            count: pieces.len(),
        })
    }
}

impl PartialOrd for ClientVersion {
    fn partial_cmp(&self, other: &ClientVersion) -> Option<Ordering> {
        Some(self.parts.cmp(&other.parts))
    }
}

impl fmt::Display for ClientVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shown: Vec<String> = self.parts[..self.count]
            .iter()
            .map(ToString::to_string)
            .collect();
        f.write_str(&shown.join("."))
    }
}

pub(super) async fn middleware(
    State(pipeline): State<Arc<Pipeline>>,
    request: Request,
    next: Next,
) -> Response {
    if is_gated(request.uri().path()) {
        // The request itself cannot be held across the lookup below.
        let (kind, reported) = {
            let header = |name: &str| {
                request
                    .headers()
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };
            (header(KIND_HEADER), header(VERSION_HEADER))
        };
        if let Some(refusal) = find_refusal(&pipeline, &kind, &reported).await {
            return refusal;
        }
    }
    next.run(request).await
}

async fn find_refusal(pipeline: &Pipeline, kind: &str, reported: &str) -> Option<Response> {
    let settings = pipeline.state.settings();
    let clients = &settings.clients;
    let messages = pipeline.state.messages();
    let refuse = |code: &str,
                  key: MessageKey,
                  values: &[(&str, stellarr_messages::Value<'_>)],
                  minimum: Option<String>| {
        let title = match messages.get(key, values) {
            Ok(title) => title,
            Err(missing) => return ApiError::from(missing).into_response(),
        };
        let mut problem = Problem::new(StatusCode::FORBIDDEN)
            .title(title)
            .with("code", code);
        if let Some(minimum) = minimum {
            problem = problem.with("minimumVersion", minimum);
        }
        if let Some(path) = &clients.download_path {
            problem = problem.with("downloadPath", path.as_str());
        }
        problem.into_response()
    };

    if kind.is_empty() {
        return clients
            .require_identification
            .then(|| refuse(UNSUPPORTED_CLIENT, MessageKey::ClientUnsupported, &[], None));
    }
    if !clients
        .allowed
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(kind))
    {
        return Some(refuse(
            UNSUPPORTED_CLIENT,
            MessageKey::ClientUnsupported,
            &[],
            None,
        ));
    }
    let Some(version) = ClientVersion::parse(reported) else {
        return Some(refuse(
            UNSUPPORTED_CLIENT,
            MessageKey::ClientVersionMissing,
            &[],
            None,
        ));
    };
    // Only the desktop client has a floor to compare against.
    if !kind.eq_ignore_ascii_case(DESKTOP_KIND) {
        return None;
    }
    let minimum = find_minimum_desktop_version(pipeline).await?;
    if version >= minimum {
        return None;
    }
    Some(refuse(
        "outdated-client",
        MessageKey::ClientOutdated,
        &[("reported", &reported), ("minimum", &minimum)],
        Some(minimum.to_string()),
    ))
}

/// The floor that the client discovery document publishes as well: what the
/// operator saved, else the configured default. Both places state the same
/// number, so a client is never told it is current and then refused.
async fn find_minimum_desktop_version(pipeline: &Pipeline) -> Option<ClientVersion> {
    let saved = pipeline
        .state
        .database()
        .read(|connection| -> Result<Option<String>, DbError> {
            Ok(connection
                .query_row(
                    "SELECT \"DesktopMinimumVersion\" FROM \"OperationsSettings\" ORDER BY \"ServerId\" LIMIT 1",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten())
        })
        .await
        // With no readable database nobody is signing in anyway, and the
        // endpoint behind the gate will fail by itself. A current client must
        // not be turned away because this lookup did.
        .unwrap_or(None);
    let configured = pipeline.state.settings().desktop.minimum_version.as_deref();
    let floor = match saved.as_deref().map(str::trim) {
        Some(saved) if !saved.is_empty() => Some(saved),
        _ => configured,
    };
    floor.and_then(ClientVersion::parse)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> Option<String> {
        ClientVersion::parse(text).map(|version| version.to_string())
    }

    #[test]
    fn a_version_is_two_to_four_numbers() {
        assert_eq!(version("0.1.3").as_deref(), Some("0.1.3"));
        assert_eq!(version("1.4").as_deref(), Some("1.4"));
        assert_eq!(version("1.2.3.4").as_deref(), Some("1.2.3.4"));
        assert_eq!(version(" 1.2 ").as_deref(), Some("1.2"));
        assert_eq!(version("01.002").as_deref(), Some("1.2"));
        assert_eq!(version("+1.2").as_deref(), Some("1.2"));
        assert_eq!(version("2147483647.1").as_deref(), Some("2147483647.1"));
    }

    #[test]
    fn a_text_that_is_not_a_version_is_refused() {
        for text in [
            "",
            "2",
            "two",
            "2.0.0-beta",
            "1.2.3.4.5",
            "1..2",
            "1.2.",
            ".1.2",
            "1.-2",
            "99999999999.1.0",
            "2147483648.1",
            "１.２",
            "1.2.x",
        ] {
            assert_eq!(version(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_number_left_out_ranks_below_zero() {
        let at = |text: &str| ClientVersion::parse(text).unwrap();

        assert!(at("0.1") < at("0.1.0"));
        assert!(at("0.1.0") < at("0.1.0.0"));
        assert!(at("0.1.3") >= at("0.1.3"));
        assert!(at("0.1.1") < at("0.1.3"));
        assert!(at("9.9.9.9") > at("0.3.0"));
        assert!(at("0.2.9") < at("0.3.0"));
    }

    #[test]
    fn only_the_paths_of_a_session_are_gated_whatever_their_letter_case() {
        for path in GATED_PATHS {
            assert!(is_gated(path));
        }
        assert!(is_gated("/API/Session"));
        assert!(!is_gated("/health/live"));
        assert!(!is_gated("/api/auth/oidc"));
        assert!(!is_gated("/api/session/"));
    }
}
