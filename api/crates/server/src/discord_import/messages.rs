//! Writing a channel's history, a page at a time.
//!
//! Each message is written with its author's placeholder, made on first sight,
//! and Discord's times. Replies and threads are only recorded here: the reader
//! goes back in time, so a reply comes before what it answers, and
//! [`ImportWriter::complete_channel`] links them once the history is whole.
//!
//! What Stellarr has no shape for is recorded and left out, so that it is
//! counted once however often a page is read: notices, forwards, a sticker or
//! embeds alone, an empty message. A message of nothing but attachments is
//! recorded for the attachment stage, which writes it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use stellarr_discord::{AuthorSnapshot, ImportResolver, MessageSnapshot, ServerSnapshot, rewrite};

use super::journal::{self, json_texts};
use super::structure::ChannelRecord;
use super::{
    Import, ImportError, ImportWriter, MESSAGES_PER_TRANSACTION, Outcome, PageResult, RecordKind,
    SkipReason, moment, names, require_same_server, require_server,
};
use crate::link_previews;
use crate::messages::MAX_CONTENT_UNITS;
use crate::uploads::Attachment;
use crate::values::{Id, Timestamp};

/// How a role reads in a message: its name, with the at sign it is mentioned by.
fn role_labels(snapshot: &ServerSnapshot) -> HashMap<String, String> {
    let mut roles = HashMap::new();
    for role in snapshot.roles.iter().filter(|role| !role.name.is_empty()) {
        roles.entry(role.id.clone()).or_insert_with(|| {
            if role.name.starts_with('@') {
                role.name.clone()
            } else {
                format!("@{}", role.name)
            }
        });
    }
    roles
}

/// The names the wanted people showed on the server: one pass over the members,
/// which a server can have a hundred thousand of, for as many pages as it has.
fn member_names(snapshot: &ServerSnapshot, wanted: &HashSet<String>) -> HashMap<String, String> {
    let mut found = HashMap::new();
    if wanted.is_empty() {
        return found;
    }
    for member in &snapshot.members {
        if wanted.contains(&member.id) && !found.contains_key(&member.id) {
            found.insert(
                member.id.clone(),
                names::display_name(
                    &member.id,
                    &[
                        member.nickname.as_deref(),
                        member.display_name.as_deref(),
                        Some(&member.username),
                    ],
                ),
            );
        }
    }
    found
}

/// The ids of the people a chunk may show by name: its authors, and whoever its
/// messages mention.
fn wanted_members(
    messages: &[MessageSnapshot],
    channel: Option<(Id, bool)>,
    roles: &HashMap<String, String>,
) -> HashSet<String> {
    let recording = ImportResolver::recording(roles.clone());
    let mut wanted = HashSet::new();
    for message in messages {
        if classify(message, channel) != SkipReason::None {
            continue;
        }
        if message.author.webhook_id.is_none() {
            wanted.insert(message.author.id.clone());
        }
        rewrite(message.content.trim(), &recording);
    }
    wanted.extend(recording.asked_users());
    wanted
}

/// The counts one page adds up to.
#[derive(Default)]
struct Counts {
    written: usize,
    already_seen: usize,
    attachments_pending: usize,
    awaiting_attachments: usize,
    placeholders_made: usize,
    skipped: BTreeMap<SkipReason, usize>,
    notices: BTreeMap<i32, usize>,
}

impl Counts {
    fn count(&mut self, outcome: Outcome, reason: SkipReason, message: &MessageSnapshot) {
        match outcome {
            Outcome::Written => {
                self.written += 1;
                self.attachments_pending += usize::from(!message.attachments.is_empty());
            }
            Outcome::AwaitingAttachments => self.awaiting_attachments += 1,
            Outcome::Skipped => {
                *self.skipped.entry(reason).or_default() += 1;
                if reason == SkipReason::Notice {
                    *self.notices.entry(message.kind).or_default() += 1;
                }
            }
        }
    }

