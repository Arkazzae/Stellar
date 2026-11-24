//! End-to-end encryption of one private conversation: the switch both people
//! have to agree on, the keys of both of them, and the epochs that new
//! messages are encrypted with. Like the messages, it is as good as absent to
//! anybody else, and closed while either person blocks the other.

use std::collections::HashSet;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use stellarr_messages::MessageKey;

use super::announce::{self, DISABLED, ENABLED, Line, REQUEST_CANCELLED, REQUESTED};
use super::problems::{self, Refusal};
use super::store::{self, Conversation, Epoch, State as Switch};
use super::wire::{
    EnvelopeRequest, EpochRequest, EpochsDto, KeysDto, Members, PersonKeysDto, RecipientKeyDto,
    is_wire_id,
};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Path, Query, json_response};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// Both account keys and every device both people may have at once.
const MAX_ENVELOPES: usize = 2 * (store::MAX_ACTIVE_DEVICES as usize + 1);
const EPOCH_PAGE_SIZE: i64 = 200;

/// What a switch asks of the conversation.
#[derive(Clone, Copy)]
enum Action {
    Request,
    Cancel,
    Confirm,
    Disable,
}

impl Action {
    /// The line it adds to the timeline.
    fn line(self) -> &'static str {
        match self {
            Action::Request => REQUESTED,
            Action::Cancel => REQUEST_CANCELLED,
            Action::Confirm => ENABLED,
            Action::Disable => DISABLED,
        }
    }

    /// Whether the person needs an account key of their own to do it.
    fn needs_account_key(self) -> bool {
        matches!(self, Action::Request | Action::Confirm)
    }
}

/// What the conversation's state does not allow of a switch.
enum Refused {
    /// The state is not the one the switch starts from.
    State,
    /// Only the other person may do it.
    NotAllowed(MessageKey),
}

/// Makes the change a switch is, in memory, or says why it is refused.
fn decide(
    action: Action,
    conversation: &mut Conversation,
    user: Id,
    now: Timestamp,
) -> Result<(), Refused> {
    match action {
        Action::Request => {
            if conversation.state != Switch::Off {
                return Err(Refused::State);
            }
            conversation.state = Switch::Requested;
            conversation.requested_by = Some(user);
            conversation.requested_at = Some(now);
        }
        Action::Cancel => {
            if conversation.state != Switch::Requested {
                return Err(Refused::State);
            }
            if conversation.requested_by != Some(user) {
                return Err(Refused::NotAllowed(MessageKey::EncryptionCancelNotRequester));
            }
            switch_off(conversation);
        }
        Action::Confirm => {
            if conversation.state != Switch::Requested {
                return Err(Refused::State);
            }
            if conversation.requested_by == Some(user) {
                return Err(Refused::NotAllowed(MessageKey::EncryptionConfirmOwnRequest));
            }
            conversation.state = Switch::On;
            conversation.requested_by = None;
            conversation.requested_at = None;
            conversation.enabled_at = Some(now);
            conversation.current_epoch = None;
        }
        Action::Disable => {
            if conversation.state == Switch::Off {
                return Err(Refused::State);
            }
            switch_off(conversation);
        }
    }
    Ok(())
}

fn switch_off(conversation: &mut Conversation) {
    conversation.state = Switch::Off;
    conversation.requested_by = None;
    conversation.requested_at = None;
    conversation.enabled_at = None;
    conversation.current_epoch = None;
}

fn state_conflict(state: &AppState, conversation: &Conversation) -> ApiError {
    Refusal::new(state, StatusCode::CONFLICT, MessageKey::EncryptionStateConflict)
        .with_json("encryption", &conversation.dto())
        .code(problems::STATE_CONFLICT)
}

fn not_allowed(state: &AppState, title: MessageKey) -> ApiError {
    problems::coded(state, StatusCode::FORBIDDEN, problems::NOT_ALLOWED, title)
}

fn off(state: &AppState, conversation: &Conversation) -> ApiError {
    Refusal::new(state, StatusCode::CONFLICT, MessageKey::EncryptionOff)
        .with_json("encryption", &conversation.dto())
        .code(problems::OFF)
}

