//! `POST /api/auth/desktop/browser/start`: the desktop application asks to be
//! signed in through the system browser.
//!
//! The answer is a request id, a secret that only the application is given,
//! and the address the application opens in the browser. The secret is kept
//! as a hash, so that nothing read from the database can complete a request.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::params;
use serde::Serialize;

use super::no_store;
use crate::encoding::percent_encode;
use crate::auth::tokens;
use crate::database::DbError;
use crate::http::{ApiResult, json_response};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// How long the person has to approve in the browser.
pub const REQUEST_LIFETIME: Duration = Duration::from_secs(2 * 60);
/// The secret's length in bytes before it is written as text.
const SECRET_BYTES: usize = 32;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    request_id: Id,
    secret: String,
    verification_url: String,
    expires_at: Timestamp,
}

pub async fn start(State(state): State<AppState>) -> ApiResult<Response> {
    // The one place this group asks which providers are set up: only an
    // instance that has a provider to send the person to offers this.
    if !state.settings().oidc.any_set_up() {
        return Ok(no_store(StatusCode::NOT_FOUND.into_response()));
    }
    let login_paths: Vec<String> = state
        .settings()
        .oidc
        .providers
        .iter()
        .map(|provider| provider.login_path())
        .collect();
    let first = &login_paths[0];

    let now = Timestamp::now();
    let secret = tokens::random(SECRET_BYTES);
    let hash = tokens::hash(&secret);
    let id = Id::random();
    let expires_at = now.plus(REQUEST_LIFETIME);
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            // Requests that ran out are of no use to anybody.
            tx.execute(
                "DELETE FROM \"DesktopBrowserLogins\" WHERE \"ExpiresAt\" <= ?1",
                params![now],
            )?;
            tx.execute(
                "INSERT INTO \"DesktopBrowserLogins\" (\"Id\", \"SecretHash\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\")
                 VALUES (?1, ?2, 0, ?3, ?4)",
                params![id, hash, now, expires_at],
            )?;
            Ok(())
        })
        .await?;

    // The browser signs in with the provider first and lands on the approval.
    let approval = format!("/api/auth/desktop/browser/verify/{id}");
    let returned = percent_encode(&approval);
    let verification_url = if login_paths.len() == 1 {
        format!("{first}?returnUrl={returned}")
    } else {
        format!("/login?returnUrl={returned}")
    };
    Ok(no_store(json_response(
        StatusCode::OK,
        &Started {
            request_id: id,
            secret,
            verification_url,
            expires_at,
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_escaped_to_one_query_value() {
        assert_eq!(
            percent_encode("/api/auth/desktop/browser/verify/0f8bcc00-f25f-4084-9a08-62e11f1fab22"),
            "%2Fapi%2Fauth%2Fdesktop%2Fbrowser%2Fverify%2F0f8bcc00-f25f-4084-9a08-62e11f1fab22"
        );
        assert_eq!(percent_encode("a b&c=d~é"), "a%20b%26c%3Dd~%C3%A9");
    }
}