    fn add(&mut self, other: Counts) {
        self.written += other.written;
        self.already_seen += other.already_seen;
        self.attachments_pending += other.attachments_pending;
        self.awaiting_attachments += other.awaiting_attachments;
        self.placeholders_made += other.placeholders_made;
        for (reason, count) in other.skipped {
            *self.skipped.entry(reason).or_default() += count;
        }
        for (kind, count) in other.notices {
            *self.notices.entry(kind).or_default() += count;
        }
    }
}

/// Who the placeholder stands for: the person, or the webhook that posted under
/// a name of its choice.
fn author_key(author: &AuthorSnapshot) -> &str {
    author.webhook_id.as_deref().unwrap_or(&author.id)
}

/// A placeholder that exists: its account and the name it shows.
struct Placeholder {
    user: Id,
    display_name: String,
}

/// What a chunk of messages is written from.
struct Chunk {
    import: Id,
    server: Id,
    source: String,
    channel: Option<(Id, bool)>,
    messages: Vec<MessageSnapshot>,
    pinned_at: HashMap<String, Timestamp>,
    roles: Arc<HashMap<String, String>>,
    members: HashMap<String, String>,
    now: Timestamp,
}

impl ImportWriter {
    pub(super) async fn attach(
        &self,
        import: &Import,
        channel: Id,
        message: &MessageSnapshot,
        attachment: Attachment,
    ) -> Result<bool, ImportError> {
        let server = require_server(import)?;
        let actor = import
            .started_by
            .ok_or_else(|| ImportError::State("Missing import owner.".into()))?;
        let now = Timestamp::now();
        let original = message.id.clone();
        let created = moment(message.created_at);
        let edited = message.edited_at.map(moment);
        let pinned = message.pinned.then_some(created);
        let chunk = Chunk {
            import: import.id,
            server,
            source: message.channel_id.clone(),
            channel: Some((channel, true)),
            messages: Vec::new(),
            pinned_at: HashMap::new(),
            roles: Arc::default(),
            members: HashMap::new(),
            now,
        };
        let author = message.author.clone();
        self.pause().await;
        let stored = self.database.write(move |tx| -> Result<bool, ImportError> {
            let current: Option<(Outcome, Option<Id>, Timestamp)> = tx.query_row(
                "SELECT d.\"Outcome\", d.\"MessageId\", d.\"CreatedAt\"
                 FROM \"DiscordImportMessages\" d
                 JOIN \"DiscordImportChannels\" c ON c.\"ServerId\" = d.\"ServerId\" AND c.\"DiscordId\" = d.\"ChannelDiscordId\"
                 JOIN \"ServerMembers\" m ON m.\"ServerId\" = d.\"ServerId\" AND m.\"UserId\" = ?5 AND m.\"OwnerSlot\" IS NOT NULL
                 WHERE d.\"ImportId\" = ?1 AND d.\"DiscordId\" = ?2 AND d.\"ChannelDiscordId\" = ?3
                   AND c.\"ChannelId\" = ?4 AND d.\"AttachmentsPending\" = 1",
                params![chunk.import, original, chunk.source, channel, actor],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional()?;
            let root = match current {
                Some((Outcome::Written, Some(id), _)) => id,
                Some((Outcome::AwaitingAttachments, None, original_time)) if !author_key(&author).is_empty() => {
                    let id = names::message_id(server, &original);
                    if taken_ids(tx, vec![id])?.contains(&id) {
                        return Ok(false);
                    }
                    let key = author_key(&author);
                    let mut accounts = placeholders_of(tx, vec![key.to_owned()])?;
                    make_placeholders(tx, &chunk, &[&author], &mut accounts, &mut Counts::default())?;
                    tx.execute(
                        "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\", \"EditedAt\", \"PinnedAt\")
                         VALUES (?1, ?2, ?3, 0, '', ?4, ?5, ?6)",
                        params![id, channel, accounts[key].user, original_time, edited, pinned],
                    )?;
                    journal::forget_post_times(tx, channel, &[accounts[key].user])?;
                    tx.execute(
                        "UPDATE \"DiscordImportMessages\" SET \"MessageId\" = ?3, \"Outcome\" = ?4
                         WHERE \"ImportId\" = ?1 AND \"DiscordId\" = ?2",
                        params![chunk.import, original, id, Outcome::Written],
                    )?;
                    id
                }
                _ => return Ok(false),
            };
            attachment.insert(tx)?;
            tx.execute(
                "INSERT INTO \"MessageAttachments\" (\"MessageId\", \"AttachmentId\") VALUES (?1, ?2)",
                params![root, attachment.id],
            )?;
            journal::reset(tx, channel, &[root], now)?;
            Ok(true)
        }).await?;
        if stored {
            self.committed();
        }
        Ok(stored)
    }

    /// Writes one page of a channel's or a thread's history, as the reader gives
    /// it: newest first, and each page older than the one before.
    ///
    /// `pinned_at` is when each pinned message was pinned, from the reader's
    /// pins, which a message itself does not say. A pinned message missing from
    /// it counts as pinned when it was sent.
    pub async fn write_messages(
        &self,
        import: &Import,
        snapshot: &ServerSnapshot,
        discord_channel_id: &str,
        page: &[MessageSnapshot],
        pinned_at: Option<&HashMap<String, stellarr_discord::Timestamp>>,
    ) -> Result<PageResult, ImportError> {
        let server = require_server(import)?;
        require_same_server(import, snapshot)?;
        if let Some(stray) = page.iter().find(|m| m.channel_id != discord_channel_id) {
            return Err(ImportError::Argument(format!(
                "Message {} was read from another channel.",
                stray.id
            )));
        }

        let wanted = discord_channel_id.to_string();
        let (source, channel) = self
            .database
            .read(
                move |db| -> Result<(ChannelRecord, Option<(Id, bool)>), ImportError> {
                    let source = ChannelRecord::find(db, server, &wanted)?.ok_or_else(|| {
                        ImportError::Argument(
                            "The import has not recorded this channel or thread.".to_string(),
                        )
                    })?;
                    let channel = match source.channel {
                        Some(id) => db
                            .query_row(
                                "SELECT \"Kind\" FROM \"Channels\" WHERE \"Id\" = ?1",
                                [id],
                                |row| row.get::<_, i64>(0),
                            )
                            .map(|kind| Some((id, kind == 0)))
                            .or_else(|error| match error {
                                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                                other => Err(other),
                            })?,
                        None => None,
                    };
                    Ok((source, channel))
                },
            )
            .await?;
        let roles = Arc::new(role_labels(snapshot));

        let mut counts = Counts::default();
        for chunk in page.chunks(MESSAGES_PER_TRANSACTION) {
            self.pause().await;
            let pinned: HashMap<String, Timestamp> = pinned_at
                .map(|map| {
                    chunk
                        .iter()
                        .filter_map(|m| map.get(&m.id).map(|at| (m.id.clone(), moment(*at))))
                        .collect()
                })
                .unwrap_or_default();
            let input = Chunk {
                import: import.id,
                server,
                source: source.discord_id.clone(),
                channel,
                messages: chunk.to_vec(),
                pinned_at: pinned,
                members: member_names(snapshot, &wanted_members(chunk, channel, &roles)),
                roles: Arc::clone(&roles),
                now: Timestamp::now(),
            };
            let done = self
                .database
                .write(move |tx| write_chunk(tx, &input))
                .await?;
            self.committed();
            counts.add(done);
        }
        Ok(PageResult {
            written: counts.written,
            already_seen: counts.already_seen,
            attachments_pending: counts.attachments_pending,
            awaiting_attachments: counts.awaiting_attachments,
            placeholders_made: counts.placeholders_made,
            skipped: counts.skipped,
            notices_by_type: counts.notices,
        })
    }
}

/// Why a message cannot be written whatever its text says, or none.
fn classify(message: &MessageSnapshot, channel: Option<(Id, bool)>) -> SkipReason {
    match channel {
        None => SkipReason::ChannelRemoved,
        Some((_, false)) => SkipReason::VoiceChannel,
        Some(_) if message.is_system_notice => SkipReason::Notice,
        Some(_) if author_key(&message.author).is_empty() => SkipReason::NoAuthor,
        Some(_) if message.forwarded.is_some() => SkipReason::Forward,
        Some(_) => SkipReason::None,
    }
}

/// The name an author showed on the server: a member's nickname or name from the
/// snapshot, else what the message itself says. A webhook shows the name it gave
/// the message.
fn placeholder_name(author: &AuthorSnapshot, members: &HashMap<String, String>) -> String {
    if author.webhook_id.is_none()
        && let Some(member) = members.get(&author.id)
    {
        return member.clone();
    }
    names::display_name(
        author_key(author),
        &[author.display_name.as_deref(), Some(&author.username)],
    )
}

fn write_chunk(tx: &rusqlite::Transaction<'_>, input: &Chunk) -> Result<Counts, ImportError> {
    let mut counts = Counts::default();

    let mut distinct: Vec<&MessageSnapshot> = Vec::new();
    {
        let mut ids = HashSet::new();
        for message in &input.messages {
            if ids.insert(message.id.as_str()) {
                distinct.push(message);
            }
        }
    }
    let seen = known_messages(tx, input.server, &distinct)?;
    let fresh: Vec<&MessageSnapshot> = distinct
        .iter()
        .copied()
        .filter(|m| !seen.contains(&m.id))
        .collect();
    counts.already_seen += input.messages.len() - fresh.len();

    // What can be written at all, before any text is read.
    let plans: Vec<(&MessageSnapshot, SkipReason)> = fresh
        .iter()
        .map(|message| (*message, classify(message, input.channel)))
        .collect();
    let candidates: Vec<&MessageSnapshot> = plans
        .iter()
        .filter(|(_, reason)| *reason == SkipReason::None)
        .map(|(message, _)| *message)
        .collect();
    let derived: HashMap<&str, Id> = candidates
        .iter()
        .map(|m| (m.id.as_str(), names::message_id(input.server, &m.id)))
        .collect();
    let taken = taken_ids(tx, derived.values().copied().collect())?;

    // The rewriter asks about the people and channels it meets; ask once
    // without answers to learn which, then load them.
    let recording = ImportResolver::recording(input.roles.as_ref().clone());
    for message in &candidates {
        rewrite(message.content.trim(), &recording);
    }
    let mut authors: Vec<(&str, &AuthorSnapshot)> = Vec::new();
    {
        let mut keys = HashSet::new();
        for message in &candidates {
            let key = author_key(&message.author);
            if keys.insert(key) {
                authors.push((key, &message.author));
            }
        }
    }
    let asked_users = recording.asked_users();
    let mut accounts = placeholders_of(
        tx,
        authors
            .iter()
            .map(|(key, _)| (*key).to_string())
            .chain(asked_users.iter().cloned())
            .collect(),
    )?;
    let author_by_key: HashMap<&str, &AuthorSnapshot> = authors.iter().copied().collect();
    let mut user_names: HashMap<String, String> = HashMap::new();
    for mentioned in candidates.iter().flat_map(|message| &message.mentions) {
        if !asked_users.contains(&mentioned.id) {
            continue;
        }
        let label = names::display_name(
            "",
            &[
                mentioned.nickname.as_deref(),
                input.members.get(&mentioned.id).map(String::as_str),
                mentioned.display_name.as_deref(),
                mentioned.username.as_deref(),
            ],
        );
        if !label.is_empty() {
            user_names.entry(mentioned.id.clone()).or_insert(label);
        }
    }
    for id in &asked_users {
        if user_names.contains_key(id) {
            continue;
        }
        if let Some(account) = accounts.get(id) {
            user_names.insert(id.clone(), account.display_name.clone());
        } else if let Some(author) = author_by_key.get(id.as_str()) {
            user_names.insert(id.clone(), placeholder_name(author, &input.members));
        } else if let Some(member) = input.members.get(id) {
            user_names.insert(id.clone(), member.clone());
        }
    }
    let resolver = ImportResolver::new(
        user_names,
        channel_labels(tx, input.server, &recording.asked_channels())?,
        input.roles.as_ref().clone(),
    );

    let mut written: Vec<(&MessageSnapshot, Id, String)> = Vec::new();
    let mut bookkeeping: Vec<(&MessageSnapshot, Outcome, SkipReason, Option<Id>)> = Vec::new();
    for (message, planned) in &plans {
        let mut reason = *planned;
        let mut outcome = Outcome::Skipped;
        let mut message_id = None;
        if reason == SkipReason::None {
            let content = rewrite(message.content.trim(), &resolver);
            if content.is_empty() {
                (outcome, reason) = if !message.attachments.is_empty() {
                    (Outcome::AwaitingAttachments, SkipReason::None)
                } else if !message.stickers.is_empty() {
                    (Outcome::Skipped, SkipReason::OnlySticker)
                } else if !message.embeds.is_empty() {
                    (Outcome::Skipped, SkipReason::OnlyEmbeds)
                } else {
                    (Outcome::Skipped, SkipReason::Empty)
                };
            } else if names::units(&content) > MAX_CONTENT_UNITS {
                reason = SkipReason::TooLong;
            } else if taken.contains(&derived[message.id.as_str()]) {
                reason = SkipReason::IdTaken;
            } else {
                outcome = Outcome::Written;
                message_id = Some(derived[message.id.as_str()]);
                written.push((message, derived[message.id.as_str()], content));
            }
        }
        counts.count(outcome, reason, message);
        bookkeeping.push((message, outcome, reason, message_id));
    }

    // Only an author of something written gets a placeholder.
    let mut makers: Vec<&AuthorSnapshot> = Vec::new();
    {
        let mut keys = HashSet::new();
        for (message, _, _) in &written {
            let key = author_key(&message.author);
            if keys.insert(key) {
                makers.push(author_by_key[key]);
            }
        }
    }
    make_placeholders(tx, input, &makers, &mut accounts, &mut counts)?;

    let channel = input.channel.map(|(id, _)| id);
    for (message, id, content) in &written {
        let pinned = match input.pinned_at.get(&message.id) {
            Some(at) => Some(*at),
            None if message.pinned => Some(moment(message.created_at)),
            None => None,
        };
        tx.execute(
            "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\", \"EditedAt\", \"PinnedAt\", \"ReplyToMessageId\")
             VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, NULL)",
            params![
                id,
                channel,
                accounts[author_key(&message.author)].user,
                content,
                moment(message.created_at),
                message.edited_at.map(moment),
                pinned
            ],
        )?;
        link_previews::remember_imported(tx, *id, content, &message.embeds)?;
    }
    for (message, outcome, reason, message_id) in &bookkeeping {
        tx.execute(
            "INSERT INTO \"DiscordImportMessages\" (\"ServerId\", \"DiscordId\", \"ImportId\", \"ChannelDiscordId\", \"Outcome\", \"Reason\", \"DiscordType\",
                 \"CreatedAt\", \"MessageId\", \"ReplyToDiscordId\", \"AttachmentsPending\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                input.server,
                message.id,
                input.import,
                input.source,
                outcome,
                reason,
                message.kind,
                moment(message.created_at),
                message_id,
                message.reply_to_message_id,
                !message.attachments.is_empty()
            ],
        )?;
    }
    let oldest: Option<String> = tx.query_row(
        "SELECT \"OldestMessageId\" FROM \"DiscordImportChannels\" WHERE \"ServerId\" = ?1 AND \"DiscordId\" = ?2",
        params![input.server, input.source],
        |row| row.get(0),
    )?;
    let oldest = distinct.iter().fold(oldest, |held, message| {
        Some(names::earlier(held, &message.id))
    });
    tx.execute(
        "UPDATE \"DiscordImportChannels\" SET \"OldestMessageId\" = ?3 WHERE \"ServerId\" = ?1 AND \"DiscordId\" = ?2",
        params![input.server, input.source, oldest],
    )?;

    if let (false, Some(channel)) = (written.is_empty(), channel) {
        let ids: Vec<Id> = written.iter().map(|(_, id, _)| *id).collect();
        journal::reset(tx, channel, &ids, input.now)?;
        let mut authors_written: Vec<Id> = Vec::new();
        for (message, _, _) in &written {
            let user = accounts[author_key(&message.author)].user;
            if !authors_written.contains(&user) {
                authors_written.push(user);
            }
        }
        journal::forget_post_times(tx, channel, &authors_written)?;
        if let Some(newest) = written.iter().map(|(m, _, _)| moment(m.created_at)).max() {
            journal::mark_read(tx, channel, newest)?;
        }
    }
    Ok(counts)
}

