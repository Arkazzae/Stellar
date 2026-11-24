//! What the two people of a conversation, and the account's own browsers, are
//! told, and the lines the timeline shows between messages.
//!
//! Each event goes to the people's groups in turn, so a person who is in both
//! (a conversation with oneself cannot be) would hear it twice; to both people
//! of a conversation it goes first to the one stored first.

use rusqlite::{Transaction, params};

use super::store::Conversation;
use super::wire::{EncryptionChangedDto, EpochCreatedDto, EpochDto, KeysChangedDto, Nothing};
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::social::conversations::DirectMessage;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The kinds of line a switch or a change of keys adds to the timeline.
pub const REQUESTED: &str = "encryption_requested";
pub const REQUEST_CANCELLED: &str = "encryption_request_cancelled";
pub const ENABLED: &str = "encryption_enabled";
pub const DISABLED: &str = "encryption_disabled";
pub const DEVICE_ADDED: &str = "device_added";
pub const KEYS_RESET: &str = "keys_reset";

/// A line that was written, as the clients read a message.
pub struct Line {
    dto: DirectMessage,
}

/// Adds a line to the conversation's timeline, made by `actor`.
pub fn add_line(
    tx: &Transaction,
    conversation: Id,
    actor: Id,
    kind: &'static str,
    now: Timestamp,
) -> Result<Line, DbError> {
    let id = Id::random();
    tx.execute(
        "INSERT INTO \"DirectMessages\" (\"Id\", \"ConversationId\", \"AuthorId\", \"Content\", \"CreatedAt\", \"SystemKind\")
         VALUES (?1, ?2, ?3, '', ?4, ?5)",
        params![id, conversation, actor, now, kind],
    )?;
    let (dto, _) = crate::social::conversations::message(tx, id)?
        .ok_or_else(|| DbError::Task("the line just written is not there".to_string()))?;
    Ok(Line { dto })
}

fn both(conversation: &Conversation) -> Audience {
    Audience::Users(vec![conversation.first, conversation.second])
}

/// Tells the account's own browsers to read their encryption state again.
pub fn account_changed(state: &AppState, user: Id) {
    state.events().publish(Event::new(
        Audience::Users(vec![user]),
        "EncryptionAccountChanged",
        &Nothing {},
    ));
}

/// Tells both people that a line was added.
pub fn line_created(state: &AppState, conversation: &Conversation, line: &Line) {
    state.events().publish(Event::new(
        both(conversation),
        "DirectMessageCreated",
        &line.dto,
    ));
}

/// Tells both people where the conversation's encryption stands.
pub fn encryption_changed(state: &AppState, conversation: &Conversation) {
    state.events().publish(Event::new(
        both(conversation),
        "DirectEncryptionChanged",
        &EncryptionChangedDto {
            conversation_id: conversation.id,
            encryption: conversation.dto(),
        },
    ));
}

/// Tells each person a new epoch, with the envelopes for their own keys.
pub fn epoch_created(state: &AppState, conversation: &Conversation, epoch: impl Fn(Id) -> EpochDto) {
    for user in [conversation.first, conversation.second] {
        state.events().publish(Event::new(
            Audience::Users(vec![user]),
            "DirectEncryptionEpochCreated",
            &EpochCreatedDto {
                conversation_id: conversation.id,
                epoch: epoch(user),
            },
        ));
    }
}

/// Tells both people of each conversation about the lines that were saved, and
/// that the person's keys changed, so that the next message starts a new
/// epoch. A conversation closed by a block hears nothing: the line is in its
/// history for when the block is lifted, but a new sign-in is not news to
/// somebody who was blocked or who blocked the person. The other people who
/// were told.
pub async fn keys_changed(
    state: &AppState,
    changes: Vec<(Conversation, Option<Line>)>,
    user: Id,
) -> Result<Vec<Id>, DbError> {
    let peers: Vec<Id> = changes
        .iter()
        .map(|(conversation, _)| conversation.peer_of(user))
        .collect();
    let open: Vec<bool> = state
        .database()
        .read(move |connection| -> Result<Vec<bool>, DbError> {
            peers
                .iter()
                .map(|peer| Ok(!crate::social::is_blocked(connection, user, *peer)?))
                .collect()
        })
        .await?;
    let mut told = Vec::new();
    for ((conversation, line), open) in changes.iter().zip(open) {
        if !open {
            continue;
        }
        told.push(conversation.peer_of(user));
        if let Some(line) = line {
            line_created(state, conversation, line);
        }
        state.events().publish(Event::new(
            both(conversation),
            "DirectEncryptionKeysChanged",
            &KeysChangedDto {
                conversation_id: conversation.id,
                user_id: user,
            },
        ));
    }
    Ok(told)
}