/// What asking the first questions of a switch came to.
enum Ready {
    NotFound,
    Blocked,
    /// The other person takes no request from this one.
    Declined,
    KeyMissing,
    Go,
}

/// Applies one switch in a write transaction, with the line it adds to the
/// timeline, and tells both people.
async fn switch(state: AppState, caller: Caller, id: Id, action: Action) -> ApiResult<Response> {
    let user = caller.user;
    let ready = state
        .database()
        .read(move |db| -> Result<Ready, DbError> {
            // 404 for an outsider, 403 within a block, as for its messages.
            let Some(conversation) = store::conversation_of(db, id, user)? else {
                return Ok(Ready::NotFound);
            };
            let peer = conversation.peer_of(user);
            if store::blocked(db, user, peer)? {
                return Ok(Ready::Blocked);
            }
            // A request reaches the other person like a message does:
            // somebody who takes direct messages only from friends takes
            // requests only from them too.
            if matches!(action, Action::Request) && !store::peer_takes_messages_from(db, user, peer)? {
                return Ok(Ready::Declined);
            }
            if action.needs_account_key() && store::account_key(db, user)?.is_none() {
                return Ok(Ready::KeyMissing);
            }
            Ok(Ready::Go)
        })
        .await?;
    match ready {
        Ready::NotFound => return Err(ApiError::not_found()),
        Ready::Blocked | Ready::Declined => return Err(ApiError::forbidden()),
        Ready::KeyMissing => return Err(problems::account_key_missing(&state)),
        Ready::Go => {}
    }

    let outcome = state
        .database()
        .write(move |tx| -> Result<Result<(Conversation, Line), Refused2>, DbError> {
            let Some(mut conversation) = store::conversation(tx, id)? else {
                return Err(DbError::Task("the conversation is gone".to_string()));
            };
            if let Err(refusal) = decide(action, &mut conversation, user, Timestamp::now()) {
                return Ok(Err(Refused2(refusal, conversation)));
            }
            conversation.save(tx)?;
            let line = announce::add_line(tx, id, user, action.line(), Timestamp::now())?;
            Ok(Ok((conversation, line)))
        })
        .await?;
    let (conversation, line) = match outcome {
        Ok(done) => done,
        Err(Refused2(Refused::State, conversation)) => return Err(state_conflict(&state, &conversation)),
        Err(Refused2(Refused::NotAllowed(title), _)) => return Err(not_allowed(&state, title)),
    };

    announce::line_created(&state, &conversation, &line);
    announce::encryption_changed(&state, &conversation);
    announce::social_changed(&state, &[conversation.first, conversation.second]);
    Ok(json_response(StatusCode::OK, &conversation.dto()))
}

/// A switch the conversation's state refused, with the conversation as it stands.
struct Refused2(Refused, Conversation);

/// Asks the other person to switch encryption on. Needs an account key.
pub async fn request(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    switch(state, caller, id, Action::Request).await
}

/// Takes back a request; only the person who sent it.
pub async fn cancel(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    switch(state, caller, id, Action::Cancel).await
}

/// Switches encryption on: only the person who was asked, and only with an
/// account key. The first message after it starts the first epoch.
pub async fn confirm(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    switch(state, caller, id, Action::Confirm).await
}

/// Switches encryption off, or turns a pending request down: either person, at
/// any time. Switching on again takes a request and a confirmation again.
pub async fn disable(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    switch(state, caller, id, Action::Disable).await
}

/// The conversation when the reader is in it and neither person blocks the
/// other.
fn readable(
    db: &rusqlite::Connection,
    id: Id,
    user: Id,
) -> rusqlite::Result<Option<Conversation>> {
    let Some(conversation) = store::conversation_of(db, id, user)? else {
        return Ok(None);
    };
    if store::blocked(db, user, conversation.peer_of(user))? {
        return Ok(None);
    }
    Ok(Some(conversation))
}

