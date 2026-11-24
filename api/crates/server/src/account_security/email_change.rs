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
use crate::auth::ceremony::{self, PasswordProof};
use crate::auth::{Caller, events, fresh, tokens};
use crate::auth::flow::Context;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, Problem, json_response};
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
    normalized_email: Option<String>,
    display_name: String,
    session_version: i64,
}

fn account_of(connection: &Connection, user: Id) -> rusqlite::Result<Option<Account>> {
    connection
        .query_row(
            "SELECT \"NormalizedEmail\", \"DisplayName\", \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1",
            [user],
            |row| {
                Ok(Account {
                    normalized_email: row.get(0)?,
                    display_name: row.get(1)?,
                    session_version: row.get(2)?,
                })
            },
        )
        .optional()
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
    if account.is_none() {
        return Err(ApiError::internal("the signed-in account is not there"));
    }
    if !is_fresh {
        return answer("second-step-required");
    }
    // An account made through a provider has no password to give: its fresh
    // proof stands in, and is asked for inside the transaction.
    let given = ceremony::check_given_password(&state, caller.user, request.password.as_deref()).await?;

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
            match ceremony::judge_given_password(tx, caller, &given, now)? {
                PasswordProof::Given => {}
                PasswordProof::WrongPassword => return Ok(Err("wrong-password")),
                PasswordProof::FreshProofNeeded => return Ok(Err("second-step-required")),
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

// ---- confirming it ----

/// A request for a change of address as it was written.
struct Waiting {
    request: Id,
    user: Id,
    session: Option<Id>,
    version: Option<i64>,
    new_email: String,
    normalized: String,
    used: Option<Timestamp>,
    expires: Timestamp,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmRequest {
    token: Option<String>,
}

/// What a link of the mail comes to. The token alone proves the new address,
/// so the link may be opened on another device; the change stands only while
/// the session that asked for it does, and the account is as it was.
pub async fn confirm(
    State(state): State<AppState>,
    origin: Origin,
    Json(Object(request)): Json<Object<ConfirmRequest>>,
) -> ApiResult<Response> {
    let token = request.token.as_deref().unwrap_or("").trim().to_string();
    let length = token.encode_utf16().count();
    if length == 0 || length > 256 {
        return answer("invalid");
    }
    let hash = tokens::hash(&token);
    let result = state
        .database()
        .write(move |tx| -> Result<&'static str, DbError> {
            let now = Timestamp::now();
            let found: Option<Waiting> = tx
                .query_row(
                    "SELECT \"Id\", \"UserId\", \"SessionId\", \"SecurityVersion\", \"NewEmail\", \"NormalizedNewEmail\", \"UsedAt\", \"ExpiresAt\"
                     FROM \"EmailChangeRequests\" WHERE \"TokenHash\" = ?1",
                    [&hash],
                    |row| {
                        Ok(Waiting {
                            request: row.get(0)?,
                            user: row.get(1)?,
                            session: row.get(2)?,
                            version: row.get(3)?,
                            new_email: row.get(4)?,
                            normalized: row.get(5)?,
                            used: row.get(6)?,
                            expires: row.get(7)?,
                        })
                    },
                )
                .optional()?;
            let Some(Waiting { request, user, session, version, new_email, normalized, used, expires }) = found else {
                return Ok("invalid");
            };
            let (disabled, recovery, current, previous): (bool, bool, i64, Option<String>) = tx.query_row(
                "SELECT \"IsDisabled\", \"SecurityRecoveryRequired\", \"SessionVersion\", \"NormalizedEmail\" FROM \"Users\" WHERE \"Id\" = ?1",
                [user],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            let Some(session) = session else {
                return Ok("invalid");
            };
            if used.is_some() || expires <= now || disabled || recovery || version != Some(current) {
                return Ok("invalid");
            }
            // Signing out or ending that one session cancels what it asked for.
            let standing: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"UserSessions\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL
                                AND \"ExpiresAt\" > ?3 AND \"AbsoluteExpiresAt\" > ?3 AND \"SecurityVersion\" = ?4)",
                params![session, user, now, current],
                |row| row.get(0),
            )?;
            if !standing {
                return Ok("invalid");
            }
            let taken: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"Users\" WHERE \"NormalizedEmail\" = ?1 AND \"Id\" <> ?2)",
                params![normalized, user],
                |row| row.get(0),
            )?;
            if taken {
                return Ok("taken");
            }
            // The alert goes to the address being replaced, read before it is.
            let alert = events::verified_email(tx, user)?;
            tx.execute(
                "UPDATE \"Users\" SET \"Email\" = ?2, \"NormalizedEmail\" = ?3, \"ProviderVerifiedEmail\" = NULL WHERE \"Id\" = ?1",
                params![user, new_email, normalized],
            )?;
            tx.execute("UPDATE \"EmailChangeRequests\" SET \"UsedAt\" = ?2 WHERE \"Id\" = ?1", params![request, now])?;
            let context = Context {
                ip_address: &origin.address,
                user_agent: &origin.user_agent,
                https: origin.https,
                trusted_device_cookie: None,
                passkeys_available: false,
            };
            // Codes and reset links sent to the old address stop working with it.
            ceremony::email_address_changed(tx, &context, user, &normalized, now)?;
            tx.execute(
                "UPDATE \"PasswordResetTokens\" SET \"UsedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL",
                params![user, now],
            )?;
            tx.execute(
                "UPDATE \"EmailChangeRequests\" SET \"UsedAt\" = ?3 WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL AND \"Id\" <> ?2",
                params![user, request, now],
            )?;
            events::record_from(
                tx,
                &context,
                user,
                events::EMAIL_CHANGED,
                if previous.is_none() { "address added" } else { "address changed" },
                alert.as_deref(),
            )?;
            Ok("changed")
        })
        .await?;
    answer(result)
}
