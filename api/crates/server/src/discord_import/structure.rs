//! Making the sections and channels of a snapshot on the server, and recording
//! its threads.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Row, params};
use stellarr_discord::{
    ChannelKind, DiscordChannelType, ServerSnapshot, compare_ids, hidden_from_everyone,
};

use super::{
    Import, ImportError, ImportWriter, MESSAGES_PER_TRANSACTION, RecordKind, StructureResult,
    names, require_bot, require_same_server, require_server, type_number, type_of,
};
use crate::values::{Id, Timestamp};

/// A Discord channel or thread, where its messages went, and how far the import
/// has read it.
#[derive(Debug, Clone)]
pub(super) struct ChannelRecord {
    pub discord_id: String,
    pub kind: RecordKind,
    pub channel: Option<Id>,
    pub discord_type: DiscordChannelType,
    pub parent: Option<String>,
    pub name: Option<String>,
    pub starter_message: Option<String>,
    pub is_private: bool,
    pub oldest_message: Option<String>,
    pub completed_at: Option<Timestamp>,
    pub root_message: Option<Id>,
}

pub(super) const RECORD_COLUMNS: &str =
    "\"DiscordId\", \"Kind\", \"ChannelId\", \"DiscordType\", \"ParentDiscordId\", \"Name\",
    \"StarterMessageId\", \"IsPrivate\", \"OldestMessageId\", \"CompletedAt\", \"RootMessageId\"";

impl ChannelRecord {
    pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<ChannelRecord> {
        Ok(ChannelRecord {
            discord_id: row.get(0)?,
            kind: row.get(1)?,
            channel: row.get(2)?,
            discord_type: type_of(row.get(3)?),
            parent: row.get(4)?,
            name: row.get(5)?,
            starter_message: row.get(6)?,
            is_private: row.get(7)?,
            oldest_message: row.get(8)?,
            completed_at: row.get(9)?,
            root_message: row.get(10)?,
        })
    }

    pub(super) fn find(
        db: &Connection,
        server: Id,
        discord_id: &str,
    ) -> rusqlite::Result<Option<ChannelRecord>> {
        db.query_row(
            &format!(
                "SELECT {RECORD_COLUMNS} FROM \"DiscordImportChannels\" WHERE \"ServerId\" = ?1 AND \"DiscordId\" = ?2"
            ),
            params![server, discord_id],
            ChannelRecord::from_row,
        )
        .optional()
    }
}

