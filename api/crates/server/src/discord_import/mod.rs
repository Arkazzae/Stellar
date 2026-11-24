//! Writes what the Discord reader read into a Stellarr server that already
//! exists: its sections and channels, placeholder accounts for its authors,
//! and its history, page by page as the reader gives it.
//!
//! Nothing calls it yet: no route starts an import. A route that does would
//! read a snapshot with `stellarr_discord`, take an [`ImportWriter`] on the
//! state's database, and call, in this order, [`ImportWriter::start`],
//! [`ImportWriter::write_structure`], then for every channel and thread
//! [`ImportWriter::write_messages`] once for each page the reader gives (newest
//! first) and [`ImportWriter::complete_channel`] when the history is whole, and
//! [`ImportWriter::tally`] to say what came of it.
//!
//! Everything written is recorded under Discord's id in the bookkeeping, and a
//! message's Stellarr id is derived from the server and Discord's id, so a
//! second run, or a page read twice, writes nothing twice. What somebody
//! deleted since stays deleted. That is also how an import that was interrupted
//! goes on: every batch is one transaction, so what is stored is whole batches,
//! and the channel's record says where to read on from
//! ([`ImportWriter::resume_after`]). Completing a channel is several
//! transactions too, so a driver that was interrupted calls
//! [`ImportWriter::complete_channel`] again for every channel it had read to the
//! end: it links what is not linked and changes nothing else.
//!
//! History is restored, not posted: nothing a send does happens here. No slow
//! mode, no mention notification or outbox row, no event, no link preview, no
//! post time and no audit line. The journal, the post times and the read
//! markers, which triggers and bootstraps keep, are left as if the history had
//! always been there ([`journal`]). The imported messages are ordinary rows of
//! `Messages`, so history, sync and search find them as they find a posted one:
//! the triggers of the schema keep the journal and the index.
//!
//! A placeholder is one per Discord id on the whole instance, so a person met
//! by an earlier import keeps the name that import gave them. It is an account
//! with `Origin` set: disabled, without a password or an address, in no server,
//! and found by nobody (a friend request, a conversation, an invitation or a
//! block answers as for an account that is not there).
//!
//! The writer holds no lock of its own: one import of a server runs at a time,
//! which whoever drives it has to see to. It keeps the time of its last commit,
//! so one instance should drive one import.
//!
//! What it does not do, as the first server's did not: roles, permission
//! overwrites, emoji, stickers and attachments are not imported. A channel
//! whose overwrites hid it from everyone is recorded as private
//! ([`StructureResult::hidden_from_everyone`]) and is open to the whole server
//! until a later stage closes it.

mod journal;
mod links;
mod messages;
mod names;
mod structure;
mod tally;

pub use names::message_id;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use stellarr_discord::DiscordChannelType;

use crate::database::{Database, DbError};
use crate::values::{Id, Timestamp};

/// How many messages one transaction writes. SQLite has one writer, and a
/// message costs its own row, one of bookkeeping, and the rows the triggers add
/// to the journal and the search index. A few hundred keep a transaction to
/// tens of milliseconds, which a live send waits out without anybody noticing;
/// a whole page of a large import would not.
pub const MESSAGES_PER_TRANSACTION: usize = 250;

/// The pause before a transaction that follows another. A writer that found
/// the database busy tries again after a short wait, so any pause shorter than
/// that could take the lock back before a waiting send looked again.
pub const PAUSE_BETWEEN_TRANSACTIONS: Duration = Duration::from_millis(200);

