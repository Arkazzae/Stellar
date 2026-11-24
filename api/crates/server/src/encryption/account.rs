//! The account's side: its key, the PIN-protected backup of that key and the
//! browsers that hold keys. Browsers do all the cryptography; the server keeps
//! public keys and opaque bytes, and counts PIN attempts.

use aws_lc_rs::constant_time::verify_slices_are_equal;
use aws_lc_rs::digest::{SHA256, digest};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use stellarr_messages::MessageKey;

use super::announce::{self, DEVICE_ADDED, KEYS_RESET, Line};
use super::problems::{self, Refusal};
use super::store::{self, Conversation};
use super::wire::{
    AccountDto, BackupParametersDto, DevicesDto, Members, Octets, Register, Replace, Reset, ResetDto, SetUp,
    Unlock, UnlockedDto,
};
use crate::auth::ceremony::{PasswordProof, check_given_password, judge_given_password};
use crate::auth::{Caller, events, fresh, password};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, Path, json_response};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// A 201 that says where the thing is.
fn created<T: Serialize>(location: &'static str, body: &T) -> Response {
    let mut response = json_response(StatusCode::CREATED, body);
    response
        .headers_mut()
        .insert(header::LOCATION, HeaderValue::from_static(location));
    response
}

/// The refusal of a key that does not fit its algorithm, or of a backup that is
/// not complete, before anything is read or stored.
fn refuse_unfit_key(
    state: &AppState,
    algorithm: Option<&str>,
    public_key: Option<&[u8]>,
) -> Result<(), ApiError> {
    if store::is_public_key(algorithm, public_key) {
        Ok(())
    } else {
        Err(problems::invalid(
            state,
            "publicKey",
            MessageKey::EncryptionKeyInvalid,
            &[],
        ))
    }
}

fn checked_backup(
    state: &AppState,
    backup: Option<super::wire::BackupRequest>,
) -> Result<super::wire::NewBackup, ApiError> {
    backup.and_then(super::wire::BackupRequest::checked).ok_or_else(|| {
        problems::invalid(state, "backup", MessageKey::EncryptionBackupInvalid, &[])
    })
}

/// Whether a request declares no body at all: a length of nothing, or neither
/// a length nor chunks.
fn says_nothing(headers: &HeaderMap) -> bool {
    match headers.get(header::CONTENT_LENGTH) {
        Some(length) => length == "0",
        None => !headers.contains_key(header::TRANSFER_ENCODING),
    }
}

/// Reads the body of a route that needs a second step passed lately, and asks
/// for it, in the order the first server did: a body that cannot be read, or
/// whose values are of the wrong type or encoding, is a 400 first; then the
/// second step; and only then a body that says nothing (empty or `null`), which
/// is a 400 too, and whatever the values mean.
async fn guarded<T>(
    state: &AppState,
    caller: Caller,
    headers: &HeaderMap,
    body: Result<Json<Option<Members>>, ApiError>,
    read: impl Fn(&Members) -> Result<T, super::wire::Malformed>,
) -> Result<T, ApiError> {
    let members = match body {
        Ok(Json(members)) => members,
        // An empty body has nothing in it to be unreadable.
        Err(_) if says_nothing(headers) => None,
        Err(refusal) => return Err(refusal),
    };
    let request = members.as_ref().map(read).transpose()?;
    let fresh = state
        .database()
        .read(move |db| fresh::has_fresh_second_step(db, caller.user, Some(caller.session), Timestamp::now()))
        .await?;
    if !fresh {
        return Err(problems::fresh_proof_needed(state));
    }
    request.ok_or_else(ApiError::bad_request)
}

pub async fn read(State(state): State<AppState>, caller: Caller) -> ApiResult<Response> {
    let account = state
        .database()
        .read(move |db| -> Result<AccountDto, DbError> {
            let now = Timestamp::now();
            let key = store::account_key(db, caller.user)?;
            let backup = store::backup(db, caller.user)?;
            let device = store::own_devices(db, caller.user, now)?
                .into_iter()
                .find(|device| device.session == caller.session);
            Ok(AccountDto {
                account_key: key.map(|key| key.dto()),
                backup: backup.map(|backup| backup.status()),
                device: device.map(|device| device.own_dto(Some(caller.session))),
            })
        })
        .await?;
    Ok(json_response(StatusCode::OK, &account))
}

