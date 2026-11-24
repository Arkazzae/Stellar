//! The page the person opens in the browser to approve a request, and the post
//! that approves it.
//!
//! Opening the page binds the request to the person and to the browser session
//! they opened it in, with a token that only the page carries. Approving needs
//! that token back, from that person, in that session, before the request runs
//! out, and records what the session had proved (how it was signed in and
//! whether it passed a second step) for the session the application will be
//! given.

use axum::Extension;
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use stellarr_messages::{MessageKey, Messages};

use super::form;
use super::no_store;
use super::page::{self, Confirmation};
use super::same_origin::is_same_origin;
use crate::auth::{Caller, events, tokens};
use crate::database::DbError;
use crate::http::{ApiResult, ClientInfo, Origin, Path, json_response};
use crate::state::AppState;
use crate::values::{Guid, Id, Timestamp};

/// The shortest and the longest token the form is asked to carry, in UTF-16
/// units, as the first server counted them.
const TOKEN_LENGTH: std::ops::RangeInclusive<usize> = 16..=512;

/// What the row of a request says, as far as this page cares.
struct Request {
    expires_at: Timestamp,
    redeemed_at: Option<Timestamp>,
    approved_by: Option<Id>,
    approved_at: Option<Timestamp>,
    intended_for: Option<Id>,
    bound_session: Option<Id>,
    token_hash: Option<String>,
}

fn find(connection: &Connection, id: Id) -> Result<Option<Request>, DbError> {
    Ok(connection
        .query_row(
            "SELECT \"ExpiresAt\", \"RedeemedAt\", \"UserId\", \"ApprovedAt\", \"ApprovalUserId\", \"ApprovalSessionId\", \"ApprovalTokenHash\"
             FROM \"DesktopBrowserLogins\" WHERE \"Id\" = ?1",
            [id],
            |row| {
                Ok(Request {
                    expires_at: row.get(0)?,
                    redeemed_at: row.get(1)?,
                    approved_by: row.get(2)?,
                    approved_at: row.get(3)?,
                    intended_for: row.get(4)?,
                    bound_session: row.get(5)?,
                    token_hash: row.get(6)?,
                })
            },
        )
        .optional()?)
}

/// What the browser session that opened the page had proved.
struct Source {
    id: Id,
    primary_source: String,
    mfa_completed_at: Option<Timestamp>,
    mfa_evidence: Option<String>,
}

/// The session of the caller, if it is still good: of this account, not ended,
/// not past its absolute end, and of an account that is not disabled.
fn source_session(
    connection: &Connection,
    session: Id,
    user: Id,
    now: Timestamp,
) -> Result<Option<Source>, DbError> {
    Ok(connection
        .query_row(
            "SELECT s.\"PrimarySource\", s.\"MfaCompletedAt\", s.\"MfaEvidence\"
             FROM \"UserSessions\" s JOIN \"Users\" u ON u.\"Id\" = s.\"UserId\"
             WHERE s.\"Id\" = ?1 AND s.\"UserId\" = ?2 AND s.\"RevokedAt\" IS NULL AND s.\"ExpiresAt\" > ?3
               AND u.\"IsDisabled\" = 0 AND s.\"AbsoluteExpiresAt\" > ?3",
            params![session, user, now],
            |row| {
                Ok(Source {
                    id: session,
                    primary_source: row.get(0)?,
                    mfa_completed_at: row.get(1)?,
                    mfa_evidence: row.get(2)?,
                })
            },
        )
        .optional()?)
}

/// What the page or the post comes to, before it is written as an answer.
enum Outcome {
    /// The request is unknown, run out, used, or the form is not right.
    Gone,
    /// The request is another account's.
    Taken,
    /// The session or the account no longer allows it.
    Forbidden,
    /// Already approved by this person.
    Done,
    /// Ask the person: the token that binds the form.
    Ask(String),
}

fn answer(state: &AppState, outcome: Outcome, request: Id) -> ApiResult<Response> {
    let messages = state.messages();
    let language = messages.language().code();
    let notice = |heading: MessageKey, body: MessageKey, status| -> ApiResult<Response> {
        Ok(page::notice(
            language,
            &messages.get(heading, &[])?,
            &messages.get(body, &[])?,
            status,
        ))
    };
    match outcome {
        Outcome::Gone => notice(
            MessageKey::BrowserLoginExpiredHeading,
            MessageKey::BrowserLoginExpiredBody,
            StatusCode::GONE,
        ),
        Outcome::Done => notice(
            MessageKey::BrowserLoginDoneHeading,
            MessageKey::BrowserLoginDoneBody,
            StatusCode::OK,
        ),
        Outcome::Taken => Ok(taken(messages)?),
        Outcome::Forbidden => Ok(no_store(StatusCode::FORBIDDEN.into_response())),
        Outcome::Ask(token) => Ok(page::confirmation(&Confirmation {
            language,
            heading: &messages.get(MessageKey::BrowserLoginConfirmHeading, &[])?,
            body: &messages.get(MessageKey::BrowserLoginConfirmBody, &[])?,
            button: &messages.get(MessageKey::BrowserLoginConfirmButton, &[])?,
            action: &format!("/api/auth/desktop/browser/verify/{request}"),
            token: &token,
        })),
    }
}