/// The keys of both people and the current epoch: everything a browser needs
/// to encrypt the next message, or to see that a new epoch is due.
pub async fn keys(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let found = state
        .database()
        .read(move |db| -> Result<Option<KeysDto>, DbError> {
            let user = caller.user;
            let Some(conversation) = readable(db, id, user)? else {
                return Ok(None);
            };
            let peer = conversation.peer_of(user);
            let now = Timestamp::now();
            let own_key = store::account_key(db, user)?;
            let peer_key = store::account_key(db, peer)?;
            let devices = store::devices_of_two(db, user, peer, now)?;
            let current = devices.iter().find(|device| device.session == caller.session);
            let own_key_ids: HashSet<Id> = own_key
                .iter()
                .map(|key| key.key_id)
                .chain(current.map(|device| device.id))
                .collect();

            let person = |who: Id, key: &Option<store::AccountKey>| PersonKeysDto {
                user_id: who,
                account_key: key.as_ref().map(store::AccountKey::dto),
                devices: devices
                    .iter()
                    .filter(|device| device.user == who)
                    .map(store::Device::key_dto)
                    .collect(),
            };

            let mut recipients: Vec<RecipientKeyDto> = [&own_key, &peer_key]
                .into_iter()
                .flatten()
                .map(|key| RecipientKeyDto {
                    key_id: key.key_id,
                    kind: store::ACCOUNT,
                    user_id: key.user,
                })
                .chain(devices.iter().map(|device| RecipientKeyDto {
                    key_id: device.id,
                    kind: store::DEVICE,
                    user_id: device.user,
                }))
                .collect();
            recipients.sort_by_key(|key| (key.user_id.to_string(), key.kind, key.key_id.to_string()));

            let current_epoch = match conversation.current_epoch {
                Some(epoch) => {
                    let epoch = store::epoch(db, epoch)?.ok_or_else(|| {
                        DbError::Task("the current epoch of a conversation is not there".to_string())
                    })?;
                    Some(epoch.dto(|envelope| own_key_ids.contains(&envelope.recipient_key_id)))
                }
                None => None,
            };

            Ok(Some(KeysDto {
                conversation_id: id,
                encryption: conversation.dto(),
                own: person(user, &own_key),
                peer: person(peer, &peer_key),
                current_device_key_id: current.map(|device| device.id),
                recipients,
                current_epoch,
            }))
        })
        .await?;
    found
        .map(|keys| json_response(StatusCode::OK, &keys))
        .ok_or_else(ApiError::not_found)
}

/// The query of the epochs: the sequence number to read after.
///
/// Read by hand the way the first server bound an `int?`: nothing or an empty
/// value is none, one that is not an integer, or given twice, is a bad request.
fn after_of(query: &[(String, String)]) -> Result<i64, ApiError> {
    let mut given = query.iter().filter(|(name, _)| name == "after");
    let first = given.next();
    if given.next().is_some() {
        return Err(ApiError::bad_request());
    }
    match first.map(|(_, value)| value.as_str()) {
        None | Some("") => Ok(0),
        Some(value) => {
            let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ApiError::bad_request());
            }
            value
                .parse::<i32>()
                .map(i64::from)
                .map_err(|_| ApiError::bad_request())
        }
    }
}

/// The conversation's epochs after a sequence number, from every period it was
/// encrypted, each with the envelopes for the reader's account key and this
/// browser's device. Envelopes for keys the reader no longer has are of no use
/// to them and are left out.
pub async fn epochs(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
    Query(query): Query<Vec<(String, String)>>,
) -> ApiResult<Response> {
    let after = after_of(&query)?;
    let found = state
        .database()
        .read(move |db| -> Result<Option<EpochsDto>, DbError> {
            let user = caller.user;
            let Some(conversation) = readable(db, id, user)? else {
                return Ok(None);
            };
            let now = Timestamp::now();
            let own_key = store::account_key(db, user)?;
            let device = store::current_device(db, caller.session, now)?;
            let own_key_ids: HashSet<Id> = own_key
                .iter()
                .map(|key| key.key_id)
                .chain(device.iter().map(|device| device.id))
                .collect();
            let epochs = store::epochs_after(db, id, after, EPOCH_PAGE_SIZE + 1)?;
            let has_more = epochs.len() as i64 > EPOCH_PAGE_SIZE;
            Ok(Some(EpochsDto {
                current_epoch_id: conversation.current_epoch,
                epochs: epochs
                    .iter()
                    .take(EPOCH_PAGE_SIZE as usize)
                    .map(|epoch| {
                        epoch.dto(|envelope| own_key_ids.contains(&envelope.recipient_key_id))
                    })
                    .collect(),
                has_more,
            }))
        })
        .await?;
    found
        .map(|epochs| json_response(StatusCode::OK, &epochs))
        .ok_or_else(ApiError::not_found)
}

