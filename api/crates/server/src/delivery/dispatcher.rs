//! Sending what the outbox owes. A row is leased for a short while, every
//! reason not to send it is checked again outside the message's transaction,
//! and only then is a provider called. Providers are never called while a
//! message is being stored.
//!
//! One look at the outbox ([`dispatch`]):
//!
//! 1. rows that ran out their week are cancelled, and finished rows older
//!    than thirty days are deleted;
//! 2. when mail rows are due and mail is off or not set up, they are put an
//!    hour off, without a try counted and without a provider called;
//! 3. up to 25 due rows are leased for as long as the whole batch can take at
//!    its worst (see [`LEASE`]) and counted as tried, which
//!    is what keeps two workers (or two servers on one file) from sending a
//!    row twice at the same time;
//! 4. each leased row is checked, sent, and written down at once, so a server
//!    that stops in the middle loses no more than the row in its hands.
//!
//! Delivery is at least once. A crash between a provider taking a message and
//! the row being written leaves the row leased; when the lease ends it is sent
//! again (a mail keeps its key, so the provider can tell, and a push carries a
//! tag the browser replaces by). A row that was leased five times without
//! being written down is not leased again, and is cancelled when it expires.
//!
//! A person who is online is not treated differently: the service worker
//! hides a notice for a window that has focus, as the first server relied on.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use stellarr_messages::MessageKey;
use stellarr_webpush::{Dispatch, PushPayload};
use tracing::warn;

use super::outbox::{self, Kind};
use super::sender::{Sender, Stored};
use super::TARGET;
use crate::auth::events;
use crate::database::DbError;
use crate::mail::setup::{self, MailSetup, Mailing};
use crate::permissions::visibility;
use crate::social;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

pub const BATCH_SIZE: usize = 25;
pub const MAX_ATTEMPTS: i64 = 5;
/// The most one row can keep the worker: a push has ten seconds in all, a mail
/// fifteen, and the reads and writes around either take little beside that.
pub const WORST_PER_ROW: Duration = Duration::from_secs(20);
/// How long a batch holds its rows. Rows are sent one after another, so the
/// last of a batch of 25 is reached after 25 rows' worst, 500 seconds; a lease
/// shorter than that would run out under the rows still waiting, and a second
/// look could take them while this one still held them.
pub const LEASE: Duration = Duration::from_secs(BATCH_SIZE as u64 * WORST_PER_ROW.as_secs());
pub const RETRY_BASE: Duration = Duration::from_secs(2 * 60);
pub const RETRY_CAP: Duration = Duration::from_secs(60 * 60);
pub const MISSING_MAIL_DELAY: Duration = Duration::from_secs(60 * 60);
pub const RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// The condition of a row that may be leased now; `?1` is the moment.
const DUE: &str = "\"Status\" = 'pending' AND \"NextAttemptAt\" <= ?1 AND \"ExpiresAt\" > ?1
    AND \"Attempts\" < 5 AND (\"LeaseUntil\" IS NULL OR \"LeaseUntil\" <= ?1)";

/// Looks at the outbox as of `now` and sends what is due. Returns how many
/// destinations took a notification.
pub async fn dispatch(state: &AppState, sender: &Sender, now: Timestamp) -> Result<usize, DbError> {
    run(state, sender, || false, now).await
}

/// A row that is leased.
struct Leased {
    id: Id,
    source: Id,
    recipient: Id,
    kind: String,
    channel: String,
    key: String,
    subscription: Option<Id>,
    expires_at: Timestamp,
    attempts: i64,
}

/// Where a leased row is to be sent.
enum Target {
    Email(String),
    Push(Stored),
}

/// What went wrong with a row, in words that name no address: the text of the
/// errors underneath can carry the person's address, so it is never kept.
#[derive(Clone, Copy)]
enum Failure {
    Checks,
    Mail,
    Text,
}

impl Failure {
    fn describe(self) -> &'static str {
        match self {
            Failure::Checks => "the checks could not be read",
            Failure::Mail => "the mail could not be written",
            Failure::Text => "a text of the notice is missing",
        }
    }
}