fn taken(messages: &Messages) -> Result<Response, stellarr_messages::MissingValue> {
    #[derive(Serialize)]
    struct Title {
        title: String,
    }
    let title = messages.get(MessageKey::BrowserLoginApprovedByAnother, &[])?;
    Ok(no_store(json_response(
        StatusCode::CONFLICT,
        &Title { title },
    )))
}

/// `GET .../verify/{id}`: shows the confirmation, and binds the request to
/// the person and session that opened it. It never approves by itself.
pub async fn show(
    State(state): State<AppState>,
    caller: Caller,
    Path(Guid(request)): Path<Guid>,
) -> ApiResult<Response> {
    let outcome = state
        .database()
        .write(move |tx| -> Result<Outcome, DbError> {
            let now = Timestamp::now();
            let Some(login) = find(tx, request)? else {
                return Ok(Outcome::Gone);
            };
            if login.expires_at <= now || login.redeemed_at.is_some() {
                return Ok(Outcome::Gone);
            }
            if login.approved_by.is_some_and(|by| by != caller.user)
                || login.intended_for.is_some_and(|by| by != caller.user)
            {
                return Ok(Outcome::Taken);
            }
            if login.approved_at.is_some() {
                return Ok(Outcome::Done);
            }
            let Some(source) = source_session(tx, caller.session, caller.user, now)? else {
                return Ok(Outcome::Forbidden);
            };
            // Each look at the page makes a new token, so that an earlier page
            // (another tab, or one that was copied) stops working.
            let token = tokens::random(32);
            tx.execute(
                "UPDATE \"DesktopBrowserLogins\" SET \"ApprovalUserId\" = ?2, \"ApprovalSessionId\" = ?3, \"ApprovalTokenHash\" = ?4
                 WHERE \"Id\" = ?1",
                params![request, caller.user, source.id, tokens::hash(&token)],
            )?;
            Ok(Outcome::Ask(token))
        })
        .await?;
    Ok(no_store(answer(&state, outcome, request)?))
}

/// `POST .../verify/{id}`: approves, when the form carries the token the page
/// was given.
pub async fn approve(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    Extension(client): Extension<ClientInfo>,
    Path(Guid(request)): Path<Guid>,
    body: Body,
) -> ApiResult<Response> {
    if !is_same_origin(&origin.headers, &client) {
        return Ok(no_store(StatusCode::BAD_REQUEST.into_response()));
    }
    let token = form::read_token(&origin.headers, body).await?;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Outcome, DbError> {
            let now = Timestamp::now();
            let login = find(tx, request)?;
            let source = source_session(tx, caller.session, caller.user, now)?;
            let (Some(login), Some(source)) = (login, source) else {
                return Ok(Outcome::Gone);
            };
            let length = token.encode_utf16().count();
            let stored = match login.token_hash.as_deref() {
                Some(hash) if hash.len() == 64 => hash,
                _ => return Ok(Outcome::Gone),
            };
            if login.expires_at <= now
                || login.redeemed_at.is_some()
                || login.approved_at.is_some()
                || login.intended_for != Some(caller.user)
                || login.bound_session != Some(source.id)
                || !TOKEN_LENGTH.contains(&length)
                || !tokens::matches(&token, stored)
            {
                return Ok(Outcome::Gone);
            }
            if login.approved_by.is_some_and(|by| by != caller.user) {
                return Ok(Outcome::Taken);
            }
            let version: Option<i64> = tx
                .query_row(
                    "SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1 AND \"IsDisabled\" = 0",
                    [caller.user],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(version) = version else {
                return Ok(Outcome::Forbidden);
            };
            let approved = tx.execute(
                "UPDATE \"DesktopBrowserLogins\" SET \"UserId\" = ?2, \"ApprovedSessionId\" = ?3, \"PrimarySource\" = ?4,
                        \"MfaCompletedAt\" = ?5, \"MfaEvidence\" = ?6, \"SecurityVersion\" = ?7, \"ApprovedAt\" = ?8,
                        \"ApprovalTokenHash\" = NULL
                 WHERE \"Id\" = ?1 AND \"ApprovedAt\" IS NULL AND \"RedeemedAt\" IS NULL AND \"ExpiresAt\" > ?8
                   AND \"ApprovalUserId\" = ?2 AND \"ApprovalSessionId\" = ?3 AND \"ApprovalTokenHash\" = ?9",
                params![
                    request,
                    caller.user,
                    source.id,
                    source.primary_source,
                    source.mfa_completed_at,
                    source.mfa_evidence,
                    version,
                    now,
                    stored
                ],
            )?;
            if approved != 1 {
                return Ok(Outcome::Gone);
            }
            let email = events::verified_email(tx, caller.user)?;
            events::record(
                tx,
                caller.user,
                events::DESKTOP_APPROVED,
                &origin.address,
                &origin.user_agent,
                "browser approval",
                email.as_deref(),
            )?;
            Ok(Outcome::Done)
        })
        .await?;
    Ok(no_store(answer(&state, outcome, request)?))
}
