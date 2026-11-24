//! The rows of encryption, and the questions asked of them.
//!
//! A device belongs to a signed-in session and nothing is written on it when
//! that session ends. Whether it still takes part is a question [`ACTIVE`]
//! asks of the session every time: a sign-out, a revoked or expired session, a
//! password change that moved the account to a new version and a disabled
//! account all end a device through the rows of the session and the account,
//! whichever route or worker made them.

use rusqlite::{Connection, OptionalExtension, Row, named_params, params};

use super::wire::{
    DeviceKeyDto, EncryptionDto, EnvelopeDto, EpochDto, Octets, OwnDeviceDto, PublicKeyDto,
    RecipientKeyDto,
};
use crate::values::{Id, Timestamp};

pub const ACCOUNT: &str = "account";
pub const DEVICE: &str = "device";
pub const X25519: &str = "x25519";

/// The most devices an account holds keys in at once. Every epoch is wrapped
/// for each of them, so the bound keeps an epoch within reach of a request.
pub const MAX_ACTIVE_DEVICES: i64 = 100;
/// Wrong PINs after which the backup is deleted.
pub const MAX_FAILED_ATTEMPTS: i32 = 10;

/// A device's row with the session and the account it is bound to, for
/// [`ACTIVE`].
const DEVICES: &str = "\"EncryptionDevices\" AS d
    INNER JOIN \"UserSessions\" AS s ON d.\"SessionId\" = s.\"Id\"
    INNER JOIN \"Users\" AS u ON s.\"UserId\" = u.\"Id\"";

/// A device that takes part: not revoked, and bound to a session that still
/// signs its browser in by the rules of the session cookie. Wants `:now`.
const ACTIVE: &str = "d.\"RevokedAt\" IS NULL AND s.\"RevokedAt\" IS NULL AND s.\"ExpiresAt\" > :now
    AND s.\"AbsoluteExpiresAt\" IS NOT NULL AND s.\"AbsoluteExpiresAt\" > :now
    AND s.\"SecurityVersion\" = u.\"SessionVersion\" AND NOT u.\"IsDisabled\"";

const DEVICE_COLUMNS: &str = "d.\"Id\", d.\"UserId\", d.\"SessionId\", d.\"Algorithm\", d.\"PublicKey\", d.\"Label\",
    d.\"CreatedAt\", d.\"LastSeenAt\", s.\"LastSeenAt\"";

pub fn is_public_key(algorithm: Option<&str>, key: Option<&[u8]>) -> bool {
    algorithm == Some(X25519) && key.is_some_and(|key| key.len() == 32)
}

// ---- the account's keys ----

pub struct AccountKey {
    pub user: Id,
    pub key_id: Id,
    pub algorithm: String,
    pub public_key: Vec<u8>,
    pub created_at: Timestamp,
}

impl AccountKey {
    pub fn dto(&self) -> PublicKeyDto {
        PublicKeyDto {
            key_id: self.key_id,
            algorithm: self.algorithm.clone(),
            public_key: Octets(self.public_key.clone()),
            created_at: self.created_at,
        }
    }
}

pub fn account_key(connection: &Connection, user: Id) -> rusqlite::Result<Option<AccountKey>> {
    connection
        .query_row(
            "SELECT \"UserId\", \"KeyId\", \"Algorithm\", \"PublicKey\", \"CreatedAt\"
             FROM \"AccountKeys\" WHERE \"UserId\" = ?1",
            [user],
            |row| {
                Ok(AccountKey {
                    user: row.get(0)?,
                    key_id: row.get(1)?,
                    algorithm: row.get(2)?,
                    public_key: row.get(3)?,
                    created_at: row.get(4)?,
                })
            },
        )
        .optional()
}

pub struct Backup {
    pub salt: Vec<u8>,
    pub iterations: i32,
    pub verifier: Vec<u8>,
    pub iv: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub failed_attempts: i32,
    pub updated_at: Timestamp,
}

impl Backup {
    pub fn remaining_attempts(&self) -> i32 {
        (MAX_FAILED_ATTEMPTS - self.failed_attempts).max(0)
    }

    pub fn status(&self) -> super::wire::BackupStatusDto {
        super::wire::BackupStatusDto {
            iterations: self.iterations,
            failed_attempts: self.failed_attempts,
            remaining_attempts: self.remaining_attempts(),
            updated_at: self.updated_at,
        }
    }
}