/// The ids among these messages that the bookkeeping already has.
fn known_messages(
    db: &Connection,
    server: Id,
    messages: &[&MessageSnapshot],
) -> rusqlite::Result<HashSet<String>> {
    let mut statement = db.prepare_cached(
        "SELECT \"DiscordId\" FROM \"DiscordImportMessages\" WHERE \"ServerId\" = ?1
           AND \"DiscordId\" IN (SELECT value FROM json_each(?2))",
    )?;
    statement
        .query_map(
            params![server, json_texts(messages.iter().map(|m| m.id.as_str()))],
            |row| row.get(0),
        )?
        .collect()
}

/// The ids among these that are a message's already.
fn taken_ids(db: &Connection, ids: Vec<Id>) -> rusqlite::Result<HashSet<Id>> {
    let mut statement = db.prepare_cached(
        "SELECT \"Id\" FROM \"Messages\" WHERE \"Id\" IN (SELECT value FROM json_each(?1))",
    )?;
    statement
        .query_map([journal::json_ids(&ids)], |row| row.get(0))?
        .collect()
}

/// The placeholders that already exist among these Discord ids.
fn placeholders_of(
    db: &Connection,
    discord_ids: Vec<String>,
) -> rusqlite::Result<HashMap<String, Placeholder>> {
    let mut keys: Vec<String> = Vec::new();
    {
        let mut seen = HashSet::new();
        for id in discord_ids {
            if seen.insert(id.clone()) {
                keys.push(id);
            }
        }
    }
    let mut statement = db.prepare_cached(
        "SELECT a.\"DiscordId\", a.\"UserId\", u.\"DisplayName\" FROM \"DiscordImportAccounts\" a
           JOIN \"Users\" u ON u.\"Id\" = a.\"UserId\"
          WHERE a.\"DiscordId\" IN (SELECT value FROM json_each(?1))",
    )?;
    let rows = statement.query_map([json_texts(keys.iter().map(String::as_str))], |row| {
        Ok((
            row.get::<_, String>(0)?,
            Placeholder {
                user: row.get(1)?,
                display_name: row.get(2)?,
            },
        ))
    })?;
    rows.collect()
}

