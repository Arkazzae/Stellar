//! What encryption asks of a post: while it is on, a message must be encrypted
//! with the conversation's current epoch, and that epoch must still reach
//! exactly the account keys and active devices of both people; while it is
//! not on, nothing may be encrypted.
//!
//! The codes are the ones clients branch on. Another group owns the routes that
//! switch encryption on and make epochs; these are the refusals of this one.

use axum::http::StatusCode;
use rusqlite::Connection;
use stellarr_messages::{MessageKey, Messages};

use crate::http::{ApiError, Problem};
use crate::social::conversations::Conversation;
use crate::values::{Id, Timestamp};

use crate::encryption::problems::{ATTACHMENT_KIND, EPOCH_NOT_CURRENT, EPOCH_OUTDATED, OFF, REQUIRED};

/// A refusal with a title in the instance's language, what the client needs to
/// recover, and its code last.
fn problem(messages: &Messages, title: MessageKey, code: &str) -> Result<Problem, ApiError> {
    Ok(Problem::new(StatusCode::CONFLICT)
        .title(messages.get(title, &[])?)
        .with("code", code))
}

pub fn attachment_kind(messages: &Messages) -> ApiError {
    match problem(messages, MessageKey::EncryptionAttachmentKindMismatch, ATTACHMENT_KIND) {
        Ok(problem) => problem.into(),
        Err(error) => error,
    }
}

/// Everything a conflict about the conversation says: its state with the
/// extras first and the code last.
fn conflict(
    messages: &Messages,
    title: MessageKey,
    code: &str,
    extras: impl FnOnce(Problem) -> Problem,
) -> ApiError {
    match messages.get(title, &[]) {
        Ok(text) => extras(Problem::new(StatusCode::CONFLICT).title(text))
            .with("code", code)
            .into(),
        Err(error) => error.into(),
    }
}

/// The keys an epoch must be wrapped for, as ids: both account keys and every
/// device of both that still signs a browser in.
fn required(connection: &Connection, first: Id, second: Id, now: Timestamp) -> rusqlite::Result<Vec<Id>> {
    Ok(crate::encryption::store::recipients(connection, first, second, now)?
        .into_iter()
        .map(|recipient| recipient.key_id)
        .collect())
}

/// What the message says it was encrypted with.
pub struct Payload {
    pub epoch: Id,
    pub iv: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// Why a message cannot go into the conversation as it stands, or none when
/// it can. It is asked in the transaction that writes the message, so that no
/// switch or new epoch can come between the check and the message.
pub fn refusal(
    connection: &Connection,
    messages: &Messages,
    conversation: &Conversation,
    payload: Option<&Payload>,
    now: Timestamp,
) -> Result<(), ApiError> {
    let shown = conversation.encryption();
    if conversation.encryption_state != "On" {
        return match payload {
            None => Ok(()),
            Some(_) => Err(conflict(messages, MessageKey::EncryptionOff, OFF, |p| {
                p.with_json("encryption", &shown)
            })),
        };
    }
    let Some(payload) = payload else {
        return Err(conflict(messages, MessageKey::EncryptionRequired, REQUIRED, |p| {
            p.with_json("encryption", &shown)
        }));
    };
    let Some(current) = conversation.current_epoch.filter(|current| *current == payload.epoch) else {
        let current = conversation.current_epoch.map(|id| id.to_string());
        return Err(conflict(
            messages,
            MessageKey::EncryptionEpochNotCurrent,
            EPOCH_NOT_CURRENT,
            |p| p.with("currentEpochId", current),
        ));
    };
    let mut needed = required(connection, conversation.user_a, conversation.user_b, now)?;
    let mut wrapped: Vec<Id> = {
        let mut statement = connection
            .prepare("SELECT \"RecipientKeyId\" FROM \"EncryptionEnvelopes\" WHERE \"EpochId\" = ?1")?;
        statement
            .query_map([current], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    needed.sort();
    needed.dedup();
    wrapped.sort();
    wrapped.dedup();
    let missing: Vec<String> = needed
        .iter()
        .filter(|id| !wrapped.contains(id))
        .map(Id::to_string)
        .collect();
    let extra: Vec<String> = wrapped
        .iter()
        .filter(|id| !needed.contains(id))
        .map(Id::to_string)
        .collect();
    if missing.is_empty() && extra.is_empty() {
        return Ok(());
    }
    Err(conflict(
        messages,
        MessageKey::EncryptionEpochOutdated,
        EPOCH_OUTDATED,
        |p| p.with("missing", missing).with("extra", extra),
    ))
}
