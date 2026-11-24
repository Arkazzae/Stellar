//! Linking replies and threads once a history is whole.
//!
//! In a channel, a reply answers what it answered on Discord, flattened to
//! Stellarr's rule that a reply answers a thread's root: a reply to a reply
//! answers the first message of the chain that is still there. A reply whose
//! target is gone, or is not written yet because it waits for its attachments,
//! stays a plain message.
//!
//! A thread started from a message of its channel becomes replies to that
//! message (to its root, if it is a reply itself). One with no such message (a
//! forum post, a private thread, a thread whose message was deleted) takes its
//! first message as the root, which gets the thread's name as a bold first
//! line. Whether the message is there, and what it answers, is only known for
//! good once the parent channel is complete, so a thread completed before it
//! waits, and completing the parent links it, and links again the ones it
//! linked before.
//!
//! Completing can be repeated, as the attachment stage will once it has
//! written what waited for it: links are worked out afresh, and a thread's name
//! is put on its root once.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};
use stellarr_discord::{compare_ids, escape, extract_normalized_usernames};

use super::journal::{self, json_texts};
use super::structure::ChannelRecord;
use super::{
    Import, ImportError, ImportWriter, LinkResult, MESSAGES_PER_TRANSACTION, Outcome, RecordKind,
    is_forum, require_server,
};
use crate::messages::MAX_CONTENT_UNITS;
use crate::values::{Id, Timestamp};

/// How many ids one lookup of the bookkeeping names, well below SQLite's limit
/// on parameters.
const LOOKUP_SIZE: usize = 500;

/// One message of the bookkeeping as linking sees it.
#[derive(Clone)]
struct Node {
    discord_id: String,
    message: Option<Id>,
    reply_to: Option<String>,
    created_at: Option<Timestamp>,
}

/// What is to be set for one thread, worked out from a read of the bookkeeping.
struct ThreadPlan {
    root: Option<Id>,
    title_root: Option<Id>,
    links: Vec<(Id, Option<Id>)>,
}

