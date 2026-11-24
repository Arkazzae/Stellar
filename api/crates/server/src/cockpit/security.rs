//! An account's security as the owner of the instance sees it: the
//! second-step methods, the recovery codes left, the remembered devices, the
//! links to identity providers and the recent security history, never a
//! secret. The owner can recover an account that lost its second step, as the
//! command line does, and remove a link to a provider.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use super::providers;
use super::{invalid, no_content, bind};
use crate::auth::Caller;
use crate::auth::policy::SecurityPolicy;
use crate::auth::{events, recovery, recovery_codes, ways_in};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Origin, Path, Problem, json_response};
use crate::panel::{self, FreshInstanceOwner};
use crate::permissions::membership::InstanceOwner;
use crate::state::AppState;
use crate::values::{Guid, Id, Timestamp};

/// How many security events the account's page shows.
const HISTORY_SHOWN: usize = 10;

/// An account that is a member of the instance server, the only ones the
/// panel manages.
fn member_exists(db: &Connection, server: Id, user: Id) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2)",
        params![server, user],
        |row| row.get(0),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Method {
    id: Id,
    #[serde(rename = "type")]
    kind: String,
    name: String,
    confirmed: bool,
    created_at: Timestamp,
    last_used_at: Option<Timestamp>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    id: Id,
    name: String,
    issuer: String,
    created_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    kind: String,
    ip_address: String,
    created_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecurityDto {
    user_id: Id,
    recovery_required: bool,
    has_password: bool,
    methods: Vec<Method>,
    remaining_recovery_codes: i64,
    trusted_devices: i64,
    identities: Vec<Identity>,
    events: Vec<Event>,
}

pub(super) async fn read(
    State(state): State<AppState>,
    Path(Guid(user)): Path<Guid>,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Response> {
    let reading = state.clone();
    let answer = state
        .database()
        .read(move |db| -> Result<Option<SecurityDto>, DbError> {
            if !member_exists(db, server, user)? {
                return Ok(None);
            }
            let (recovery_required, has_password): (bool, bool) = db.query_row(
                "SELECT \"SecurityRecoveryRequired\", \"PasswordHash\" IS NOT NULL FROM \"Users\" WHERE \"Id\" = ?1",
                [user],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let mut statement = db.prepare(
                "SELECT \"Id\", \"Type\", \"Name\", \"ConfirmedAt\" IS NOT NULL, \"CreatedAt\", \"LastUsedAt\" FROM \"MfaMethods\"
                 WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL ORDER BY \"CreatedAt\"",
            )?;
            let methods = statement
                .query_map([user], |row| {
                    Ok(Method {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        name: row.get(2)?,
                        confirmed: row.get(3)?,
                        created_at: row.get(4)?,
                        last_used_at: row.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut statement = db.prepare(
                "SELECT \"Id\", \"ProviderId\", \"Issuer\", \"CreatedAt\" FROM \"ExternalIdentities\" WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\"",
            )?;
            let identities = statement
                .query_map([user], |row| {
                    let provider: Option<String> = row.get(1)?;
                    let issuer: String = row.get(2)?;
                    Ok(Identity {
                        id: row.get(0)?,
                        name: crate::identity_providers::identity_name(
                            &reading,
                            provider.as_deref().unwrap_or("default"),
                            &issuer,
                        )
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                        issuer,
                        created_at: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut statement = db.prepare(
                "SELECT \"Kind\", \"IpAddress\", \"CreatedAt\" FROM \"SecurityEvents\" WHERE \"UserId\" = ?1
                 ORDER BY \"CreatedAt\" DESC LIMIT ?2",
            )?;
            let events = statement
                .query_map(params![user, HISTORY_SHOWN as i64], |row| {
                    Ok(Event {
                        kind: row.get(0)?,
                        ip_address: row.get(1)?,
                        created_at: row.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let now = Timestamp::now();
            Ok(Some(SecurityDto {
                user_id: user,
                recovery_required,
                has_password,
                methods,
                remaining_recovery_codes: recovery_codes::remaining(db, user)?,
                trusted_devices: db.query_row(
                    "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?2",
                    params![user, now],
                    |row| row.get(0),
                )?,
                identities,
                events,
            }))
        })
        .await?;
    match answer {
        Some(answer) => Ok(json_response(StatusCode::OK, &answer)),
        None => Err(ApiError::not_found()),
    }
}

#[derive(Deserialize)]
struct RecoverRequest {
    reason: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Recovered {
    credential: String,
    expires_at: Timestamp,
    methods_revoked: usize,
    trusted_devices_revoked: usize,
    sessions_revoked: usize,
}

/// What an operator's recovery does, and the credential to hand the person.
pub(super) async fn recover(
    State(state): State<AppState>,
    Path(Guid(user)): Path<Guid>,
    request: Request,
) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, bind::<RecoverRequest>).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    let messages = *state.messages();
    if user == caller.user {
        // Recovery ends every session of the account; from here that would
        // lock the owner out of the very panel that issued the credential.
        return Err(Problem::new(StatusCode::CONFLICT)
            .title(messages.get(MessageKey::CockpitCannotRecoverSelf, &[])?)
            .into());
    }
    let reason = body.reason.as_deref().unwrap_or("").trim().to_string();
    if !(3..=500).contains(&super::length(&reason)) {
        return Err(invalid(
            "reason",
            messages.get(MessageKey::CockpitRecoveryReasonInvalid, &[])?,
        ));
    }
    let done = state
        .database()
        .write(move |tx| -> Result<Option<recovery::Recovered>, ApiError> {
            if !member_exists(tx, server, user)? {
                return Err(ApiError::not_found());
            }
            let policy = SecurityPolicy::load(tx)?;
            let name: String = tx.query_row(
                "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                [user],
                |row| row.get(0),
            )?;
            let sentence = messages
                .get(MessageKey::AuditAccountRecovered, &[("name", &name)])
                .map_err(|e| DbError::Task(e.to_string()))?;
            Ok(recovery::recover(
                tx,
                user,
                &reason,
                &policy,
                Timestamp::now(),
                |tx| Ok(panel::record(tx, server, caller.user, sentence)?),
            )?)
        })
        .await?;
    let Some(done) = done else {
        return Err(Problem::new(StatusCode::CONFLICT)
            .title(messages.get(MessageKey::CockpitRecoverDisabled, &[])?)
            .into());
    };
    Ok(json_response(
        StatusCode::OK,
        &Recovered {
            credential: done.credential.clone(),
            expires_at: done.expires_at,
            methods_revoked: done.methods_revoked,
            trusted_devices_revoked: done.trusted_devices_revoked,
            sessions_revoked: done.sessions_revoked,
        },
    ))
}

/// Removes a link to a provider, unless it is the only way the account signs
/// in: the owner cannot give it a password, so the person adds one first.
pub(super) async fn unlink(
    State(state): State<AppState>,
    Path((Guid(user), Guid(identity))): Path<(Guid, Guid)>,
    origin: Origin,
    caller: Caller,
    FreshInstanceOwner(server): FreshInstanceOwner,
) -> ApiResult<Response> {
    let registry = providers::registry(&state)?;
    let owned = registry.owned();
    let messages = *state.messages();
    // The question whether a way in remains and the removal are one
    // transaction, so that two removals at once cannot each count the
    // other's way in as left.
    state
        .database()
        .write(move |tx| -> Result<(), ApiError> {
            if !member_exists(tx, server, user)? {
                return Err(ApiError::not_found());
            }
            let found: Option<String> = tx
                .query_row(
                    "SELECT \"Issuer\" FROM \"ExternalIdentities\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2",
                    params![identity, user],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(issuer) = found else {
                return Err(ApiError::not_found());
            };
            let name: String = tx.query_row(
                "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                [user],
                |row| row.get(0),
            )?;
            if !ways_in::remain(tx, &owned.view(), user, Some(identity), None)? {
                return Err(ways_in::refusal(
                    messages.get(MessageKey::CockpitLastWayIn, &[("name", &name)])?,
                )
                .into());
            }
            tx.execute("DELETE FROM \"ExternalIdentities\" WHERE \"Id\" = ?1", [identity])?;
            let verified = events::verified_email(tx, user)?;
            events::record(
                tx,
                user,
                events::IDENTITY_UNLINKED,
                &origin.address,
                &origin.user_agent,
                &format!("operator: {issuer}"),
                verified.as_deref(),
            )?;
            panel::record(
                tx,
                server,
                caller.user,
                messages.get(MessageKey::AuditIdentityUnlinked, &[("name", &name)])?,
            )?;
            Ok(())
        })
        .await?;
    Ok(no_content())
}
