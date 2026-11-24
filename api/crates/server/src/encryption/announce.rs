//! What the two people of a conversation, and the account's own browsers, are
//! told, and the lines the timeline shows between messages.
//!
//! Each event goes to the people's groups in turn, so a person who is in both
//! (a conversation with oneself cannot be) would hear it twice; to both people
//! of a conversation it goes first to the one stored first.

use rusqlite::{Transaction, params};
use serde::Serialize;

use super::store::{self, Conversation};
use super::wire::{EncryptionChangedDto, EpochCreatedDto, EpochDto, KeysChangedDto, Nothing};
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::instance::users::{User, UserDto};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The kinds of line a switch or a change of keys adds to the timeline.
pub const REQUESTED: &str = "encryption_requested";
pub const REQUEST_CANCELLED: &str = "encryption_request_cancelled";
pub const ENABLED: &str = "encryption_enabled";
pub const DISABLED: &str = "encryption_disabled";
pub const DEVICE_ADDED: &str = "device_added";
pub const KEYS_RESET: &str = "keys_reset";

/// A line of a conversation's timeline, as the clients read a message.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineDto {
    id: Id,
    channel_id: Id,
    author: UserDto,
    content: &'static str,
    created_at: Timestamp,
    attachments: [(); 0],
    embeds: [(); 0],
    edited_at: Option<Timestamp>,
    pinned_at: Option<Timestamp>,
    reply_to: Option<()>,
    reactions: Option<[(); 0]>,
    encrypted: Option<()>,
    system: SystemDto,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemDto {
    kind: &'static str,
    actor_id: Id,
}

/// A line that was written, with what the clients are told of it.
pub struct Line {
    dto: LineDto,
}

/// Adds a line to the conversation's timeline, made by `actor`.
pub fn add_line(
    tx: &Transaction,
    conversation: Id,
    actor: Id,
    kind: &'static str,
    now: Timestamp,
) -> Result<Line, DbError> {
    let author = User::find(tx, actor)?.ok_or_else(|| {
        DbError::Task("the account that makes a line of the timeline is not there".to_string())
    })?;
    let id = Id::random();
    tx.execute(
        "INSERT INTO \"DirectMessages\" (\"Id\", \"ConversationId\", \"AuthorId\", \"Content\", \"CreatedAt\", \"SystemKind\")
         VALUES (?1, ?2, ?3, '', ?4, ?5)",
        params![id, conversation, actor, now, kind],
    )?;
    Ok(Line {
        dto: LineDto {
            id,
            channel_id: conversation,
            author: author.dto("online"),
            content: "",
            created_at: now,
            attachments: [],
            embeds: [],
            edited_at: None,
            pinned_at: None,
            reply_to: None,
            reactions: None,
            encrypted: None,
            system: SystemDto {
                kind,
                actor_id: actor,
            },
        },
    })
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

/// Tells people's clients to reload the friends screen, each once.
pub fn social_changed(state: &AppState, people: &[Id]) {
    let mut each: Vec<Id> = Vec::new();
    for person in people {
        if !each.contains(person) {
            each.push(*person);
        }
    }
    state.events().publish(Event::new(
        Audience::Users(each),
        "SocialChanged",
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
                .map(|peer| Ok(!store::blocked(connection, user, *peer)?))
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