/// Why an import step was refused or failed.
#[derive(Debug)]
pub enum ImportError {
    /// An argument that no import can be made of: a template, the snapshot of
    /// another Discord server, a page of another channel, a channel the import
    /// has not recorded, a server that is not there.
    Argument(String),
    /// The state of the data forbids it: an account that is a person's, not a
    /// placeholder, holds the name a placeholder would take.
    State(String),
    Database(DbError),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Argument(text) | ImportError::State(text) => f.write_str(text),
            ImportError::Database(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<DbError> for ImportError {
    fn from(error: DbError) -> ImportError {
        ImportError::Database(error)
    }
}

impl From<rusqlite::Error> for ImportError {
    fn from(error: rusqlite::Error) -> ImportError {
        ImportError::Database(error.into())
    }
}

/// One import of a Discord server into a Stellarr server that already exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub id: Id,
    /// The Stellarr server it writes into, or none once that server is deleted.
    pub server: Option<Id>,
    pub discord_server_id: String,
    pub discord_server_name: String,
    pub started_by: Option<Id>,
    pub started_at: Timestamp,
}

/// Whether a record is a channel or a thread in one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordKind {
    Channel = 0,
    Thread = 1,
}

/// What became of one Discord message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Written = 0,
    Skipped = 1,
    AwaitingAttachments = 2,
}

/// Why a Discord message was left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkipReason {
    None = 0,
    /// A notice Discord wrote (a join, a boost, a pin and the like).
    Notice = 1,
    /// A copy of another message, which Stellarr has no shape for.
    Forward = 2,
    OnlySticker = 3,
    /// Only embeds, as bots post.
    OnlyEmbeds = 4,
    /// Nothing at all, which is what every message reads as when the bot's app
    /// lacks the Message Content intent.
    Empty = 5,
    /// The chat of a voice channel, which Stellarr does not have.
    VoiceChannel = 6,
    /// Its Stellarr channel was deleted after the import made it.
    ChannelRemoved = 7,
    /// Discord named no author and no webhook.
    NoAuthor = 8,
    /// Longer, once rewritten, than a Stellarr message may be.
    TooLong = 9,
    /// The id the message would get is held by a message the import did not
    /// write: a client may choose the id of what it sends.
    IdTaken = 10,
}

impl SkipReason {
    pub(crate) fn from_number(number: i64) -> SkipReason {
        match number {
            1 => SkipReason::Notice,
            2 => SkipReason::Forward,
            3 => SkipReason::OnlySticker,
            4 => SkipReason::OnlyEmbeds,
            5 => SkipReason::Empty,
            6 => SkipReason::VoiceChannel,
            7 => SkipReason::ChannelRemoved,
            8 => SkipReason::NoAuthor,
            9 => SkipReason::TooLong,
            10 => SkipReason::IdTaken,
            _ => SkipReason::None,
        }
    }
}

macro_rules! stored_as_number {
    ($($kind:ident),*) => {$(
        impl ToSql for $kind {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(*self as i64))
            }
        }
    )*};
}
stored_as_number!(RecordKind, Outcome, SkipReason);

impl FromSql for RecordKind {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<RecordKind> {
        Ok(if i64::column_result(value)? == 1 {
            RecordKind::Thread
        } else {
            RecordKind::Channel
        })
    }
}

impl FromSql for Outcome {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Outcome> {
        Ok(match i64::column_result(value)? {
            0 => Outcome::Written,
            2 => Outcome::AwaitingAttachments,
            _ => Outcome::Skipped,
        })
    }
}

/// What a channel was on Discord, stored as a number.
pub(crate) fn type_number(kind: DiscordChannelType) -> i64 {
    match kind {
        DiscordChannelType::Conversation => 0,
        DiscordChannelType::Announcements => 1,
        DiscordChannelType::Forum => 2,
        DiscordChannelType::Media => 3,
        DiscordChannelType::Stage => 4,
    }
}

pub(crate) fn type_of(number: i64) -> DiscordChannelType {
    match number {
        1 => DiscordChannelType::Announcements,
        2 => DiscordChannelType::Forum,
        3 => DiscordChannelType::Media,
        4 => DiscordChannelType::Stage,
        _ => DiscordChannelType::Conversation,
    }
}

pub(crate) fn is_forum(kind: DiscordChannelType) -> bool {
    matches!(kind, DiscordChannelType::Forum | DiscordChannelType::Media)
}