impl ImportWriter {
    /// Records the start of an import of `snapshot`'s server into `server`.
    pub async fn start(
        &self,
        server: Id,
        started_by: Id,
        snapshot: &ServerSnapshot,
    ) -> Result<Import, ImportError> {
        require_bot(snapshot)?;
        let import = Import {
            id: Id::random(),
            server: Some(server),
            discord_server_id: snapshot.source_id.clone(),
            discord_server_name: names::clamp(&snapshot.name, 100),
            started_by: Some(started_by),
            started_at: Timestamp::now(),
        };
        let row = import.clone();
        self.database
            .write(move |tx| -> Result<(), ImportError> {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS (SELECT 1 FROM \"Servers\" WHERE \"Id\" = ?1)",
                    [server],
                    |row| row.get(0),
                )?;
                if !exists {
                    return Err(ImportError::Argument(
                        "There is no such server to import into.".to_string(),
                    ));
                }
                tx.execute(
                    "INSERT INTO \"DiscordImports\" (\"Id\", \"ServerId\", \"DiscordServerId\", \"DiscordServerName\", \"StartedById\", \"StartedAt\")
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        row.id,
                        row.server,
                        row.discord_server_id,
                        row.discord_server_name,
                        row.started_by,
                        row.started_at
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(import)
    }

    /// Where reading a channel or thread goes on after an interruption: the
    /// oldest message it holds, whose id is where the next page, older than it,
    /// starts. None when nothing of it is read yet.
    pub async fn resume_after(
        &self,
        import: &Import,
        discord_channel_id: &str,
    ) -> Result<Option<String>, ImportError> {
        let server = require_server(import)?;
        let wanted = discord_channel_id.to_string();
        let record = self
            .database
            .read(move |db| ChannelRecord::find(db, server, &wanted))
            .await?
            .ok_or_else(|| {
                ImportError::Argument(
                    "The import has not recorded this channel or thread.".to_string(),
                )
            })?;
        Ok(record.oldest_message)
    }

    /// Makes the snapshot's sections and channels on the server, below what it
    /// has, in Discord's order, and records its threads. Roles, overwrites,
    /// emoji and attachments are not imported here.
    ///
    /// A forum, a media or an announcement channel becomes a text channel and a
    /// stage a voice channel; what it was stays in the bookkeeping. A name is made
    /// one Stellarr accepts and unique on the server. What an earlier run made is
    /// left as it is, and what somebody deleted since is not made again.
    pub async fn write_structure(
        &self,
        import: &Import,
        snapshot: &ServerSnapshot,
    ) -> Result<StructureResult, ImportError> {
        let server = require_server(import)?;
        require_same_server(import, snapshot)?;
        self.pause().await;

        let (import_id, source) = (import.id, snapshot.clone());
        let made = self
            .database
            .write(move |tx| made_structure(tx, import_id, server, &source))
            .await?;
        self.committed();

        let mut result = StructureResult {
            sections_made: made.sections_made,
            channels_made: made.channels_made,
            ..StructureResult::default()
        };
        let already = made.already;
        for channel in &snapshot.channels {
            if channel.discord_type != DiscordChannelType::Conversation {
                result
                    .simplified
                    .insert(channel.id.clone(), channel.discord_type);
            }
            if made
                .channels
                .get(&channel.id)
                .is_some_and(|record| record.is_private)
            {
                result.hidden_from_everyone.push(channel.id.clone());
            }
        }
        let (recorded, threads_already) = self
            .write_threads(import, server, snapshot, &made.channels)
            .await?;
        result.threads_recorded = recorded;
        result.already_imported = already + threads_already;
        Ok(result)
    }

    /// Records the threads, whose messages become threads in their parent's
    /// channel. There can be tens of thousands in a busy forum, so they go in
    /// bounded transactions too.
    async fn write_threads(
        &self,
        import: &Import,
        server: Id,
        snapshot: &ServerSnapshot,
        parents: &HashMap<String, ChannelRecord>,
    ) -> Result<(usize, usize), ImportError> {
        let known: HashSet<String> = {
            let wanted = server;
            self.database
                .read(move |db| -> Result<HashSet<String>, ImportError> {
                    let mut statement = db.prepare(
                        "SELECT \"DiscordId\" FROM \"DiscordImportChannels\" WHERE \"ServerId\" = ?1 AND \"Kind\" = ?2",
                    )?;
                    Ok(statement
                        .query_map(params![wanted, RecordKind::Thread], |row| row.get(0))?
                        .collect::<rusqlite::Result<_>>()?)
                })
                .await?
        };
        let already = snapshot
            .threads
            .iter()
            .filter(|x| known.contains(&x.id))
            .count();
        let mut seen = HashSet::new();
        let fresh: Vec<_> = snapshot
            .threads
            .iter()
            .filter(|x| !known.contains(&x.id) && parents.contains_key(&x.parent_channel_id))
            .filter(|x| seen.insert(x.id.clone()))
            .cloned()
            .collect();
        for chunk in fresh.chunks(MESSAGES_PER_TRANSACTION) {
            self.pause().await;
            let chunk = chunk.to_vec();
            let parents = parents.clone();
            let import_id = import.id;
            self.database
                .write(move |tx| -> Result<(), ImportError> {
                    for thread in &chunk {
                        let Some(parent) = parents.get(&thread.parent_channel_id) else {
                            continue;
                        };
                        tx.execute(
                            "INSERT INTO \"DiscordImportChannels\" (\"ServerId\", \"DiscordId\", \"ImportId\", \"Kind\", \"ChannelId\", \"DiscordType\",
                                 \"ParentDiscordId\", \"Name\", \"StarterMessageId\", \"IsPrivate\", \"OldestMessageId\", \"CompletedAt\", \"RootMessageId\", \"LinkedAt\")
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, NULL, NULL)",
                            params![
                                server,
                                thread.id,
                                import_id,
                                RecordKind::Thread,
                                parent.channel,
                                type_number(parent.discord_type),
                                parent.discord_id,
                                names::clamp(&thread.name, 100),
                                thread.starter_message_id,
                                thread.private || parent.is_private
                            ],
                        )?;
                    }
                    Ok(())
                })
                .await?;
            self.committed();
        }
        Ok((fresh.len(), already))
    }
}

struct Made {
    sections_made: usize,
    channels_made: usize,
    already: usize,
    channels: HashMap<String, ChannelRecord>,
}

fn made_structure(
    tx: &rusqlite::Transaction<'_>,
    import: Id,
    server: Id,
    snapshot: &ServerSnapshot,
) -> Result<Made, ImportError> {
    let everyone = snapshot.roles.iter().find(|role| role.is_everyone);
    let mut sections: HashMap<String, Option<Id>> = HashMap::new();
    {
        let mut statement = tx.prepare(
            "SELECT \"DiscordId\", \"SectionId\" FROM \"DiscordImportSections\" WHERE \"ServerId\" = ?1",
        )?;
        for row in statement.query_map([server], |row| Ok((row.get(0)?, row.get(1)?)))? {
            let (id, section): (String, Option<Id>) = row?;
            sections.insert(id, section);
        }
    }
    let mut channels: HashMap<String, ChannelRecord> = HashMap::new();
    {
        let mut statement = tx.prepare(&format!(
            "SELECT {RECORD_COLUMNS} FROM \"DiscordImportChannels\" WHERE \"ServerId\" = ?1 AND \"Kind\" = ?2"
        ))?;
        for record in statement.query_map(
            params![server, RecordKind::Channel],
            ChannelRecord::from_row,
        )? {
            let record = record?;
            channels.insert(record.discord_id.clone(), record);
        }
    }
    let mut section_position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(\"Position\"), -1) + 1 FROM \"Sections\" WHERE \"ServerId\" = ?1",
        [server],
        |row| row.get(0),
    )?;
    let mut channel_position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(\"Position\"), -1) + 1 FROM \"Channels\" WHERE \"ServerId\" = ?1",
        [server],
        |row| row.get(0),
    )?;
    let mut taken: HashSet<String> = {
        let mut statement =
            tx.prepare("SELECT \"Name\" FROM \"Channels\" WHERE \"ServerId\" = ?1")?;
        statement
            .query_map([server], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };

    let mut ordered_sections: Vec<_> = snapshot.sections.iter().collect();
    ordered_sections.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| compare_ids(&a.id, &b.id))
    });
    let (mut sections_made, mut channels_made, mut already) = (0, 0, 0);
    for section in &ordered_sections {
        if sections.contains_key(&section.id) {
            already += 1;
            continue;
        }
        let made = Id::random();
        tx.execute(
            "INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, ?3, ?4)",
            params![made, server, names::section_name(&section.name), section_position],
        )?;
        section_position += 1;
        tx.execute(
            "INSERT INTO \"DiscordImportSections\" (\"ServerId\", \"DiscordId\", \"ImportId\", \"SectionId\") VALUES (?1, ?2, ?3, ?4)",
            params![server, section.id, import, made],
        )?;
        sections.insert(section.id.clone(), Some(made));
        sections_made += 1;
    }