pub fn backup(connection: &Connection, user: Id) -> rusqlite::Result<Option<Backup>> {
    connection
        .query_row(
            "SELECT \"Salt\", \"Iterations\", \"Verifier\", \"Iv\", \"Ciphertext\", \"FailedAttempts\", \"UpdatedAt\"
             FROM \"AccountKeyBackups\" WHERE \"UserId\" = ?1",
            [user],
            |row| {
                Ok(Backup {
                    salt: row.get(0)?,
                    iterations: row.get(1)?,
                    verifier: row.get(2)?,
                    iv: row.get(3)?,
                    ciphertext: row.get(4)?,
                    failed_attempts: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        )
        .optional()
}

/// Replaces the account's backup with a fresh one: no wrong PINs yet.
pub fn put_backup(
    connection: &Connection,
    user: Id,
    backup: &super::wire::NewBackup,
    now: Timestamp,
) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM \"AccountKeyBackups\" WHERE \"UserId\" = ?1",
        [user],
    )?;
    connection.execute(
        "INSERT INTO \"AccountKeyBackups\" (\"UserId\", \"Salt\", \"Iterations\", \"Verifier\", \"Iv\", \"Ciphertext\",
                                          \"FailedAttempts\", \"UpdatedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
        params![
            user,
            backup.salt,
            backup.iterations,
            backup.verifier,
            backup.iv,
            backup.ciphertext,
            now
        ],
    )?;
    Ok(())
}

// ---- devices ----

#[derive(Clone)]
pub struct Device {
    pub id: Id,
    pub user: Id,
    pub session: Id,
    pub algorithm: String,
    pub public_key: Vec<u8>,
    pub label: String,
    pub created_at: Timestamp,
    pub last_seen_at: Timestamp,
    /// When the session the device is bound to was last seen.
    pub session_last_seen_at: Timestamp,
}

impl Device {
    fn read(row: &Row<'_>) -> rusqlite::Result<Device> {
        Ok(Device {
            id: row.get(0)?,
            user: row.get(1)?,
            session: row.get(2)?,
            algorithm: row.get(3)?,
            public_key: row.get(4)?,
            label: row.get(5)?,
            created_at: row.get(6)?,
            last_seen_at: row.get(7)?,
            session_last_seen_at: row.get(8)?,
        })
    }

    /// The device as its owner sees it: in use whenever its session is.
    pub fn own_dto(&self, current_session: Option<Id>) -> OwnDeviceDto {
        OwnDeviceDto {
            id: self.id,
            algorithm: self.algorithm.clone(),
            public_key: Octets(self.public_key.clone()),
            label: self.label.clone(),
            created_at: self.created_at,
            last_seen_at: self.last_seen_at.max(self.session_last_seen_at),
            current: Some(self.session) == current_session,
        }
    }

    /// The device as the other person sees it: no label, nothing of the session.
    pub fn key_dto(&self) -> DeviceKeyDto {
        DeviceKeyDto {
            key_id: self.id,
            algorithm: self.algorithm.clone(),
            public_key: Octets(self.public_key.clone()),
            created_at: self.created_at,
        }
    }
}

/// The devices of an account that take part, oldest first.
pub fn own_devices(
    connection: &Connection,
    user: Id,
    now: Timestamp,
) -> rusqlite::Result<Vec<Device>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {DEVICE_COLUMNS} FROM {DEVICES} WHERE {ACTIVE} AND d.\"UserId\" = :user ORDER BY d.\"CreatedAt\""
    ))?;
    statement
        .query_map(named_params! { ":now": now, ":user": user }, Device::read)?
        .collect()
}

/// The device of a login session, if it has one that takes part.
pub fn current_device(
    connection: &Connection,
    session: Id,
    now: Timestamp,
) -> rusqlite::Result<Option<Device>> {
    connection
        .query_row(
            &format!("SELECT {DEVICE_COLUMNS} FROM {DEVICES} WHERE {ACTIVE} AND d.\"SessionId\" = :session"),
            named_params! { ":now": now, ":session": session },
            Device::read,
        )
        .optional()
}

/// One of the account's devices that takes part, by its id.
pub fn active_device(
    connection: &Connection,
    user: Id,
    device: Id,
    now: Timestamp,
) -> rusqlite::Result<Option<Device>> {
    connection
        .query_row(
            &format!(
                "SELECT {DEVICE_COLUMNS} FROM {DEVICES} WHERE {ACTIVE} AND d.\"Id\" = :device AND d.\"UserId\" = :user"
            ),
            named_params! { ":now": now, ":device": device, ":user": user },
            Device::read,
        )
        .optional()
}