/// First set-up: the account's public key and the backup of its private key,
/// together. An account has one key; a forgotten PIN is a reset.
pub async fn set_up(
    State(state): State<AppState>,
    caller: Caller,
    Json(body): Json<Members>,
) -> ApiResult<Response> {
    let request = SetUp::read(&body)?;
    refuse_unfit_key(&state, request.algorithm.as_deref(), request.public_key.as_deref())?;
    let backup = checked_backup(&state, request.backup)?;
    let (algorithm, public_key) = (request.algorithm.unwrap_or_default(), request.public_key.unwrap_or_default());

    let written = state
        .database()
        .write(move |tx| -> Result<Option<AccountDto>, DbError> {
            if store::account_key(tx, caller.user)?.is_some() {
                return Ok(None);
            }
            let now = Timestamp::now();
            let key_id = Id::random();
            tx.execute(
                "INSERT INTO \"AccountKeys\" (\"UserId\", \"KeyId\", \"Algorithm\", \"PublicKey\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![caller.user, key_id, algorithm, public_key, now],
            )?;
            store::put_backup(tx, caller.user, &backup, now)?;
            let key = store::account_key(tx, caller.user)?;
            let backup = store::backup(tx, caller.user)?;
            Ok(Some(AccountDto {
                account_key: key.map(|key| key.dto()),
                backup: backup.map(|backup| backup.status()),
                device: None,
            }))
        })
        .await?;
    let Some(account) = written else {
        return Err(problems::coded(
            &state,
            StatusCode::CONFLICT,
            problems::ACCOUNT_KEY_EXISTS,
            MessageKey::EncryptionAccountKeyExists,
        ));
    };
    announce::account_changed(&state, caller.user);
    Ok(created("/api/me/encryption", &account))
}

/// What a browser derives the PIN's keys with, before it tries one.
pub async fn backup_parameters(
    State(state): State<AppState>,
    caller: Caller,
) -> ApiResult<Response> {
    let found = state
        .database()
        .read(move |db| store::backup(db, caller.user))
        .await?;
    match found {
        None => Err(problems::backup_missing(&state)),
        Some(backup) => Ok(json_response(
            StatusCode::OK,
            &BackupParametersDto {
                remaining_attempts: backup.remaining_attempts(),
                salt: Octets(backup.salt),
                iterations: backup.iterations,
            },
        )),
    }
}

/// What one attempt at a PIN came to.
enum Attempt {
    NoBackup,
    Right(UnlockedDto),
    Wrong(i32),
}

