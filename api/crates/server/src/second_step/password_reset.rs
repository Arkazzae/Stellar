//! Setting a new password through a link that is mailed to the address of the
//! account, for somebody who forgot theirs.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use stellarr_messages::MessageKey;

use super::common::body_of;
use crate::auth::{events, password, recovery, tokens};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, Problem};
use crate::mail::{self, MailSetup, Outcome};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// How long a link stays good.
const LINK_LIFETIME: Duration = Duration::from_secs(60 * 60);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgotRequest {
    email: Option<String>,
}

/// Mails a link to the address, if it is an account's. Every answer is the same
/// 202, whether or not the address belongs to an account and whether or not a
/// mail could go out, and comes as slowly.
pub async fn forgot(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<ForgotRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, false) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    // What an account that exists costs (a lookup, a write) goes on beside the
    // wait every answer has, and the answer comes when the wait is over. The
    // mail is sent after that write and the answer does not depend on it, so a
    // slow mail server cannot make a known address the one whose answer is
    // late.
    let wait = tokio::time::sleep(Duration::from_millis(u64::from(tokens::random_below(100)) + 80));
    let (prepared, ()) = tokio::join!(prepare(&state, request.email.unwrap_or_default()), wait);
    if let Some(link) = prepared? {
        // A task of its own, so that it runs to the end when the client has
        // gone. If the server stops in the moment between the write and the
        // delivery, the mail is not sent: the link is stored and good, the
        // earlier links are still good too (they are retired only once a new
        // one is out), and asking again sends another.
        tokio::spawn(async move {
            if let Err(error) = link.deliver(&state).await {
                tracing::error!(target: "Stellarr.Api.Mail", "Retiring the earlier reset links failed: {error}");
            }
        });
    }
    Ok(StatusCode::ACCEPTED.into_response())
}

/// A link that is written and waits to be mailed.
struct Link {
    mailing: crate::mail::Mailing,
    address: String,
    display_name: String,
    raw: String,
    token: Id,
    earlier: Vec<Id>,
    now: Timestamp,
}

/// Looks the address up and, if it is an account's and the instance is set up
/// to send recovery mail, writes a link for it.
async fn prepare(state: &AppState, email: String) -> Result<Option<Link>, DbError> {
    let normalized = crate::instance::names::to_upper(email.trim());
    let found = state
        .database()
        .read(move |connection| -> Result<Option<(Id, String, String)>, DbError> {
            Ok(connection
                .query_row(
                    "SELECT \"Id\", \"Email\", \"DisplayName\" FROM \"Users\" WHERE \"NormalizedEmail\" = ?1 AND \"IsDisabled\" = 0",
                    [normalized],
                    |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?.unwrap_or_default(), row.get(2)?)),
                )
                .optional()?)
        })
        .await?;
    let Some((user, address, display_name)) = found else {
        return Ok(None);
    };
    let mailing = match mail::setup::find_for(state).await? {
        MailSetup::Ready(mailing) if mailing.switches.recovery => *mailing,
        _ => return Ok(None),
    };
    let now = Timestamp::now();
    let raw = tokens::random(32);
    let (token, hash) = (Id::random(), tokens::hash(&raw));
    let earlier = state
        .database()
        .write(move |tx| -> Result<Vec<Id>, DbError> {
            let mut statement = tx.prepare(
                "SELECT \"Id\" FROM \"PasswordResetTokens\" WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL",
            )?;
            let earlier = statement
                .query_map([user], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<Id>>>()?;
            drop(statement);
            tx.execute(
                "INSERT INTO \"PasswordResetTokens\" (\"Id\", \"UserId\", \"TokenHash\", \"CreatedAt\", \"ExpiresAt\") VALUES (?1, ?2, ?3, ?4, ?5)",
                params![token, user, hash, now, now.plus(LINK_LIFETIME)],
            )?;
            Ok(earlier)
        })
        .await?;
    Ok(Some(Link { mailing, address, display_name, raw, token, earlier, now }))
}

