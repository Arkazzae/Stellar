//! Changing the account's address. The address receives the sign-in codes,
//! the security alerts and the reset links, so a change asks for the current
//! password and, once the account has a second step, one passed lately (an
//! account without a password gives that fresh proof alone); and the new
//! address has to prove that it receives mail before it replaces the old one.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use crate::instance::addresses::is_plain_address;
use crate::auth::{Caller, fresh, password, tokens};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Problem, json_response};
use crate::instance::names;
use crate::mail::setup::{self, MailSetup, Mailing};
use crate::mail::Outcome as Delivery;
use crate::servers::Object;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// How long the link in the mail to the new address works.
const LINK_LIFETIME: Duration = Duration::from_secs(3600);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    email: Option<String>,
    password: Option<String>,
}

#[derive(Serialize)]
struct Answer {
    result: &'static str,
}

fn answer(result: &'static str) -> ApiResult<Response> {
    Ok(json_response(StatusCode::OK, &Answer { result }))
}

/// What the account says that a change of address is decided on.
struct Account {
    hash: Option<String>,
    normalized_email: Option<String>,
    display_name: String,
    session_version: i64,
}

fn account_of(connection: &Connection, user: Id) -> rusqlite::Result<Option<Account>> {
    connection
        .query_row(
            "SELECT \"PasswordHash\", \"NormalizedEmail\", \"DisplayName\", \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1",
            [user],
            |row| {
                Ok(Account {
                    hash: row.get(0)?,
                    normalized_email: row.get(1)?,
                    display_name: row.get(2)?,
                    session_version: row.get(3)?,
                })
            },
        )
        .optional()
}

/// What a change that asks for the current password was given.
enum Proof {
    Given,
    WrongPassword,
    FreshProofNeeded,
}

/// The proof an address change takes: the account's password when it has one
/// (`proves` says it was checked against `checked_against`, and counts only if
/// that is still the stored hash), and when it has none the fresh proof the
/// session gives. Kept apart so that the shared call of `auth` can replace it.
fn password_or_fresh_proof(
    tx: &rusqlite::Transaction,
    caller: Caller,
    stored: Option<&str>,
    checked_against: Option<&str>,
    proves: bool,
    now: Timestamp,
) -> Result<Proof, DbError> {
    Ok(match stored {
        Some(hash) if proves && checked_against == Some(hash) => Proof::Given,
        Some(_) => Proof::WrongPassword,
        None => {
            if fresh::can_change_security(tx, caller.user, Some(caller.session), || false, now)? {
                Proof::Given
            } else {
                Proof::FreshProofNeeded
            }
        }
    })
}

/// What was written, so that the mail can go once it is committed.
struct Written {
    mailing: Mailing,
    request: Id,
    token: String,
    display_name: String,
}

pub async fn start(State(state): State<AppState>, caller: Caller, Json(Object(request)): Json<Object<StartRequest>>) -> ApiResult<Response> {
    let email = request.email.as_deref().unwrap_or("").trim().to_string();
    if !is_plain_address(&email) {
        let text = state.messages().get(MessageKey::EmailInvalid, &[])?;
        return Err(Problem::validation(vec![("email".into(), vec![text])]).into());
    }

    // Asked before the password, so that a stolen session cannot use this form
    // to try passwords on an account that has a second step.
    let (is_fresh, account) = state
        .database()
        .read(move |connection| -> Result<_, DbError> {
            let fresh = fresh::has_fresh_second_step(connection, caller.user, Some(caller.session), Timestamp::now())?;
            Ok((fresh, account_of(connection, caller.user)?))
        })
        .await?;
    let account = account.ok_or_else(|| ApiError::internal("the signed-in account is not there"))?;
    if !is_fresh {
        return answer("second-step-required");
    }
    // An account made through a provider has no password to give: its fresh
    // proof stands in, and is asked for inside the transaction.
    let proves = match (&account.hash, request.password) {
        (Some(hash), Some(given)) => {
            let hash = hash.clone();
            tokio::task::spawn_blocking(move || password::verify(&given, Some(&hash)))
                .await
                .map_err(ApiError::internal)?
        }
        _ => false,
    };
    let checked_against = account.hash;

    let normalized = names::to_upper(&email);
    let (inner, new_email) = (state.clone(), email.clone());
    let written = state
        .database()
        .write(move |tx| -> Result<Result<Written, &'static str>, DbError> {
            let now = Timestamp::now();
            if !fresh::has_fresh_second_step(tx, caller.user, Some(caller.session), now)? {
                return Ok(Err("second-step-required"));
            }
            let Some(account) = account_of(tx, caller.user)? else {
                return Ok(Err("second-step-required"));
            };
            match password_or_fresh_proof(tx, caller, account.hash.as_deref(), checked_against.as_deref(), proves, now)? {
                Proof::Given => {}
                Proof::WrongPassword => return Ok(Err("wrong-password")),
                Proof::FreshProofNeeded => return Ok(Err("second-step-required")),
            }
            if account.normalized_email.as_deref() == Some(normalized.as_str()) {
                return Ok(Err("same"));
            }
            let taken: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"Users\" WHERE \"NormalizedEmail\" = ?1 AND \"Id\" <> ?2)",
                params![normalized, caller.user],
                |row| row.get(0),
            )?;
            if taken {
                return Ok(Err("taken"));
            }
            let mailing = match setup::find(tx, inner.settings(), inner.sealer())? {
                MailSetup::Ready(mailing) => *mailing,
                MailSetup::Off | MailSetup::NotConfigured => return Ok(Err("mail-unavailable")),
            };
            let token = tokens::random(32);
            let request = Id::random();
            // Only the latest request counts.
            tx.execute(
                "UPDATE \"EmailChangeRequests\" SET \"UsedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL",
                params![caller.user, now],
            )?;
            tx.execute(
                "INSERT INTO \"EmailChangeRequests\" (\"Id\", \"UserId\", \"SessionId\", \"SecurityVersion\", \"NewEmail\", \"NormalizedNewEmail\",
                                                      \"TokenHash\", \"CreatedAt\", \"ExpiresAt\", \"UsedAt\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
                params![
                    request,
                    caller.user,
                    caller.session,
                    account.session_version,
                    new_email,
                    normalized,
                    tokens::hash(&token),
                    now,
                    now.plus(LINK_LIFETIME),
                ],
            )?;
            Ok(Ok(Written { mailing, request, token, display_name: account.display_name }))
        })
        .await?;
    let written = match written {
        Ok(written) => written,
        Err(result) => return answer(result),
    };

    let link = format!("{}/confirm-email?token={}", written.mailing.public_url, written.token);
    let message = stellarr_mail::email_change(
        &written.mailing.sender,
        &email,
        &written.display_name,
        &written.mailing.instance_name,
        &link,
        u128::from_be_bytes(*written.request.as_bytes()),
        state.messages(),
    );
    let delivered = match message {
        Ok(message) => matches!(state.mail().send(&message, &written.mailing.transport).await, Delivery::Sent { .. }),
        Err(_) => false,
    };
    if !delivered {
        tracing::error!(target: "Stellarr.Api.Mail", "Sending the address confirmation for request {} failed.", written.request);
        return answer("mail-unavailable");
    }
    answer("sent")
}