enum Finished {
    Sent,
    Cancelled,
    /// The browser dropped the subscription: it goes, and the row is done.
    Gone(Id),
    Retry,
}

/// [`dispatch`], which asks `stopping` between two rows.
pub(crate) async fn run(
    state: &AppState,
    sender: &Sender,
    stopping: impl Fn() -> bool,
    now: Timestamp,
) -> Result<usize, DbError> {
    let reading = state.clone();
    let mailing: Option<Box<Mailing>> = state
        .database()
        .write(move |tx| -> Result<Option<Box<Mailing>>, DbError> {
            tx.execute(
                "UPDATE \"NotificationDeliveries\" SET \"Status\" = 'cancelled', \"LeaseId\" = NULL, \"LeaseUntil\" = NULL
                 WHERE \"Status\" = 'pending' AND \"ExpiresAt\" <= ?1",
                [now],
            )?;
            tx.execute(
                "DELETE FROM \"NotificationDeliveries\" WHERE \"Status\" <> 'pending' AND \"CreatedAt\" < ?1",
                [now.minus(RETENTION)],
            )?;
            let due_email: bool = tx.query_row(
                &format!(
                    "SELECT EXISTS (SELECT 1 FROM \"NotificationDeliveries\" WHERE \"Channel\" = 'email' AND {DUE})"
                ),
                [now],
                |row| row.get(0),
            )?;
            if !due_email {
                return Ok(None);
            }
            match setup::find(tx, reading.settings(), reading.sealer())? {
                MailSetup::Ready(mailing) => Ok(Some(mailing)),
                _ => {
                    // The operator has not set mail up, or has turned it off.
                    // The rows wait, and nothing outside is called.
                    tx.execute(
                        "UPDATE \"NotificationDeliveries\" SET \"NextAttemptAt\" = ?2
                         WHERE \"Status\" = 'pending' AND \"Channel\" = 'email' AND \"NextAttemptAt\" <= ?1",
                        params![now, now.plus(MISSING_MAIL_DELAY)],
                    )?;
                    Ok(None)
                }
            }
        })
        .await?;

    let claimed = claim(state, now).await?;
    let mut delivered = 0;
    for item in claimed {
        if stopping() {
            break;
        }
        let finished = match attempt(state, sender, mailing.as_deref(), &item, now).await {
            Ok(finished) => finished,
            Err(error) => {
                // The kind of failure and the id, never the error: its text can name an address.
                warn!(target: TARGET, "An activity notification delivery {} failed: {}.", item.id, error.describe());
                Finished::Retry
            }
        };
        if matches!(finished, Finished::Sent) {
            delivered += 1;
        }
        record(state, &item, finished, now).await?;
    }
    Ok(delivered)
}