impl Link {
    /// Mails the link. The older links go only once a new one is out, so that
    /// an unreachable provider does not take away the last link the person was
    /// sent.
    async fn deliver(self, state: &AppState) -> Result<(), DbError> {
        let link = format!("{}/reset-password?token={}", self.mailing.public_url, self.raw);
        let message = stellarr_mail::password_reset(
            &self.mailing.sender,
            &self.address,
            &self.display_name,
            &self.mailing.instance_name,
            &link,
            u128::from_be_bytes(*self.token.as_bytes()),
            state.messages(),
        );
        let outcome = match message {
            Ok(message) => state.mail().send(&message, &self.mailing.transport).await,
            Err(error) => Outcome::Refused { reason: error.to_string() },
        };
        if !outcome.is_sent() {
            // The new link stays valid: a provider that timed out may well
            // have sent it.
            tracing::error!(target: "Stellarr.Api.Mail", "Sending the password reset mail for token {} failed.", self.token);
            return Ok(());
        }
        let (earlier, now) = (self.earlier, self.now);
        state
            .database()
            .write(move |tx| -> Result<(), DbError> {
                for id in earlier {
                    tx.execute(
                        "UPDATE \"PasswordResetTokens\" SET \"UsedAt\" = ?2 WHERE \"Id\" = ?1",
                        params![id, now],
                    )?;
                }
                Ok(())
            })
            .await
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetRequest {
    token: Option<String>,
    new_password: Option<String>,
}

/// Sets the new password once, and ends everything the old one signed in.
pub async fn reset(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<ResetRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, false) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let messages = state.messages();
    let mut errors = Vec::new();
    let new_password = request.new_password.as_deref();
    if password::is_too_short(new_password) {
        errors.push((
            "newPassword".to_string(),
            vec![messages.get(MessageKey::PasswordTooShort, &[("min", &password::MIN_LENGTH)])?],
        ));
    } else if password::is_too_long(new_password) {
        errors.push((
            "newPassword".to_string(),
            vec![messages.get(MessageKey::PasswordTooLong, &[("max", &password::MAX_LENGTH)])?],
        ));
    }
    let token = request.token.unwrap_or_default();
    if token.trim().is_empty() {
        errors.push((
            "token".to_string(),
            vec![messages.get(MessageKey::PasswordResetLinkInvalid, &[])?],
        ));
    }
    if !errors.is_empty() {
        return Ok(Problem::validation(errors).into_response());
    }
    let new_password = new_password.unwrap_or_default().to_string();
    let hash = tokens::hash(&token);

    // Nothing is derived for a link that is no good.
    let usable = {
        let hash = hash.clone();
        state
            .database()
            .read(move |connection| -> Result<bool, DbError> { Ok(link_is_good(connection, &hash, Timestamp::now())?.is_some()) })
            .await?
    };
    if !usable {
        return link_invalid(&state);
    }
    // The derivation takes long and there is one writer: it is made before the
    // write, and the link is looked at again inside it.
    let derived = tokio::task::spawn_blocking(move || password::hash(&new_password))
        .await
        .map_err(ApiError::internal)?;
    let reset = state
        .database()
        .write(move |tx| -> Result<bool, DbError> {
            let now = Timestamp::now();
            let Some(user) = link_is_good(tx, &hash, now)? else {
                return Ok(false);
            };
            tx.execute(
                "UPDATE \"Users\" SET \"PasswordHash\" = ?2 WHERE \"Id\" = ?1",
                params![user, derived],
            )?;
            // Every cookie issued before this point stops working, and there
            // is no session of the person's to keep.
            recovery::credentials_changed(tx, user, None, now)?;
            tx.execute(
                "UPDATE \"PasswordResetTokens\" SET \"UsedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL",
                params![user, now],
            )?;
            let alert = events::verified_email(tx, user)?;
            events::record(tx, user, events::PASSWORD_CHANGED, "", "", "password reset", alert.as_deref())?;
            Ok(true)
        })
        .await?;
    if !reset {
        return link_invalid(&state);
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The account a link opens, if it is unused, has not run out and its account
/// is not disabled.
fn link_is_good(
    connection: &rusqlite::Connection,
    hash: &str,
    now: Timestamp,
) -> rusqlite::Result<Option<Id>> {
    connection
        .query_row(
            "SELECT t.\"UserId\" FROM \"PasswordResetTokens\" t JOIN \"Users\" u ON u.\"Id\" = t.\"UserId\"
             WHERE t.\"TokenHash\" = ?1 AND t.\"UsedAt\" IS NULL AND t.\"ExpiresAt\" > ?2 AND u.\"IsDisabled\" = 0",
            params![hash, now],
            |row| row.get(0),
        )
        .optional()
}

fn link_invalid(state: &AppState) -> ApiResult<Response> {
    let title = state.messages().get(MessageKey::PasswordResetLinkInvalid, &[])?;
    Ok(Problem::new(StatusCode::BAD_REQUEST).title(title).into_response())
}