/// How many devices of the account take part.
pub fn count_active(connection: &Connection, user: Id, now: Timestamp) -> rusqlite::Result<i64> {
    connection.query_row(
        &format!("SELECT COUNT(*) FROM {DEVICES} WHERE {ACTIVE} AND d.\"UserId\" = :user"),
        named_params! { ":now": now, ":user": user },
        |row| row.get(0),
    )
}

/// The device that this session registered and has not lost, whether or not
/// the session still signs in.
pub fn session_device(connection: &Connection, session: Id) -> rusqlite::Result<Option<Device>> {
    connection
        .query_row(
            &format!(
                "SELECT {DEVICE_COLUMNS} FROM {DEVICES} WHERE d.\"SessionId\" = ?1 AND d.\"RevokedAt\" IS NULL"
            ),
            [session],
            Device::read,
        )
        .optional()
}

/// The devices of two people that take part, oldest first.
pub fn devices_of_two(
    connection: &Connection,
    first: Id,
    second: Id,
    now: Timestamp,
) -> rusqlite::Result<Vec<Device>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {DEVICE_COLUMNS} FROM {DEVICES} WHERE {ACTIVE} AND (d.\"UserId\" = :first OR d.\"UserId\" = :second)
         ORDER BY d.\"CreatedAt\", d.\"Id\""
    ))?;
    statement
        .query_map(
            named_params! { ":now": now, ":first": first, ":second": second },
            Device::read,
        )?
        .collect()
}

// ---- conversations ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Off,
    Requested,
    On,
}

impl State {
    fn stored(text: &str) -> State {
        match text {
            "Requested" => State::Requested,
            "On" => State::On,
            _ => State::Off,
        }
    }

    pub fn to_stored(self) -> &'static str {
        match self {
            State::Off => "Off",
            State::Requested => "Requested",
            State::On => "On",
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            State::Off => "off",
            State::Requested => "requested",
            State::On => "on",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Conversation {
    pub id: Id,
    pub first: Id,
    pub second: Id,
    pub state: State,
    pub requested_by: Option<Id>,
    pub requested_at: Option<Timestamp>,
    pub enabled_at: Option<Timestamp>,
    pub current_epoch: Option<Id>,
}

const CONVERSATION_COLUMNS: &str = "\"Id\", \"UserAId\", \"UserBId\", \"EncryptionState\", \"EncryptionRequestedById\",
    \"EncryptionRequestedAt\", \"EncryptionEnabledAt\", \"CurrentEpochId\"";

impl Conversation {
    fn read(row: &Row<'_>) -> rusqlite::Result<Conversation> {
        Ok(Conversation {
            id: row.get(0)?,
            first: row.get(1)?,
            second: row.get(2)?,
            state: State::stored(&row.get::<_, String>(3)?),
            requested_by: row.get(4)?,
            requested_at: row.get(5)?,
            enabled_at: row.get(6)?,
            current_epoch: row.get(7)?,
        })
    }

    pub fn peer_of(&self, user: Id) -> Id {
        if self.first == user {
            self.second
        } else {
            self.first
        }
    }

    pub fn dto(&self) -> EncryptionDto {
        EncryptionDto {
            state: self.state.wire(),
            requested_by_id: self.requested_by,
            requested_at: self.requested_at,
            enabled_at: self.enabled_at,
            current_epoch_id: self.current_epoch,
        }
    }

    /// Writes the state and what goes with it back.
    pub fn save(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection.execute(
            "UPDATE \"DirectConversations\" SET \"EncryptionState\" = ?2, \"EncryptionRequestedById\" = ?3,
                \"EncryptionRequestedAt\" = ?4, \"EncryptionEnabledAt\" = ?5, \"CurrentEpochId\" = ?6
             WHERE \"Id\" = ?1",
            params![
                self.id,
                self.state.to_stored(),
                self.requested_by,
                self.requested_at,
                self.enabled_at,
                self.current_epoch
            ],
        )?;
        Ok(())
    }
}

/// The conversation, when the person is one of its two.
pub fn conversation_of(
    connection: &Connection,
    conversation: Id,
    user: Id,
) -> rusqlite::Result<Option<Conversation>> {
    connection
        .query_row(
            &format!(
                "SELECT {CONVERSATION_COLUMNS} FROM \"DirectConversations\"
                 WHERE \"Id\" = ?1 AND (\"UserAId\" = ?2 OR \"UserBId\" = ?2)"
            ),
            params![conversation, user],
            Conversation::read,
        )
        .optional()
}

pub fn conversation(
    connection: &Connection,
    conversation: Id,
) -> rusqlite::Result<Option<Conversation>> {
    connection
        .query_row(
            &format!("SELECT {CONVERSATION_COLUMNS} FROM \"DirectConversations\" WHERE \"Id\" = ?1"),
            [conversation],
            Conversation::read,
        )
        .optional()
}

/// The conversations with encryption on that include the person.
pub fn encrypted_conversations_of(
    connection: &Connection,
    user: Id,
) -> rusqlite::Result<Vec<Conversation>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {CONVERSATION_COLUMNS} FROM \"DirectConversations\"
         WHERE (\"UserAId\" = ?1 OR \"UserBId\" = ?1) AND \"EncryptionState\" = 'On'"
    ))?;
    statement.query_map([user], Conversation::read)?.collect()
}