/// A moment of the reader's as one of the server's.
pub(crate) fn moment(at: stellarr_discord::Timestamp) -> Timestamp {
    let ticks = at.unix_seconds() * 10_000_000 + i64::from(at.subsec_nanos() / 100);
    Timestamp::from_ticks(ticks).unwrap_or_else(Timestamp::now)
}

/// What one structure write made, and what it found already there.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StructureResult {
    pub sections_made: usize,
    pub channels_made: usize,
    pub threads_recorded: usize,
    pub already_imported: usize,
    /// The channels that Stellarr only has as text or voice, by Discord id,
    /// with what they were: forums, media channels, announcement and stage
    /// channels.
    pub simplified: BTreeMap<String, DiscordChannelType>,
    /// The channels whose Discord overwrites hid them from everyone. Overwrites
    /// are not imported, so these are open to the whole Stellarr server until a
    /// later stage closes them.
    pub hidden_from_everyone: Vec<String>,
}

/// What one page of a channel's messages came to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageResult {
    pub written: usize,
    /// Messages an earlier page already took, which this one left alone.
    pub already_seen: usize,
    /// Written with their text; their attachments wait for the attachment stage.
    pub attachments_pending: usize,
    /// Nothing but attachments: recorded, not written, for the attachment stage.
    pub awaiting_attachments: usize,
    pub placeholders_made: usize,
    pub skipped: BTreeMap<SkipReason, usize>,
    /// The notices among `skipped`, by Discord's message type.
    pub notices_by_type: BTreeMap<i32, usize>,
}

/// What linking one channel or thread did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkResult {
    pub replies_linked: usize,
    pub replies_left_plain: usize,
    pub threads_linked: usize,
    /// A thread started from a message of a parent channel whose history is not
    /// read yet: it is linked when the parent is completed.
    pub waiting: bool,
}

/// Everything an import has met on one server so far, counted from the
/// bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tally {
    pub written: usize,
    pub attachments_pending: usize,
    pub awaiting_attachments: usize,
    pub skipped: BTreeMap<SkipReason, usize>,
    pub notices_by_type: BTreeMap<i32, usize>,
    pub placeholders: usize,
}

/// The writer. It owns nothing but the database handle and the memory of its
/// last commit, which the pause between transactions is measured from.
pub struct ImportWriter {
    database: Database,
    last_commit: Mutex<Option<Instant>>,
}

impl ImportWriter {
    pub fn new(database: Database) -> ImportWriter {
        ImportWriter {
            database,
            last_commit: Mutex::new(None),
        }
    }

    /// Waits out [`PAUSE_BETWEEN_TRANSACTIONS`] since the last commit, so that a
    /// write that queued behind the last transaction gets its turn.
    async fn pause(&self) {
        let last = *self.last_commit.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(last) = last
            && let Some(left) = PAUSE_BETWEEN_TRANSACTIONS.checked_sub(last.elapsed())
        {
            tokio::time::sleep(left).await;
        }
    }

    fn committed(&self) {
        *self.last_commit.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
    }
}

fn require_bot(snapshot: &stellarr_discord::ServerSnapshot) -> Result<(), ImportError> {
    if snapshot.source != stellarr_discord::SnapshotSource::Bot {
        // A template numbers its channels 0, 1, 2...: those are not ids
        // anything can be recorded under, and it has no history anyway.
        return Err(ImportError::Argument(
            "An import is read by a bot; a template has no ids to record.".to_string(),
        ));
    }
    Ok(())
}

fn require_same_server(
    import: &Import,
    snapshot: &stellarr_discord::ServerSnapshot,
) -> Result<(), ImportError> {
    require_bot(snapshot)?;
    if snapshot.source_id != import.discord_server_id {
        return Err(ImportError::Argument(
            "The snapshot is of another Discord server than the import's.".to_string(),
        ));
    }
    Ok(())
}

fn require_server(import: &Import) -> Result<Id, ImportError> {
    import
        .server
        .ok_or_else(|| ImportError::State("The server this import wrote into is gone.".to_string()))
}