impl ImportWriter {
    /// Says that a channel's or a thread's history has been read to its first
    /// message, and links what waited for that.
    pub async fn complete_channel(
        &self,
        import: &Import,
        discord_channel_id: &str,
    ) -> Result<LinkResult, ImportError> {
        let server = require_server(import)?;
        let wanted = discord_channel_id.to_string();
        let mut record = self
            .database
            .read(move |db| ChannelRecord::find(db, server, &wanted))
            .await?
            .ok_or_else(|| {
                ImportError::Argument(
                    "The import has not recorded this channel or thread.".to_string(),
                )
            })?;
        if record.completed_at.is_none() {
            let (now, id) = (Timestamp::now(), record.discord_id.clone());
            self.database
                .write(move |tx| {
                    tx.execute(
                        "UPDATE \"DiscordImportChannels\" SET \"CompletedAt\" = ?3 WHERE \"ServerId\" = ?1 AND \"DiscordId\" = ?2",
                        params![server, id, now],
                    )
                })
                .await?;
            record.completed_at = Some(now);
        }

        let mut result = LinkResult::default();
        if record.kind == RecordKind::Thread {
            self.link_thread(server, &record, &mut result).await?;
            return Ok(result);
        }

        self.link_replies(server, &record, &mut result).await?;
        // The threads that waited for this channel, and those that grew from one
        // of its messages, whose root a new completion may have moved.
        let (parent, forum) = (record.discord_id.clone(), is_forum(record.discord_type));
        let mut threads = self
            .database
            .read(move |db| -> rusqlite::Result<Vec<ChannelRecord>> {
                let mut statement = db.prepare_cached(&format!(
                    "SELECT {} FROM \"DiscordImportChannels\"
                      WHERE \"ServerId\" = ?1 AND \"ParentDiscordId\" = ?2 AND \"Kind\" = 1
                        AND \"CompletedAt\" IS NOT NULL
                        AND (\"LinkedAt\" IS NULL OR (?3 = 0 AND \"StarterMessageId\" IS NOT NULL))",
                    super::structure::RECORD_COLUMNS
                ))?;
                statement
                    .query_map(params![server, parent, forum], ChannelRecord::from_row)?
                    .collect()
            })
            .await?;
        threads.sort_by(|a, b| compare_ids(&a.discord_id, &b.discord_id));
        for thread in &threads {
            self.link_thread(server, thread, &mut result).await?;
        }
        Ok(result)
    }

    async fn link_replies(
        &self,
        server: Id,
        record: &ChannelRecord,
        result: &mut LinkResult,
    ) -> Result<(), ImportError> {
        let Some(channel) = record.channel else {
            return Ok(());
        };
        let source = record.discord_id.clone();
        let links = self
            .database
            .read(move |db| -> rusqlite::Result<Vec<(Id, Option<Id>)>> {
                let replies = written_nodes(db, server, &source, true)?;
                if replies.is_empty() {
                    return Ok(Vec::new());
                }
                let mut nodes: HashMap<String, Node> = replies
                    .iter()
                    .map(|n| (n.discord_id.clone(), n.clone()))
                    .collect();
                let targets = replies.iter().filter_map(|n| n.reply_to.clone()).collect();
                load_chains(db, server, &source, &mut nodes, targets)?;
                Ok(replies
                    .iter()
                    .filter_map(|reply| Some((reply.message?, root_of(reply, &nodes))))
                    .collect())
            })
            .await?;
        for (_, root) in &links {
            if root.is_none() {
                result.replies_left_plain += 1;
            } else {
                result.replies_linked += 1;
            }
        }
        self.set_replies(channel, links).await
    }

    async fn link_thread(
        &self,
        server: Id,
        thread: &ChannelRecord,
        result: &mut LinkResult,
    ) -> Result<(), ImportError> {
        let Some(channel) = thread.channel else {
            return Ok(());
        };
        let record = thread.clone();
        let plan = self
            .database
            .read(move |db| plan_thread(db, server, &record))
            .await?;
        let Some(plan) = plan else {
            result.waiting = true;
            return Ok(());
        };

        self.set_replies(channel, plan.links.clone()).await?;
        if let Some(title_root) = plan.title_root {
            self.title(server, thread, channel, title_root).await?;
        }
        let (now, id, root) = (Timestamp::now(), thread.discord_id.clone(), plan.root);
        self.database
            .write(move |tx| {
                tx.execute(
                    "UPDATE \"DiscordImportChannels\" SET \"RootMessageId\" = ?3, \"LinkedAt\" = ?4
                      WHERE \"ServerId\" = ?1 AND \"DiscordId\" = ?2",
                    params![server, id, root, now],
                )
            })
            .await?;
        result.replies_linked += plan.links.iter().filter(|(_, root)| root.is_some()).count();
        result.threads_linked += 1;
        Ok(())
    }

    /// Puts the thread's name on its root as a bold first line, escaped, so that
    /// the name shows as written and mentions nobody. The root is recorded in the
    /// same transaction, which is what keeps a second completion from putting the
    /// name on twice.
    async fn title(
        &self,
        server: Id,
        thread: &ChannelRecord,
        channel: Id,
        root: Id,
    ) -> Result<(), ImportError> {
        self.pause().await;
        let (id, name, now) = (
            thread.discord_id.clone(),
            thread.name.clone(),
            Timestamp::now(),
        );
        self.database
            .write(move |tx| -> rusqlite::Result<()> {
                tx.execute(
                    "UPDATE \"DiscordImportChannels\" SET \"RootMessageId\" = ?3 WHERE \"ServerId\" = ?1 AND \"DiscordId\" = ?2",
                    params![server, id, root],
                )?;
                let content: Option<String> = tx
                    .query_row("SELECT \"Content\" FROM \"Messages\" WHERE \"Id\" = ?1", [root], |row| {
                        row.get(0)
                    })
                    .map(Some)
                    .or_else(|error| match error {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                if let (Some(content), Some(name)) = (content, name.filter(|name| !name.is_empty())) {
                    let titled = format!("**{}**\n{content}", escape(&name));
                    if super::names::units(&titled) <= MAX_CONTENT_UNITS
                        && extract_normalized_usernames(&titled).is_empty()
                    {
                        tx.execute(
                            "UPDATE \"Messages\" SET \"Content\" = ?2 WHERE \"Id\" = ?1",
                            params![root, titled],
                        )?;
                        journal::reset(tx, channel, &[root], now)?;
                    }
                }
                Ok(())
            })
            .await?;
        self.committed();
        Ok(())
    }

    /// Sets what each message answers, a bounded transaction at a time, and
    /// resets the channel's journal.
    async fn set_replies(
        &self,
        channel: Id,
        links: Vec<(Id, Option<Id>)>,
    ) -> Result<(), ImportError> {
        for chunk in links.chunks(MESSAGES_PER_TRANSACTION) {
            self.pause().await;
            let chunk = chunk.to_vec();
            let now = Timestamp::now();
            self.database
                .write(move |tx| -> rusqlite::Result<()> {
                    let mut changed = 0;
                    for (message, root) in &chunk {
                        changed += tx.execute(
                            "UPDATE \"Messages\" SET \"ReplyToMessageId\" = ?2
                              WHERE \"Id\" = ?1 AND \"ReplyToMessageId\" IS NOT ?2",
                            params![message, root],
                        )?;
                    }
                    // What a message answers is not in the journal; a client that
                    // loaded the channel before has to read it again.
                    if changed > 0 {
                        journal::reset(tx, channel, &[], now)?;
                    }
                    Ok(())
                })
                .await?;
            self.committed();
        }
        Ok(())
    }
}

/// The written messages of a channel, or those of them that answer something.
fn written_nodes(
    db: &Connection,
    server: Id,
    channel: &str,
    only_replies: bool,
) -> rusqlite::Result<Vec<Node>> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT \"DiscordId\", \"MessageId\", \"ReplyToDiscordId\", \"CreatedAt\" FROM \"DiscordImportMessages\"
          WHERE \"ServerId\" = ?1 AND \"ChannelDiscordId\" = ?2 AND \"Outcome\" = ?3 AND \"MessageId\" IS NOT NULL{}",
        if only_replies { " AND \"ReplyToDiscordId\" IS NOT NULL" } else { "" }
    ))?;
    statement
        .query_map(params![server, channel, Outcome::Written], |row| {
            Ok(Node {
                discord_id: row.get(0)?,
                message: row.get(1)?,
                reply_to: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?
        .collect()
}

/// Adds to `nodes` every message the chains starting at `targets` pass through
/// in this channel, a lookup at a time; one that is not there at all is added as
/// gone.
fn load_chains(
    db: &Connection,
    server: Id,
    channel: &str,
    nodes: &mut HashMap<String, Node>,
    targets: Vec<String>,
) -> rusqlite::Result<()> {
    let mut pending: Vec<String> = Vec::new();
    {
        let mut seen = HashSet::new();
        for id in targets {
            if !nodes.contains_key(&id) && seen.insert(id.clone()) {
                pending.push(id);
            }
        }
    }
    let mut statement = db.prepare_cached(
        "SELECT \"DiscordId\", CASE WHEN \"Outcome\" = ?4 THEN \"MessageId\" END, \"ReplyToDiscordId\", \"CreatedAt\"
           FROM \"DiscordImportMessages\"
          WHERE \"ServerId\" = ?1 AND \"ChannelDiscordId\" = ?2 AND \"DiscordId\" IN (SELECT value FROM json_each(?3))",
    )?;
    while !pending.is_empty() {
        let mut next: Vec<String> = Vec::new();
        let mut queued = HashSet::new();
        for chunk in pending.chunks(LOOKUP_SIZE) {
            let found: Vec<Node> = statement
                .query_map(
                    params![
                        server,
                        channel,
                        json_texts(chunk.iter().map(String::as_str)),
                        Outcome::Written
                    ],
                    |row| {
                        Ok(Node {
                            discord_id: row.get(0)?,
                            message: row.get(1)?,
                            reply_to: row.get(2)?,
                            created_at: row.get(3)?,
                        })
                    },
                )?
                .collect::<rusqlite::Result<_>>()?;
            for node in found {
                if let Some(further) = node.reply_to.clone()
                    && !nodes.contains_key(&further)
                    && queued.insert(further.clone())
                {
                    next.push(further);
                }
                nodes.insert(node.discord_id.clone(), node);
            }
            for id in chunk {
                nodes.entry(id.clone()).or_insert_with(|| Node {
                    discord_id: id.clone(),
                    message: None,
                    reply_to: None,
                    created_at: None,
                });
            }
        }
        next.retain(|id| !nodes.contains_key(id));
        pending = next;
    }
    Ok(())
}

/// The root a message answers: the earliest message of its chain that is
/// written, walking back until a target is gone. None when even the first target
/// is gone.
fn root_of(reply: &Node, nodes: &HashMap<String, Node>) -> Option<Id> {
    let mut root = None;
    let mut visited: HashSet<&str> = HashSet::from([reply.discord_id.as_str()]);
    let mut target = reply.reply_to.as_deref();
    while let Some(id) = target {
        if !visited.insert(id) {
            break;
        }
        let Some(node) = nodes.get(id) else { break };
        let Some(written) = node.message else { break };
        root = Some(written);
        target = node.reply_to.as_deref();
    }
    root
}

/// What a thread's messages are to answer, or none while the channel it grew
/// from is not read to its first message.
fn plan_thread(
    db: &Connection,
    server: Id,
    thread: &ChannelRecord,
) -> rusqlite::Result<Option<ThreadPlan>> {
    let parent = match &thread.parent {
        Some(id) => ChannelRecord::find(db, server, id)?,
        None => None,
    };
    let forum = is_forum(thread.discord_type);

    // A thread under a text channel grew from one of its messages, when that
    // message is there; what it answers is only certain once the whole channel
    // is read. In a forum, the post's own first message opens it, inside the
    // thread.
    let mut root = None;
    if let (false, Some(parent), Some(starter)) = (forum, &parent, &thread.starter_message) {
        if parent.completed_at.is_none() {
            return Ok(None);
        }
        let mut nodes = HashMap::new();
        load_chains(
            db,
            server,
            &parent.discord_id,
            &mut nodes,
            vec![starter.clone()],
        )?;
        let starter = &nodes[starter];
        if let Some(written) = starter.message {
            root = Some(root_of(starter, &nodes).unwrap_or(written));
        }
    }

    let messages = written_nodes(db, server, &thread.discord_id, false)?;
    let titled = root.is_none();
    if titled {
        root = messages
            .iter()
            .min_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| compare_ids(&a.discord_id, &b.discord_id))
            })
            .and_then(|first| first.message);
    }
    let title_root = if titled && root.is_some() && thread.root_message != root {
        root
    } else {
        None
    };
    let mut links: Vec<(Id, Option<Id>)> = messages
        .iter()
        .filter_map(|n| n.message)
        .filter(|message| Some(*message) != root)
        .map(|message| (message, root))
        .collect();
    if let (true, Some(own)) = (titled, root) {
        // The root starts the thread and answers nothing itself.
        links.push((own, None));
    }
    Ok(Some(ThreadPlan {
        root,
        title_root,
        links,
    }))
}