/// One PIN attempt. A right PIN hands out the backup and clears the count of
/// wrong ones; the tenth wrong one in a row deletes the backup. The account key
/// stays, so browsers that hold it keep working and can store a backup under a
/// new PIN.
///
/// A PIN is short, so what is counted here is the whole protection of the
/// backup against somebody who holds a session: every attempt costs one of the
/// account's, whatever it holds and whatever it comes to, and the count of
/// wrong ones is written in the transaction that compares, so that attempts
/// sent at once are counted one after another. The handler runs to the end
/// when the client leaves, so a refusal is never undone by hanging up.
pub async fn unlock(
    State(state): State<AppState>,
    caller: Caller,
    Json(body): Json<Members>,
) -> ApiResult<Response> {
    let request = Unlock::read(&body)?;
    if !state.pin_attempts().try_acquire(caller.user) {
        return Err(ApiError::status(StatusCode::TOO_MANY_REQUESTS));
    }
    let Some(auth_key) = request.auth_key.filter(|key| key.len() == 32) else {
        return Err(problems::invalid(
            &state,
            "authKey",
            MessageKey::EncryptionAuthKeyInvalid,
            &[],
        ));
    };
    let proof = digest(&SHA256, &auth_key);

    let attempt = state
        .database()
        .write(move |tx| -> Result<Attempt, DbError> {
            let Some(backup) = store::backup(tx, caller.user)? else {
                return Ok(Attempt::NoBackup);
            };
            if verify_slices_are_equal(proof.as_ref(), &backup.verifier).is_ok() {
                let key = store::account_key(tx, caller.user)?.ok_or_else(|| {
                    DbError::Task("an account with a backup has no key".to_string())
                })?;
                tx.execute(
                    "UPDATE \"AccountKeyBackups\" SET \"FailedAttempts\" = 0 WHERE \"UserId\" = ?1",
                    [caller.user],
                )?;
                return Ok(Attempt::Right(UnlockedDto {
                    account_key_id: key.key_id,
                    iv: Octets(backup.iv),
                    ciphertext: Octets(backup.ciphertext),
                }));
            }
            tx.execute(
                "UPDATE \"AccountKeyBackups\" SET \"FailedAttempts\" = \"FailedAttempts\" + 1 WHERE \"UserId\" = ?1",
                [caller.user],
            )?;
            let failed: i32 = tx.query_row(
                "SELECT \"FailedAttempts\" FROM \"AccountKeyBackups\" WHERE \"UserId\" = ?1",
                [caller.user],
                |row| row.get(0),
            )?;
            let remaining = (store::MAX_FAILED_ATTEMPTS - failed).max(0);
            if remaining == 0 {
                tx.execute(
                    "DELETE FROM \"AccountKeyBackups\" WHERE \"UserId\" = ?1",
                    [caller.user],
                )?;
            }
            Ok(Attempt::Wrong(remaining))
        })
        .await?;
    match attempt {
        Attempt::NoBackup => Err(problems::backup_missing(&state)),
        Attempt::Right(unlocked) => Ok(json_response(StatusCode::OK, &unlocked)),
        Attempt::Wrong(0) => Err(Refusal::new(
            &state,
            StatusCode::GONE,
            MessageKey::EncryptionBackupDeleted,
        )
        .with("remainingAttempts", 0)
        .code(problems::BACKUP_DELETED)),
        Attempt::Wrong(remaining) => Err(Refusal::new(
            &state,
            StatusCode::FORBIDDEN,
            MessageKey::EncryptionPinIncorrect,
        )
        .with("remainingAttempts", remaining)
        .code(problems::PIN_INCORRECT)),
    }
}

/// What a new PIN came to.
enum Replaced {
    WrongPassword,
    FreshProofNeeded,
    KeyMissing,
    KeyChanged(Id),
    DeviceMissing,
    Done(super::wire::BackupStatusDto),
}