fn invalid_epoch(state: &AppState) -> ApiError {
    problems::invalid(state, "envelopes", MessageKey::EncryptionEpochInvalid, &[])
}

/// An envelope as the protocol makes it: a nonce of 96 bits, a wrapped 256-bit
/// key with its tag, and the id of the key it is for in the one spelling.
struct Wrapped {
    key_id: Id,
    ephemeral_public_key: Vec<u8>,
    iv: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn wrapped(envelope: &EnvelopeRequest) -> Option<Wrapped> {
    let key_id = is_wire_id(envelope.recipient_key_id.as_deref()?)?;
    let iv = envelope.iv.clone().filter(|iv| iv.len() == 12)?;
    let ciphertext = envelope
        .ciphertext
        .clone()
        .filter(|bytes| (17..=256).contains(&bytes.len()))?;
    Some(Wrapped {
        key_id,
        ephemeral_public_key: envelope.ephemeral_public_key.clone()?,
        iv,
        ciphertext,
    })
}

/// What making an epoch came to.
enum Made {
    /// The same epoch again, as it was stored.
    Replayed(Epoch),
    Taken,
    Off(Conversation),
    Stale(Option<Id>),
    Coverage { missing: Vec<Id>, extra: Vec<Id> },
    Invalid,
    Created(Conversation, Epoch),
}

/// Starts a new epoch. It must follow the current one, so two browsers rotating
/// at once cannot both win, and it must reach exactly the account keys and
/// active devices of both people, so that no browser can leave a device out
/// without anybody noticing. Sending the same epoch again answers with the
/// stored one.
pub async fn create_epoch(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
    Json(body): Json<Members>,
) -> ApiResult<Response> {
    let request = EpochRequest::read(&body)?;
    let user = caller.user;
    let Some(epoch_id) = request.epoch_id.as_deref().and_then(is_wire_id) else {
        return Err(invalid_epoch(&state));
    };
    let Some(envelopes) = request
        .envelopes
        .as_ref()
        .filter(|list| (1..=MAX_ENVELOPES).contains(&list.len()))
    else {
        return Err(invalid_epoch(&state));
    };
    let mut keys: Vec<Wrapped> = Vec::with_capacity(envelopes.len());
    for envelope in envelopes {
        // A hole in the list is not an envelope.
        let Some(found) = envelope.as_ref().and_then(wrapped) else {
            return Err(invalid_epoch(&state));
        };
        keys.push(found);
    }
    let distinct: HashSet<Id> = keys.iter().map(|key| key.key_id).collect();
    if distinct.len() != keys.len() {
        return Err(invalid_epoch(&state));
    }
    let previous = request.previous_epoch_id;

    // A conversation the person is not in, or is blocked in, is refused as for
    // its messages.
    let allowed = state
        .database()
        .read(move |db| -> Result<Option<bool>, DbError> {
            let Some(conversation) = store::conversation_of(db, id, user)? else {
                return Ok(None);
            };
            Ok(Some(!store::blocked(db, user, conversation.peer_of(user))?))
        })
        .await?;
    match allowed {
        None => return Err(ApiError::not_found()),
        Some(false) => return Err(ApiError::forbidden()),
        Some(true) => {}
    }

    let made = state
        .database()
        .write(move |tx| -> Result<Made, DbError> {
            let now = Timestamp::now();
            if let Some(stored) = store::epoch(tx, epoch_id)? {
                return Ok(if stored.conversation == id && stored.created_by == user {
                    Made::Replayed(stored)
                } else {
                    Made::Taken
                });
            }
            let Some(mut conversation) = store::conversation(tx, id)? else {
                return Err(DbError::Task("the conversation is gone".to_string()));
            };
            if conversation.state != Switch::On {
                return Ok(Made::Off(conversation));
            }
            if conversation.current_epoch != previous {
                return Ok(Made::Stale(conversation.current_epoch));
            }
            let recipients = store::recipients(tx, conversation.first, conversation.second, now)?;
            let required: HashSet<Id> = recipients.iter().map(|recipient| recipient.key_id).collect();
            let mut missing: Vec<Id> = required.difference(&distinct).copied().collect();
            let mut extra: Vec<Id> = distinct.difference(&required).copied().collect();
            if !missing.is_empty() || !extra.is_empty() {
                missing.sort();
                extra.sort();
                return Ok(Made::Coverage { missing, extra });
            }
            let mut envelopes = Vec::with_capacity(recipients.len());
            for recipient in &recipients {
                let Some(key) = keys.iter().find(|key| key.key_id == recipient.key_id) else {
                    return Ok(Made::Invalid);
                };
                if !store::is_public_key(Some(&recipient.algorithm), Some(&key.ephemeral_public_key)) {
                    return Ok(Made::Invalid);
                }
                envelopes.push(store::Envelope {
                    recipient_key_id: recipient.key_id,
                    recipient_user: recipient.user,
                    kind: recipient.kind.to_string(),
                    ephemeral_public_key: key.ephemeral_public_key.clone(),
                    iv: key.iv.clone(),
                    ciphertext: key.ciphertext.clone(),
                });
            }
            let epoch = Epoch {
                id: epoch_id,
                conversation: id,
                sequence: store::last_sequence(tx, id)? + 1,
                previous,
                created_by: user,
                created_at: now,
                envelopes,
            };
            store::insert_epoch(tx, &epoch)?;
            conversation.current_epoch = Some(epoch_id);
            conversation.save(tx)?;
            Ok(Made::Created(conversation, epoch))
        })
        .await?;
    match made {
        Made::Replayed(stored) => Ok(json_response(
            StatusCode::OK,
            &stored.dto(|envelope| envelope.recipient_user == user),
        )),
        Made::Taken => Err(problems::coded(
            &state,
            StatusCode::CONFLICT,
            problems::EPOCH_EXISTS,
            MessageKey::EncryptionEpochExists,
        )),
        Made::Off(conversation) => Err(off(&state, &conversation)),
        Made::Stale(current) => Err(Refusal::new(
            &state,
            StatusCode::CONFLICT,
            MessageKey::EncryptionEpochStale,
        )
        .with("currentEpochId", current.map(|id| id.to_string()))
        .code(problems::EPOCH_STALE)),
        Made::Coverage { missing, extra } => Err(Refusal::new(
            &state,
            StatusCode::CONFLICT,
            MessageKey::EncryptionEpochCoverage,
        )
        .with("missing", missing.iter().map(Id::to_string).collect::<Vec<_>>())
        .with("extra", extra.iter().map(Id::to_string).collect::<Vec<_>>())
        .code(problems::EPOCH_COVERAGE)),
        Made::Invalid => Err(invalid_epoch(&state)),
        Made::Created(conversation, epoch) => {
            // Each person gets the envelopes for all of their own keys, so
            // that every one of their browsers can open the epoch without
            // asking.
            announce::epoch_created(&state, &conversation, |who| {
                epoch.dto(|envelope| envelope.recipient_user == who)
            });
            let mut response = json_response(
                StatusCode::CREATED,
                &epoch.dto(|envelope| envelope.recipient_user == user),
            );
            let location = format!("/api/social/conversations/{id}/encryption/epochs");
            if let Ok(location) = HeaderValue::from_str(&location) {
                response.headers_mut().insert(header::LOCATION, location);
            }
            Ok(response)
        }
    }
}