/// Leases the next due rows. Each lease is taken by a statement that repeats
/// the condition, so a row another worker took in between is not taken twice.
async fn claim(state: &AppState, now: Timestamp) -> Result<Vec<Leased>, DbError> {
    state
        .database()
        .write(move |tx| -> Result<Vec<Leased>, DbError> {
            let lease = Id::random();
            let ids: Vec<Id> = {
                let mut statement = tx.prepare(&format!(
                    "SELECT \"Id\" FROM \"NotificationDeliveries\" WHERE {DUE}
                     ORDER BY \"NextAttemptAt\", \"CreatedAt\" LIMIT {BATCH_SIZE}"
                ))?;
                statement
                    .query_map([now], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?
            };
            for id in ids {
                tx.execute(
                    &format!(
                        "UPDATE \"NotificationDeliveries\" SET \"LeaseId\" = ?2, \"LeaseUntil\" = ?3, \"LastAttemptAt\" = ?1,
                             \"Attempts\" = \"Attempts\" + 1
                         WHERE \"Id\" = ?4 AND {DUE}"
                    ),
                    params![now, lease, now.plus(LEASE), id],
                )?;
            }
            let mut statement = tx.prepare(
                "SELECT \"Id\", \"SourceMessageId\", \"RecipientId\", \"EventKind\", \"Channel\", \"DestinationKey\",
                        \"PushSubscriptionId\", \"ExpiresAt\", \"Attempts\"
                 FROM \"NotificationDeliveries\" WHERE \"LeaseId\" = ?1 ORDER BY \"NextAttemptAt\", \"CreatedAt\"",
            )?;
            let rows = statement
                .query_map([lease], |row| {
                    Ok(Leased {
                        id: row.get(0)?,
                        source: row.get(1)?,
                        recipient: row.get(2)?,
                        kind: row.get(3)?,
                        channel: row.get(4)?,
                        key: row.get(5)?,
                        subscription: row.get(6)?,
                        expires_at: row.get(7)?,
                        attempts: row.get(8)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(rows)
        })
        .await
}

/// Checks one row again and sends it.
async fn attempt(
    state: &AppState,
    sender: &Sender,
    mailing: Option<&Mailing>,
    item: &Leased,
    now: Timestamp,
) -> Result<Finished, Failure> {
    let checking = Leased {
        id: item.id,
        source: item.source,
        recipient: item.recipient,
        kind: item.kind.clone(),
        channel: item.channel.clone(),
        key: item.key.clone(),
        subscription: item.subscription,
        expires_at: item.expires_at,
        attempts: item.attempts,
    };
    let target = state
        .database()
        .read(move |db| -> Result<Option<Target>, DbError> { Ok(prepare(db, &checking, now)?) })
        .await
        .map_err(|_| Failure::Checks)?;
    let Some(target) = target else {
        return Ok(Finished::Cancelled);
    };
    let Some(kind) = Kind::parse(&item.kind) else {
        return Ok(Finished::Cancelled);
    };
    let messages = state.messages();
    let delivery = u128::from_be_bytes(*item.id.as_bytes());
    match target {
        Target::Email(address) => {
            let Some(mailing) = mailing else {
                // Leased in the same look that found the setup gone.
                return Ok(Finished::Retry);
            };
            let build = match kind {
                Kind::Mention => stellarr_mail::mention,
                Kind::DirectMessage => stellarr_mail::direct_message,
            };
            let message = build(
                &mailing.sender,
                &address,
                &mailing.public_url,
                delivery,
                messages,
            )
            .map_err(|_| Failure::Mail)?;
            let outcome = state.mail().send(&message, &mailing.transport).await;
            Ok(if outcome.is_sent() {
                Finished::Sent
            } else {
                Finished::Retry
            })
        }
        Target::Push(subscription) => {
            let (title, body) = match kind {
                Kind::Mention => (
                    MessageKey::NotificationMentionPushTitle,
                    MessageKey::NotificationMentionBody,
                ),
                Kind::DirectMessage => (
                    MessageKey::NotificationDirectMessagePushTitle,
                    MessageKey::NotificationDirectMessageBody,
                ),
            };
            let public_url = &state.settings().operations.public_url;
            let id = item.id.to_string().replace('-', "");
            let payload = PushPayload {
                id: id.clone(),
                title: messages.get(title, &[]).map_err(|_| Failure::Text)?,
                body: messages.get(body, &[]).map_err(|_| Failure::Text)?,
                url: if public_url.is_empty() {
                    "/".to_string()
                } else {
                    format!("{public_url}/")
                },
                tag: id,
            };
            Ok(match sender.send(&subscription, &payload).await {
                Dispatch::Delivered => Finished::Sent,
                Dispatch::Gone => Finished::Gone(subscription.id),
                Dispatch::Retry => Finished::Retry,
            })
        }
    }
}

/// Writes down what became of one row.
async fn record(
    state: &AppState,
    item: &Leased,
    finished: Finished,
    now: Timestamp,
) -> Result<(), DbError> {
    let id = item.id;
    let attempts = item.attempts;
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            match finished {
                Finished::Sent => {
                    tx.execute(
                        "UPDATE \"NotificationDeliveries\" SET \"Status\" = 'sent', \"LastError\" = '', \"LeaseId\" = NULL,
                             \"LeaseUntil\" = NULL, \"SentAt\" = ?2 WHERE \"Id\" = ?1",
                        params![id, now],
                    )?;
                }
                Finished::Cancelled | Finished::Gone(_) => {
                    if let Finished::Gone(subscription) = finished {
                        // It is no destination for any later message either.
                        tx.execute(
                            "DELETE FROM \"PushSubscriptions\" WHERE \"Id\" = ?1",
                            [subscription],
                        )?;
                    }
                    tx.execute(
                        "UPDATE \"NotificationDeliveries\" SET \"Status\" = 'cancelled', \"LastError\" = '', \"LeaseId\" = NULL,
                             \"LeaseUntil\" = NULL WHERE \"Id\" = ?1",
                        [id],
                    )?;
                }
                Finished::Retry => {
                    let status = if attempts >= MAX_ATTEMPTS {
                        "failed"
                    } else {
                        "pending"
                    };
                    tx.execute(
                        "UPDATE \"NotificationDeliveries\" SET \"Status\" = ?2, \"LastError\" = 'delivery failed',
                             \"NextAttemptAt\" = ?3, \"LeaseId\" = NULL, \"LeaseUntil\" = NULL WHERE \"Id\" = ?1",
                        params![id, status, now.plus(backoff(attempts))],
                    )?;
                }
            }
            Ok(())
        })
        .await
}

/// How long after the `attempts`th try the next one waits: two minutes, then
/// doubling, to an hour.
pub fn backoff(attempts: i64) -> Duration {
    let exponent = (attempts - 1).clamp(0, 10) as u32;
    (RETRY_BASE * 2u32.pow(exponent)).min(RETRY_CAP)
}

/// The checks that must hold at delivery time, not only when the message was
/// stored. `None` means the row is cancelled.
fn prepare(db: &Connection, item: &Leased, now: Timestamp) -> rusqlite::Result<Option<Target>> {
    if item.expires_at <= now {
        return Ok(None);
    }
    let Some((disabled, username, session_version)) = db
        .query_row(
            "SELECT \"IsDisabled\", \"NormalizedUsername\", \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1",
            [item.recipient],
            |row| Ok((row.get::<_, bool>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    if disabled {
        return Ok(None);
    }
    let Some(preferences) = db
        .query_row(
            "SELECT \"EmailNotifications\", \"PushNotifications\", \"AllowDirectMessages\", \"MutedChannelIdsJson\"
             FROM \"UserPreferences\" WHERE \"UserId\" = ?1",
            [item.recipient],
            |row| {
                Ok(Preferences {
                    email: row.get(0)?,
                    push: row.get(1)?,
                    allow_direct_messages: row.get(2)?,
                    muted: row.get(3)?,
                })
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let wants = if item.channel == "email" {
        preferences.email
    } else {
        preferences.push
    };
    if !wants {
        return Ok(None);
    }
    let still_wanted = match Kind::parse(&item.kind) {
        Some(Kind::Mention) => mention_wanted(db, item, &username, &preferences)?,
        Some(Kind::DirectMessage) => direct_message_wanted(db, item, &preferences)?,
        None => false,
    };
    if !still_wanted {
        return Ok(None);
    }
    if item.channel == "email" {
        // The account must still have the same verified address. Changing or
        // revoking it completes the old row without sending.
        let Some(address) = events::verified_email(db, item.recipient)? else {
            return Ok(None);
        };
        return Ok((outbox::destination_key(&address) == item.key).then_some(Target::Email(address)));
    }
    let Some(subscription) = item.subscription else {
        return Ok(None);
    };
    let Some(stored) = db
        .query_row(
            "SELECT \"Id\", \"SessionId\", \"Endpoint\", \"P256dh\", \"Auth\" FROM \"PushSubscriptions\"
             WHERE \"Id\" = ?1 AND \"UserId\" = ?2",
            params![subscription, item.recipient],
            |row| {
                Ok((
                    Stored {
                        id: row.get(0)?,
                        endpoint: row.get(2)?,
                        p256dh: row.get(3)?,
                        auth: row.get(4)?,
                    },
                    row.get::<_, Id>(1)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let (stored, session) = stored;
    if outbox::push_key(stored.id) != item.key {
        return Ok(None);
    }
    let live: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"UserSessions\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL
             AND \"ExpiresAt\" > ?3 AND \"AbsoluteExpiresAt\" IS NOT NULL AND \"AbsoluteExpiresAt\" > ?3
             AND \"SecurityVersion\" = ?4)",
        params![session, item.recipient, now, session_version],
        |row| row.get(0),
    )?;
    Ok(live.then_some(Target::Push(stored)))
}

struct Preferences {
    email: bool,
    push: bool,
    allow_direct_messages: bool,
    muted: String,
}

fn mention_wanted(
    db: &Connection,
    item: &Leased,
    username: &str,
    preferences: &Preferences,
) -> rusqlite::Result<bool> {
    let Some((channel, author, content, created_at)) = db
        .query_row(
            "SELECT \"ChannelId\", \"AuthorId\", \"Content\", \"CreatedAt\" FROM \"Messages\"
             WHERE \"Id\" = ?1 AND \"IsSystem\" = 0",
            [item.source],
            |row| Ok((row.get::<_, Id>(0)?, row.get::<_, Id>(1)?, row.get::<_, String>(2)?, row.get::<_, Timestamp>(3)?)),
        )
        .optional()?
    else {
        return Ok(false);
    };
    if author == item.recipient {
        return Ok(false);
    }
    // The recipient must still see the channel: one they lost since the
    // message was sent tells them nothing of it either.
    if !visibility::views(db, item.recipient, channel)? {
        return Ok(false);
    }
    if social::is_blocked(db, item.recipient, author)? || is_muted(&preferences.muted, channel) {
        return Ok(false);
    }
    // A message the recipient has already read must not pop up later.
    let read: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"ChannelReads\" WHERE \"ChannelId\" = ?1 AND \"UserId\" = ?2 AND \"LastReadAt\" >= ?3)",
        params![channel, item.recipient, created_at],
        |row| row.get(0),
    )?;
    if read {
        return Ok(false);
    }
    // An edit may have removed the name; only a mention still in the message
    // is worth an external notice.
    Ok(stellarr_mentions::extract_normalized_usernames(&content).contains(username))
}

fn direct_message_wanted(
    db: &Connection,
    item: &Leased,
    preferences: &Preferences,
) -> rusqlite::Result<bool> {
    let Some((conversation, author, created_at)) = db
        .query_row(
            "SELECT \"ConversationId\", \"AuthorId\", \"CreatedAt\" FROM \"DirectMessages\" WHERE \"Id\" = ?1",
            [item.source],
            |row| Ok((row.get::<_, Id>(0)?, row.get::<_, Id>(1)?, row.get::<_, Timestamp>(2)?)),
        )
        .optional()?
    else {
        return Ok(false);
    };
    let Some((user_a, user_b)) = db
        .query_row(
            "SELECT \"UserAId\", \"UserBId\" FROM \"DirectConversations\" WHERE \"Id\" = ?1",
            [conversation],
            |row| Ok((row.get::<_, Id>(0)?, row.get::<_, Id>(1)?)),
        )
        .optional()?
    else {
        return Ok(false);
    };
    let recipient = item.recipient;
    if (user_a != recipient && user_b != recipient) || author == recipient {
        return Ok(false);
    }
    let peer = if user_a == recipient { user_b } else { user_a };
    if peer != author || social::is_blocked(db, recipient, author)? {
        return Ok(false);
    }
    if !preferences.allow_direct_messages {
        let friends = social::are_friends(db, recipient, author)?;
        if !friends {
            return Ok(false);
        }
    }
    let read: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"DirectConversationReads\" WHERE \"ConversationId\" = ?1 AND \"UserId\" = ?2
             AND \"LastReadAt\" >= ?3)",
        params![conversation, recipient, created_at],
        |row| row.get(0),
    )?;
    Ok(!read)
}

/// Whether the channel is in the person's list of muted ones. A list that is
/// not a list of strings counts as none.
fn is_muted(json: &str, channel: Id) -> bool {
    serde_json::from_str::<Option<Vec<String>>>(json)
        .ok()
        .flatten()
        .is_some_and(|muted| {
            muted
                .iter()
                .any(|value| Id::parse_guid(value) == Some(channel))
        })
}