/// A new PIN: a browser that holds the account key encrypts it again. It
/// needs the account password (or, for an account that has none, a recent
/// sign-in) and, where the account has a second step, a fresh one, like a
/// password change; it does not need the old PIN, so it also restores a backup
/// that wrong PINs deleted.
pub async fn replace_backup(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    headers: HeaderMap,
    body: Result<Json<Option<Members>>, ApiError>,
) -> ApiResult<Response> {
    let request = guarded(&state, caller, &headers, body, Replace::read).await?;
    let backup = checked_backup(&state, request.backup)?;
    let given =
        check_given_password(&state, caller.user, request.current_password.as_deref()).await?;
    let claimed = request.account_key_id;

    let outcome = state
        .database()
        .write(move |tx| -> Result<Replaced, DbError> {
            let now = Timestamp::now();
            match judge_given_password(tx, caller, &given, now)? {
                PasswordProof::Given => {}
                PasswordProof::WrongPassword => {
                    events::record_wrong_password(tx, caller.user, &origin.address, &origin.user_agent)?;
                    return Ok(Replaced::WrongPassword);
                }
                PasswordProof::FreshProofNeeded => return Ok(Replaced::FreshProofNeeded),
            }
            let Some(key) = store::account_key(tx, caller.user)? else {
                return Ok(Replaced::KeyMissing);
            };
            // The server cannot see what the backup holds; it can at least
            // refuse a browser that encrypted a key the account no longer has,
            // or that never registered a device and so cannot have unlocked or
            // made the key.
            if claimed.as_deref() != Some(key.key_id.to_string().as_str()) {
                return Ok(Replaced::KeyChanged(key.key_id));
            }
            if store::current_device(tx, caller.session, now)?.is_none() {
                return Ok(Replaced::DeviceMissing);
            }
            store::put_backup(tx, caller.user, &backup, now)?;
            let stored = store::backup(tx, caller.user)?
                .ok_or_else(|| DbError::Task("the backup just written is not there".to_string()))?;
            Ok(Replaced::Done(stored.status()))
        })
        .await?;
    match outcome {
        Replaced::WrongPassword => {
            password::wrong_password_wait().await;
            Err(problems::password_incorrect(&state))
        }
        Replaced::FreshProofNeeded => Err(problems::fresh_proof_needed(&state)),
        Replaced::KeyMissing => Err(problems::account_key_missing(&state)),
        Replaced::KeyChanged(key_id) => Err(Refusal::new(
            &state,
            StatusCode::CONFLICT,
            MessageKey::EncryptionAccountKeyChanged,
        )
        .with("accountKeyId", key_id.to_string())
        .code(problems::ACCOUNT_KEY_CHANGED)),
        Replaced::DeviceMissing => Err(problems::coded(
            &state,
            StatusCode::CONFLICT,
            problems::DEVICE_MISSING,
            MessageKey::EncryptionDeviceMissing,
        )),
        Replaced::Done(status) => {
            announce::account_changed(&state, caller.user);
            Ok(json_response(StatusCode::OK, &status))
        }
    }
}

/// Adds a line about the person's keys to each of their encrypted
/// conversations.
fn add_lines(
    tx: &rusqlite::Transaction,
    user: Id,
    kind: &'static str,
    now: Timestamp,
) -> Result<Vec<(Conversation, Option<Line>)>, DbError> {
    store::encrypted_conversations_of(tx, user)?
        .into_iter()
        .map(|conversation| {
            let line = announce::add_line(tx, conversation.id, user, kind, now)?;
            Ok((conversation, Some(line)))
        })
        .collect()
}

/// What a reset came to.
enum Resetting {
    WrongPassword,
    FreshProofNeeded,
    KeyMissing,
    Done {
        answer: ResetDto,
        changes: Vec<(Conversation, Option<Line>)>,
    },
}

/// For a forgotten PIN that no browser can stand in for: a new account key and
/// its backup replace the old ones. Everything encrypted for the old key is
/// lost. Every other device loses its keys, and the people in the account's
/// encrypted conversations see that the keys changed.
pub async fn reset(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    headers: HeaderMap,
    body: Result<Json<Option<Members>>, ApiError>,
) -> ApiResult<Response> {
    let request = guarded(&state, caller, &headers, body, Reset::read).await?;
    refuse_unfit_key(&state, request.algorithm.as_deref(), request.public_key.as_deref())?;
    let backup = checked_backup(&state, request.backup)?;
    let given =
        check_given_password(&state, caller.user, request.current_password.as_deref()).await?;
    let (algorithm, public_key) = (
        request.algorithm.unwrap_or_default(),
        request.public_key.unwrap_or_default(),
    );

    let outcome = state
        .database()
        .write(move |tx| -> Result<Resetting, DbError> {
            let now = Timestamp::now();
            match judge_given_password(tx, caller, &given, now)? {
                PasswordProof::Given => {}
                PasswordProof::WrongPassword => {
                    events::record_wrong_password(tx, caller.user, &origin.address, &origin.user_agent)?;
                    return Ok(Resetting::WrongPassword);
                }
                PasswordProof::FreshProofNeeded => return Ok(Resetting::FreshProofNeeded),
            }
            if store::account_key(tx, caller.user)?.is_none() {
                return Ok(Resetting::KeyMissing);
            }
            tx.execute(
                "UPDATE \"AccountKeys\" SET \"KeyId\" = ?2, \"Algorithm\" = ?3, \"PublicKey\" = ?4, \"CreatedAt\" = ?5
                 WHERE \"UserId\" = ?1",
                rusqlite::params![caller.user, Id::random(), algorithm, public_key, now],
            )?;
            store::put_backup(tx, caller.user, &backup, now)?;
            // Every device of the account that is not this session's, whether
            // or not its session still signs in.
            let revoked = tx.execute(
                "UPDATE \"EncryptionDevices\" SET \"RevokedAt\" = ?3
                 WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"SessionId\" <> ?2",
                rusqlite::params![caller.user, caller.session, now],
            )?;
            let changes = add_lines(tx, caller.user, KEYS_RESET, now)?;
            let key = store::account_key(tx, caller.user)?.ok_or_else(|| {
                DbError::Task("the account key just written is not there".to_string())
            })?;
            let stored = store::backup(tx, caller.user)?
                .ok_or_else(|| DbError::Task("the backup just written is not there".to_string()))?;
            Ok(Resetting::Done {
                answer: ResetDto {
                    account_key: key.dto(),
                    backup: stored.status(),
                    revoked_devices: i64::try_from(revoked).unwrap_or(i64::MAX),
                },
                changes,
            })
        })
        .await?;
    match outcome {
        Resetting::WrongPassword => {
            password::wrong_password_wait().await;
            Err(problems::password_incorrect(&state))
        }
        Resetting::FreshProofNeeded => Err(problems::fresh_proof_needed(&state)),
        Resetting::KeyMissing => Err(problems::account_key_missing(&state)),
        Resetting::Done { answer, changes } => {
            let told = announce::keys_changed(&state, changes, caller.user).await?;
            announce::account_changed(&state, caller.user);
            let mut people = vec![caller.user];
            people.extend(told);
            crate::social::announce(&state, &people);
            Ok(json_response(StatusCode::OK, &answer))
        }
    }
}