/// Whether the peer accepts a message, and so a request, from this person.
/// Somebody who turned direct messages off still hears from their friends.
pub fn peer_takes_messages_from(
    connection: &Connection,
    sender: Id,
    peer: Id,
) -> rusqlite::Result<bool> {
    let allowed: Option<bool> = connection
        .query_row(
            "SELECT \"AllowDirectMessages\" FROM \"UserPreferences\" WHERE \"UserId\" = ?1",
            [peer],
            |row| row.get(0),
        )
        .optional()?;
    if allowed != Some(false) {
        return Ok(true);
    }
    crate::social::are_friends(connection, sender, peer)
}

// ---- epochs ----

/// The keys an epoch has to be wrapped for: a device or an account key of one
/// of the two people.
#[derive(Debug, Clone)]
pub struct Recipient {
    pub key_id: Id,
    pub kind: &'static str,
    pub user: Id,
    pub algorithm: String,
}

/// Exactly the keys a new epoch of a conversation between the two people
/// must be wrapped for: both account keys and every device of both that takes
/// part.
pub fn recipients(
    connection: &Connection,
    first: Id,
    second: Id,
    now: Timestamp,
) -> rusqlite::Result<Vec<Recipient>> {
    let mut statement = connection.prepare(
        "SELECT \"KeyId\", \"UserId\", \"Algorithm\" FROM \"AccountKeys\" WHERE \"UserId\" = ?1 OR \"UserId\" = ?2",
    )?;
    let mut found: Vec<Recipient> = statement
        .query_map(params![first, second], |row| {
            Ok(Recipient {
                key_id: row.get(0)?,
                kind: ACCOUNT,
                user: row.get(1)?,
                algorithm: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut statement = connection.prepare(&format!(
        "SELECT d.\"Id\", d.\"UserId\", d.\"Algorithm\" FROM {DEVICES}
         WHERE {ACTIVE} AND (d.\"UserId\" = :first OR d.\"UserId\" = :second)"
    ))?;
    let devices = statement.query_map(
        named_params! { ":now": now, ":first": first, ":second": second },
        |row| {
            Ok(Recipient {
                key_id: row.get(0)?,
                kind: DEVICE,
                user: row.get(1)?,
                algorithm: row.get(2)?,
            })
        },
    )?;
    for device in devices {
        found.push(device?);
    }
    Ok(found)
}

/// What a key of an epoch holds.
#[derive(Debug, Clone)]
pub struct Envelope {
    pub recipient_key_id: Id,
    pub recipient_user: Id,
    pub kind: String,
    pub ephemeral_public_key: Vec<u8>,
    pub iv: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

impl Envelope {
    fn dto(&self) -> EnvelopeDto {
        EnvelopeDto {
            recipient_key_id: self.recipient_key_id,
            ephemeral_public_key: Octets(self.ephemeral_public_key.clone()),
            iv: Octets(self.iv.clone()),
            ciphertext: Octets(self.ciphertext.clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Epoch {
    pub id: Id,
    pub conversation: Id,
    pub sequence: i32,
    pub previous: Option<Id>,
    pub created_by: Id,
    pub created_at: Timestamp,
    pub envelopes: Vec<Envelope>,
}

impl Epoch {
    /// The epoch with the envelopes `keep` lets through: the reader's own.
    /// Who it was made for is told for all of them.
    pub fn dto(&self, keep: impl Fn(&Envelope) -> bool) -> EpochDto {
        let mut recipients: Vec<RecipientKeyDto> = self
            .envelopes
            .iter()
            .map(|envelope| RecipientKeyDto {
                key_id: envelope.recipient_key_id,
                kind: if envelope.kind == ACCOUNT { ACCOUNT } else { DEVICE },
                user_id: envelope.recipient_user,
            })
            .collect();
        recipients.sort_by(|a, b| {
            (a.user_id.to_string(), a.kind, a.key_id.to_string()).cmp(&(
                b.user_id.to_string(),
                b.kind,
                b.key_id.to_string(),
            ))
        });
        EpochDto {
            id: self.id,
            sequence: self.sequence,
            previous_epoch_id: self.previous,
            created_by_id: self.created_by,
            created_at: self.created_at,
            recipients,
            envelopes: self
                .envelopes
                .iter()
                .filter(|envelope| keep(envelope))
                .map(Envelope::dto)
                .collect(),
        }
    }
}

fn read_epoch(row: &Row<'_>) -> rusqlite::Result<Epoch> {
    Ok(Epoch {
        id: row.get(0)?,
        conversation: row.get(1)?,
        sequence: row.get(2)?,
        previous: row.get(3)?,
        created_by: row.get(4)?,
        created_at: row.get(5)?,
        envelopes: Vec::new(),
    })
}

const EPOCH_COLUMNS: &str =
    "\"Id\", \"ConversationId\", \"Sequence\", \"PreviousEpochId\", \"CreatedById\", \"CreatedAt\"";

/// The envelopes of an epoch in the order the rows are keyed.
fn envelopes_of(connection: &Connection, epoch: Id) -> rusqlite::Result<Vec<Envelope>> {
    let mut statement = connection.prepare(
        "SELECT \"RecipientKeyId\", \"RecipientUserId\", \"RecipientKind\", \"EphemeralPublicKey\", \"Iv\", \"Ciphertext\"
         FROM \"EncryptionEnvelopes\" WHERE \"EpochId\" = ?1 ORDER BY \"RecipientKeyId\"",
    )?;
    statement
        .query_map([epoch], |row| {
            Ok(Envelope {
                recipient_key_id: row.get(0)?,
                recipient_user: row.get(1)?,
                kind: row.get(2)?,
                ephemeral_public_key: row.get(3)?,
                iv: row.get(4)?,
                ciphertext: row.get(5)?,
            })
        })?
        .collect()
}

/// The epoch with this id, whichever conversation it is of.
pub fn epoch(connection: &Connection, id: Id) -> rusqlite::Result<Option<Epoch>> {
    let found = connection
        .query_row(
            &format!("SELECT {EPOCH_COLUMNS} FROM \"EncryptionEpochs\" WHERE \"Id\" = ?1"),
            [id],
            read_epoch,
        )
        .optional()?;
    found
        .map(|mut epoch| {
            epoch.envelopes = envelopes_of(connection, epoch.id)?;
            Ok(epoch)
        })
        .transpose()
}

/// Up to `limit` epochs of a conversation after a sequence number, oldest first.
pub fn epochs_after(
    connection: &Connection,
    conversation: Id,
    after: i64,
    limit: i64,
) -> rusqlite::Result<Vec<Epoch>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {EPOCH_COLUMNS} FROM \"EncryptionEpochs\"
         WHERE \"ConversationId\" = ?1 AND \"Sequence\" > ?2 ORDER BY \"Sequence\" LIMIT ?3"
    ))?;
    let mut epochs: Vec<Epoch> = statement
        .query_map(params![conversation, after, limit], read_epoch)?
        .collect::<rusqlite::Result<_>>()?;
    for epoch in &mut epochs {
        epoch.envelopes = envelopes_of(connection, epoch.id)?;
    }
    Ok(epochs)
}

/// The highest sequence number of a conversation's epochs, or 0.
pub fn last_sequence(connection: &Connection, conversation: Id) -> rusqlite::Result<i32> {
    connection.query_row(
        "SELECT COALESCE(MAX(\"Sequence\"), 0) FROM \"EncryptionEpochs\" WHERE \"ConversationId\" = ?1",
        [conversation],
        |row| row.get(0),
    )
}

pub fn insert_epoch(connection: &Connection, epoch: &Epoch) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO \"EncryptionEpochs\" (\"Id\", \"ConversationId\", \"Sequence\", \"PreviousEpochId\", \"CreatedById\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            epoch.id,
            epoch.conversation,
            epoch.sequence,
            epoch.previous,
            epoch.created_by,
            epoch.created_at
        ],
    )?;
    for envelope in &epoch.envelopes {
        connection.execute(
            "INSERT INTO \"EncryptionEnvelopes\" (\"EpochId\", \"RecipientKeyId\", \"RecipientUserId\", \"RecipientKind\",
                                                 \"EphemeralPublicKey\", \"Iv\", \"Ciphertext\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                epoch.id,
                envelope.recipient_key_id,
                envelope.recipient_user,
                envelope.kind,
                envelope.ephemeral_public_key,
                envelope.iv,
                envelope.ciphertext
            ],
        )?;
    }
    Ok(())
}
