//! The account's side: its key, the recovery backup for that key and the
//! browsers that hold keys. The server seals the browser-encrypted backup at rest
//! and counts recovery attempts.

use aws_lc_rs::constant_time::verify_slices_are_equal;
use aws_lc_rs::digest::{SHA256, digest};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use stellarr_messages::MessageKey;

use super::announce::{self, DEVICE_ADDED, KEYS_RESET, Line};
use super::key_backups;
use super::problems::{self, Refusal};
use super::store::{self, Conversation};
use super::wire::{
    AccountDto, BackupParametersDto, DevicesDto, Octets, Register, Replace, Reset, ResetDto, SetUp,
    Unlock, UnlockedDto,
};
use crate::auth::ceremony::{PasswordProof, check_given_password, judge_given_password};
use crate::auth::{Caller, events, fresh, password};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, Path, created, json_response};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

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
/// for it. A body that cannot be read, or
/// whose values are of the wrong type or encoding, is a 400 first; then the
/// second step; and only then a body that says nothing (empty or `null`), which
/// is a 400 too, and whatever the values mean.
async fn guarded<T>(
    state: &AppState,
    caller: Caller,
    headers: &HeaderMap,
    body: Result<Json<Option<T>>, ApiError>,
) -> Result<T, ApiError> {
    let request = match body {
        Ok(Json(request)) => request,
        // An empty body has nothing in it to be unreadable.
        Err(_) if says_nothing(headers) => None,
        Err(refusal) => return Err(refusal),
    };
    let fresh = state
        .database()
        .read(move |db| fresh::has_fresh_second_step(db, caller.user, Some(caller.session), Timestamp::now()))
        .await?;
    if !fresh {
        return Err(fresh::refusal(state.messages())?.into());
    }
    request.ok_or_else(ApiError::bad_request)
}

pub async fn read(State(state): State<AppState>, caller: Caller) -> ApiResult<Response> {
    let sealer = state.key_backup_sealer().clone();
    let account = state
        .database()
        .read(move |db| -> Result<AccountDto, DbError> {
            let now = Timestamp::now();
            let key = store::account_key(db, caller.user)?;
            let backup = key_backups::get(db, &sealer, caller.user)?;
            let device = store::own_devices(db, caller.user, now)?
                .into_iter()
                .find(|device| device.session == caller.session);
            Ok(AccountDto {
                account_key: key.map(|key| key.dto()),
                backup: backup.map(|backup| backup.status()),
                device: device.map(|device| device.own_dto(Some(caller.session))),
                recovery_version: if key_backups::uses_code(db, caller.user)? { 2 } else { 1 },
            })
        })
        .await?;
    Ok(json_response(StatusCode::OK, &account))
}