/// Makes the placeholders these authors lack and adds them to `accounts`. A
/// mention makes none: only an author has one.
fn make_placeholders(
    tx: &rusqlite::Transaction<'_>,
    input: &Chunk,
    authors: &[&AuthorSnapshot],
    accounts: &mut HashMap<String, Placeholder>,
    counts: &mut Counts,
) -> Result<(), ImportError> {
    let missing: Vec<&&AuthorSnapshot> = authors
        .iter()
        .filter(|author| !accounts.contains_key(author_key(author)))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let usernames: Vec<String> = missing
        .iter()
        .map(|author| stellarr_unicode::to_upper(&names::placeholder_username(author_key(author))))
        .collect();
    let existing: HashMap<String, (Id, String, bool)> = {
        let mut statement = tx.prepare_cached(
            "SELECT \"NormalizedUsername\", \"Id\", \"DisplayName\", \"Origin\" IS NOT NULL FROM \"Users\"
              WHERE \"NormalizedUsername\" IN (SELECT value FROM json_each(?1))",
        )?;
        let rows = statement
            .query_map([json_texts(usernames.iter().map(String::as_str))], |row| {
                Ok((row.get(0)?, (row.get(1)?, row.get(2)?, row.get(3)?)))
            })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for author in missing {
        let key = author_key(author);
        let username = names::placeholder_username(key);
        let normalized = stellarr_unicode::to_upper(&username);
        let (user, display_name) = match existing.get(&normalized) {
            // A placeholder whose record went missing is adopted; a person is
            // never one, and no form lets a person take such a name.
            Some((id, name, true)) => (*id, name.clone()),
            Some((_, _, false)) => {
                return Err(ImportError::State(format!(
                    "The account {username} is a person's, not a placeholder."
                )));
            }
            None => {
                let id = Id::random();
                let display_name = placeholder_name(author, &input.members);
                tx.execute(
                    "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"Email\", \"NormalizedEmail\",
                         \"ProviderVerifiedEmail\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\",
                         \"AvatarUrl\", \"BannerUrl\", \"WallpaperUrl\", \"CustomStatus\", \"Bio\", \"Pronouns\", \"Origin\")
                     VALUES (?1, ?2, ?3, ?4, NULL, NULL, NULL, NULL, 1, 0, 0, ?5, ?6, NULL, NULL, NULL, NULL, NULL, NULL, 'discord')",
                    params![
                        id,
                        username,
                        normalized,
                        display_name,
                        format!("oklch(0.58 0.14 {})", crate::accounts::avatar_hue(id)),
                        input.now
                    ],
                )?;
                counts.placeholders_made += 1;
                (id, display_name)
            }
        };
        let avatar = author
            .avatar_url
            .as_deref()
            .filter(|url| names::units(url) <= 256);
        tx.execute(
            "INSERT INTO \"DiscordImportAccounts\" (\"DiscordId\", \"UserId\", \"ImportId\", \"IsWebhook\", \"IsBot\", \"DiscordUsername\", \"DiscordAvatarUrl\", \"CreatedAt\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                key,
                user,
                input.import,
                author.webhook_id.is_some(),
                author.is_bot,
                names::clamp(&author.username, 100),
                avatar,
                input.now
            ],
        )?;
        accounts.insert(key.to_string(), Placeholder { user, display_name });
    }
    Ok(())
}

/// A channel as the import made it, `#name`; a thread by its own name.
fn channel_labels(
    db: &Connection,
    server: Id,
    asked: &std::collections::BTreeSet<String>,
) -> rusqlite::Result<HashMap<String, String>> {
    if asked.is_empty() {
        return Ok(HashMap::new());
    }
    let ids = json_texts(asked.iter().map(String::as_str));
    let mut statement = db.prepare_cached(
        "SELECT r.\"DiscordId\", r.\"Kind\", r.\"Name\", c.\"Name\" FROM \"DiscordImportChannels\" r
           LEFT JOIN \"Channels\" c ON c.\"Id\" = r.\"ChannelId\"
          WHERE r.\"ServerId\" = ?1 AND r.\"ChannelId\" IS NOT NULL
            AND r.\"DiscordId\" IN (SELECT value FROM json_each(?2))",
    )?;
    let rows = statement.query_map(params![server, ids], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, RecordKind>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut labels = HashMap::new();
    for row in rows {
        let (id, kind, record_name, channel_name) = row?;
        let name = if kind == RecordKind::Thread {
            record_name
        } else {
            channel_name
        };
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            labels.insert(id, format!("#{name}"));
        }
    }
    Ok(labels)
}