pub async fn list_devices(State(state): State<AppState>, caller: Caller) -> ApiResult<Response> {
    let devices = state
        .database()
        .read(move |db| store::own_devices(db, caller.user, Timestamp::now()))
        .await?;
    Ok(json_response(
        StatusCode::OK,
        &DevicesDto {
            devices: devices
                .iter()
                .map(|device| device.own_dto(Some(caller.session)))
                .collect(),
        },
    ))
}

/// What registering a device came to.
enum Registered {
    KeyMissing,
    Limit,
    Again(super::wire::OwnDeviceDto),
    New {
        device: super::wire::OwnDeviceDto,
        changes: Vec<(Conversation, Option<Line>)>,
    },
}

/// Registers the key of the browser asking, under its login session. It needs
/// no PIN: a device without one still takes part in new messages. The same key
/// again is a no-op, so a browser may register on every start; a different key
/// replaces the session's device.
pub async fn register_device(
    State(state): State<AppState>,
    caller: Caller,
    Json(body): Json<Members>,
) -> ApiResult<Response> {
    let request = Register::read(&body)?;
    refuse_unfit_key(&state, request.algorithm.as_deref(), request.public_key.as_deref())?;
    let label = request.label.as_deref().map_or("", str::trim).to_string();
    if !(1..=100).contains(&label.encode_utf16().count()) {
        return Err(problems::invalid(
            &state,
            "label",
            MessageKey::EncryptionDeviceLabelInvalid,
            &[("max", &100)],
        ));
    }
    let (algorithm, public_key) = (
        request.algorithm.unwrap_or_default(),
        request.public_key.unwrap_or_default(),
    );

    let outcome = state
        .database()
        .write(move |tx| -> Result<Registered, DbError> {
            let now = Timestamp::now();
            if store::account_key(tx, caller.user)?.is_none() {
                return Ok(Registered::KeyMissing);
            }
            let existing = store::session_device(tx, caller.session)?;
            if let Some(mut existing) = existing.clone()
                && existing.algorithm == algorithm
                && existing.public_key == public_key
            {
                tx.execute(
                    "UPDATE \"EncryptionDevices\" SET \"Label\" = ?2, \"LastSeenAt\" = ?3 WHERE \"Id\" = ?1",
                    rusqlite::params![existing.id, label, now],
                )?;
                existing.label = label;
                existing.last_seen_at = now;
                existing.session_last_seen_at = now;
                return Ok(Registered::Again(existing.own_dto(Some(caller.session))));
            }
            if existing.is_none()
                && store::count_active(tx, caller.user, now)? >= store::MAX_ACTIVE_DEVICES
            {
                return Ok(Registered::Limit);
            }
            if let Some(existing) = &existing {
                tx.execute(
                    "UPDATE \"EncryptionDevices\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1",
                    rusqlite::params![existing.id, now],
                )?;
            }
            let id = Id::random();
            tx.execute(
                "INSERT INTO \"EncryptionDevices\" (\"Id\", \"UserId\", \"SessionId\", \"Algorithm\", \"PublicKey\", \"Label\",
                                                   \"CreatedAt\", \"LastSeenAt\", \"RevokedAt\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, NULL)",
                rusqlite::params![id, caller.user, caller.session, algorithm, public_key, label, now],
            )?;
            let changes = add_lines(tx, caller.user, DEVICE_ADDED, now)?;
            let device = store::Device {
                id,
                user: caller.user,
                session: caller.session,
                algorithm,
                public_key,
                label,
                created_at: now,
                last_seen_at: now,
                session_last_seen_at: now,
            };
            Ok(Registered::New {
                device: device.own_dto(Some(caller.session)),
                changes,
            })
        })
        .await?;
    match outcome {
        Registered::KeyMissing => Err(problems::account_key_missing(&state)),
        Registered::Limit => Err(Refusal::new(
            &state,
            StatusCode::CONFLICT,
            MessageKey::EncryptionDeviceLimit,
        )
        .with("max", store::MAX_ACTIVE_DEVICES)
        .code(problems::DEVICE_LIMIT)),
        Registered::Again(device) => Ok(json_response(StatusCode::OK, &device)),
        Registered::New { device, changes } => {
            let told = announce::keys_changed(&state, changes, caller.user).await?;
            announce::account_changed(&state, caller.user);
            if !told.is_empty() {
                let mut people = vec![caller.user];
                people.extend(told);
                crate::social::announce(&state, &people);
            }
            Ok(created("/api/me/encryption/devices", &device))
        }
    }
}