    // Discord's client shows the channels outside any category first, then each
    // category's; within one, text before voice, each by position and then by id.
    let section_order: HashMap<&str, usize> = ordered_sections
        .iter()
        .enumerate()
        .map(|(index, section)| (section.id.as_str(), index))
        .collect();
    let mut ordered_channels: Vec<_> = snapshot.channels.iter().collect();
    let section_key = |channel: &&stellarr_discord::ChannelSnapshot| -> i64 {
        match &channel.section_id {
            Some(section) => section_order
                .get(section.as_str())
                .map_or(i64::from(i32::MAX), |index| *index as i64),
            None => -1,
        }
    };
    ordered_channels.sort_by(|a, b| {
        section_key(a)
            .cmp(&section_key(b))
            .then_with(|| (a.kind == ChannelKind::Voice).cmp(&(b.kind == ChannelKind::Voice)))
            .then_with(|| a.position.cmp(&b.position))
            .then_with(|| compare_ids(&a.id, &b.id))
    });
    for channel in ordered_channels {
        if channels.contains_key(&channel.id) {
            already += 1;
            continue;
        }
        let text = channel.kind == ChannelKind::Text;
        let made = Id::random();
        let section = channel
            .section_id
            .as_ref()
            .and_then(|section| sections.get(section).copied().flatten());
        let name = names::unique(names::channel_name(&channel.name), &mut taken);
        tx.execute(
            "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\",
                 \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9)",
            params![
                made,
                server,
                name,
                if text { 0 } else { 1 },
                channel_position,
                if text { names::topic(channel.topic.as_deref()) } else { String::new() },
                if text { names::slow_mode(channel.slow_mode_seconds) } else { 0 },
                if text { 0 } else { names::user_limit(channel.user_limit) },
                section
            ],
        )?;
        channel_position += 1;
        let record = ChannelRecord {
            discord_id: channel.id.clone(),
            kind: RecordKind::Channel,
            channel: Some(made),
            discord_type: channel.discord_type,
            parent: None,
            name: Some(names::clamp(&channel.name, 100)),
            starter_message: None,
            is_private: hidden_from_everyone(channel, everyone),
            oldest_message: None,
            completed_at: None,
            root_message: None,
        };
        tx.execute(
            "INSERT INTO \"DiscordImportChannels\" (\"ServerId\", \"DiscordId\", \"ImportId\", \"Kind\", \"ChannelId\", \"DiscordType\",
                 \"ParentDiscordId\", \"Name\", \"StarterMessageId\", \"IsPrivate\", \"OldestMessageId\", \"CompletedAt\", \"RootMessageId\", \"LinkedAt\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, NULL, ?8, NULL, NULL, NULL, NULL)",
            params![
                server,
                record.discord_id,
                import,
                RecordKind::Channel,
                made,
                type_number(record.discord_type),
                record.name,
                record.is_private
            ],
        )?;
        channels.insert(channel.id.clone(), record);
        channels_made += 1;
    }
    Ok(Made {
        sections_made,
        channels_made,
        already,
        channels,
    })
}