/// First set-up: the account's public key and the backup of its private key,
/// together. A browser that still holds the private key can replace its recovery code.
pub async fn set_up(
    State(state): State<AppState>,
    caller: Caller,
    Json(request): Json<SetUp>,
) -> ApiResult<Response> {
    refuse_unfit_key(&state, request.algorithm.as_deref(), request.public_key.as_deref())?;
    let backup = checked_backup(&state, request.backup)?;
    let (algorithm, public_key) = (request.algorithm.unwrap_or_default(), request.public_key.unwrap_or_default());

    let sealer = state.key_backup_sealer().clone();
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
            key_backups::put(tx, &sealer, caller.user, &backup, 0, now)?;
            let key = store::account_key(tx, caller.user)?;
            let backup = key_backups::get(tx, &sealer, caller.user)?;
            Ok(Some(AccountDto {
                account_key: key.map(|key| key.dto()),
                backup: backup.map(|backup| backup.status()),
                device: None,
                recovery_version: 2,
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

/// Public parameters for the client's local derivation.
pub async fn backup_parameters(
    State(state): State<AppState>,
    caller: Caller,
) -> ApiResult<Response> {
    let sealer = state.key_backup_sealer().clone();
    let found = state
        .database()
        .read(move |db| key_backups::get(db, &sealer, caller.user))
        .await?;
    match found {
        None => Err(problems::backup_missing(&state)),
        Some(backup) => Ok(json_response(
            StatusCode::OK,
            &BackupParametersDto {
                version: backup.payload.version,
                remaining_attempts: backup.remaining_attempts(),
                salt: Octets(backup.payload.salt),
                iterations: backup.payload.iterations,
            },
        )),
    }
}

/// Outcome of an attempt to unlock the backup.
enum Attempt {
    NoBackup,
    Right(UnlockedDto),
    Wrong(i32),
}

/// Verifies a proof for the active backup version. Ten consecutive incorrect
/// proofs delete that backup. Updating the counter in the same transaction as
/// verification serializes concurrent attempts, even if a client disconnects.
pub async fn unlock(
    State(state): State<AppState>,
    caller: Caller,
    Json(request): Json<Unlock>,
) -> ApiResult<Response> {
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

    let sealer = state.key_backup_sealer().clone();
    let attempt = state
        .database()
        .write(move |tx| -> Result<Attempt, DbError> {
            let Some(backup) = key_backups::get(tx, &sealer, caller.user)? else {
                return Ok(Attempt::NoBackup);
            };
            if request.version != Some(backup.payload.version) || (backup.payload.version == 1 && key_backups::uses_code(tx, caller.user)?) {
                return Ok(Attempt::NoBackup);
            }
            if verify_slices_are_equal(proof.as_ref(), &backup.payload.verifier).is_ok() {
                let key = store::account_key(tx, caller.user)?.ok_or_else(|| {
                    DbError::Task("an account with a backup has no key".to_string())
                })?;
                tx.execute(
                    "UPDATE \"ProtectedAccountKeyBackups\" SET \"FailedAttempts\" = 0 WHERE \"UserId\" = ?1",
                    [caller.user],
                )?;
                return Ok(Attempt::Right(UnlockedDto {
                    account_key_id: key.key_id,
                    iv: Octets(backup.payload.iv),
                    ciphertext: Octets(backup.payload.ciphertext),
                }));
            }
            tx.execute(
                "UPDATE \"ProtectedAccountKeyBackups\" SET \"FailedAttempts\" = \"FailedAttempts\" + 1 WHERE \"UserId\" = ?1",
                [caller.user],
            )?;
            let failed: i32 = tx.query_row(
                "SELECT \"FailedAttempts\" FROM \"ProtectedAccountKeyBackups\" WHERE \"UserId\" = ?1",
                [caller.user],
                |row| row.get(0),
            )?;
            let remaining = (store::MAX_FAILED_ATTEMPTS - failed).max(0);
            if remaining == 0 {
                tx.execute(
                    "DELETE FROM \"ProtectedAccountKeyBackups\" WHERE \"UserId\" = ?1",
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

/// Outcome of replacing an existing recovery-code backup.
enum Replaced {
    WrongPassword,
    FreshProofNeeded,
    KeyMissing,
    KeyChanged(Id),
    DeviceMissing,
    Done(super::wire::BackupStatusDto),
}

/// Reissues a recovery-code backup from an unlocked device after fresh
/// authentication. This also replaces a missing backup. Legacy accounts must
/// rotate their identity through the migration route.
pub async fn replace_backup(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    headers: HeaderMap,
    body: Result<Json<Option<Replace>>, ApiError>,
) -> ApiResult<Response> {
    let request = guarded(&state, caller, &headers, body).await?;
    let backup = checked_backup(&state, request.backup)?;
    let given =
        check_given_password(&state, caller.user, request.current_password.as_deref()).await?;
    let claimed = request.account_key_id;

    let sealer = state.key_backup_sealer().clone();
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
            if claimed.as_deref() != Some(key.key_id.to_string().as_str()) || !key_backups::uses_code(tx, caller.user)? {
                return Ok(Replaced::KeyChanged(key.key_id));
            }
            if store::current_device(tx, caller.session, now)?.is_none() {
                return Ok(Replaced::DeviceMissing);
            }
            key_backups::put(tx, &sealer, caller.user, &backup, 0, now)?;
            let stored = key_backups::get(tx, &sealer, caller.user)?
                .ok_or_else(|| DbError::Task("the backup just written is not there".to_string()))?;
            Ok(Replaced::Done(stored.status()))
        })
        .await?;
    match outcome {
        Replaced::WrongPassword => {
            password::wrong_password_wait().await;
            Err(problems::password_incorrect(&state))
        }
        Replaced::FreshProofNeeded => Err(fresh::refusal(state.messages())?.into()),
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
    Replayed(ResetDto),
    Stale,
    WrongPassword,
    FreshProofNeeded,
    KeyMissing,
    Done {
        answer: ResetDto,
        changes: Vec<(Conversation, Option<Line>)>,
    },
}

/// Rotates the account identity and revokes other device registrations.
/// Migration retains the previous key ID for history. A destructive reset
/// drops access to the previous key envelopes.
pub async fn reset(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    headers: HeaderMap,
    body: Result<Json<Option<Reset>>, ApiError>,
) -> ApiResult<Response> {
    let request = guarded(&state, caller, &headers, body).await?;
    refuse_unfit_key(&state, request.algorithm.as_deref(), request.public_key.as_deref())?;
    let backup = checked_backup(&state, request.backup)?;
    let given =
        check_given_password(&state, caller.user, request.current_password.as_deref()).await?;
    let (algorithm, public_key) = (
        request.algorithm.unwrap_or_default(),
        request.public_key.unwrap_or_default(),
    );

    let sealer = state.key_backup_sealer().clone();
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
            let Some(previous) = store::account_key(tx, caller.user)? else {
                return Ok(Resetting::KeyMissing);
            };
            let archived = request.migrate_from_key_id;
            if previous.public_key == public_key {
                if let Some(saved) = key_backups::get(tx, &sealer, caller.user)?
                    && saved.payload == backup
                    && key_backups::archived_key(tx, caller.user)? == archived
                {
                    return Ok(Resetting::Replayed(ResetDto {
                        account_key: previous.dto(), backup: saved.status(), revoked_devices: 0,
                    }));
                }
                return Ok(Resetting::Stale);
            }
            if request.expected_account_key_id.is_some_and(|id| id != previous.key_id) {
                return Ok(Resetting::Stale);
            }
            let legacy = !key_backups::uses_code(tx, caller.user)?;
            if archived.is_some_and(|expected| expected != previous.key_id || !legacy) {
                return Ok(Resetting::Stale);
            }
            if legacy {
                tx.execute("INSERT OR IGNORE INTO RetiredPinAccounts (UserId) VALUES (?1)", [caller.user])?;
            }
            tx.pragma_update(None, "secure_delete", true)?;
            tx.execute("DELETE FROM ProtectedAccountKeyBackups WHERE UserId = ?1", [caller.user])?;
            tx.execute(
                "INSERT INTO CodeRecoveryAccounts (UserId, ArchivedKeyId) VALUES (?1, ?2)
                 ON CONFLICT(UserId) DO UPDATE SET ArchivedKeyId = excluded.ArchivedKeyId",
                rusqlite::params![caller.user, archived],
            )?;
            tx.execute(
                "UPDATE \"AccountKeys\" SET \"KeyId\" = ?2, \"Algorithm\" = ?3, \"PublicKey\" = ?4, \"CreatedAt\" = ?5
                 WHERE \"UserId\" = ?1",
                rusqlite::params![caller.user, Id::random(), algorithm, public_key, now],
            )?;
            key_backups::put(tx, &sealer, caller.user, &backup, 0, now)?;
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
            let stored = key_backups::get(tx, &sealer, caller.user)?
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
        Resetting::FreshProofNeeded => Err(fresh::refusal(state.messages())?.into()),
        Resetting::Replayed(answer) => Ok(json_response(StatusCode::OK, &answer)),
        Resetting::Stale => Err(problems::coded(&state, StatusCode::CONFLICT,
            problems::ACCOUNT_KEY_CHANGED, MessageKey::EncryptionAccountKeyChanged)),
        Resetting::KeyMissing => Err(problems::account_key_missing(&state)),
        Resetting::Done { answer, changes } => {
            if let Err(error) = state.database().checkpoint_key_backups().await {
                tracing::warn!(%error, "Could not checkpoint the superseded key backup");
            }
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

/// Registers a browser key under its login session. Registering the same
/// key is idempotent. Account-bound encryption separately requires the
/// account private key.
pub async fn register_device(
    State(state): State<AppState>,
    caller: Caller,
    Json(request): Json<Register>,
) -> ApiResult<Response> {
    refuse_unfit_key(&state, request.algorithm.as_deref(), request.public_key.as_deref())?;
    let label = request.label.as_deref().map_or("", str::trim).to_string();
    if !(1..=100).contains(&stellarr_unicode::utf16::length(&label)) {
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