/// What removing a device came to.
enum Revoked {
    Unknown,
    Current,
    Done(Vec<(Conversation, Option<Line>)>, Id),
}

/// Removes the keys of one of the account's other browsers and signs that
/// browser out. The browser asking removes its own keys by signing out.
pub async fn revoke_device(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    Path(device_id): Path<Id>,
) -> ApiResult<Response> {
    let outcome = state
        .database()
        .write(move |tx| -> Result<Revoked, DbError> {
            let now = Timestamp::now();
            let Some(device) = store::active_device(tx, caller.user, device_id, now)? else {
                return Ok(Revoked::Unknown);
            };
            if device.session == caller.session {
                return Ok(Revoked::Current);
            }
            tx.execute(
                "UPDATE \"EncryptionDevices\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1",
                rusqlite::params![device_id, now],
            )?;
            if crate::auth::session::revoke(tx, device.session, now)? {
                events::record(
                    tx,
                    caller.user,
                    events::SESSION_REVOKED,
                    &origin.address,
                    &origin.user_agent,
                    "encryption keys removed",
                    None,
                )?;
            }
            let changes = store::encrypted_conversations_of(tx, caller.user)?
                .into_iter()
                .map(|conversation| (conversation, None))
                .collect();
            Ok(Revoked::Done(changes, device.session))
        })
        .await?;
    match outcome {
        Revoked::Unknown => Err(ApiError::not_found()),
        Revoked::Current => Err(problems::coded(
            &state,
            StatusCode::CONFLICT,
            problems::CURRENT_DEVICE,
            MessageKey::EncryptionDeviceIsCurrent,
        )),
        Revoked::Done(changes, session) => {
            state.events().session_ended(caller.user, session);
            announce::keys_changed(&state, changes, caller.user).await?;
            announce::account_changed(&state, caller.user);
            Ok(StatusCode::NO_CONTENT.into_response())
        }
    }
}
