use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_discord::{
    BotReader, ChannelKind, DiscordClient, DiscordError, FailureKind, ServerSnapshot,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::structure::ChannelRecord;
use super::{Import, ImportWriter, RecordKind, moment, names};
use super::{
    media,
    plan::{self, History, Options},
};
use crate::database::DbError;
use crate::instance::starter::Starter;
use crate::permissions::ownership;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

pub(crate) struct Jobs {
    pub(crate) discord: DiscordClient,
    pub(super) previews: Semaphore,
    gate: Arc<Semaphore>,
    run_id: Id,
}

impl Default for Jobs {
    fn default() -> Self {
        Self {
            discord: DiscordClient::new(),
            previews: Semaphore::new(4),
            gate: Arc::new(Semaphore::new(1)),
            run_id: Id::random(),
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    messages: usize,
    skipped_messages: usize,
    messages_missing_attachments: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Status {
    history_requested: bool,
    pending_attachments: i64,
    id: Id,
    pub(super) state: String,
    server_id: Option<Id>,
    name: String,
    total_sources: Option<i64>,
    completed_sources: i64,
    channels_imported: i64,
    sections_imported: i64,
    report: Option<Report>,
    error: Option<String>,
}

pub(super) struct Job {
    pub(super) import: Import,
    pub(super) options: Options,
    state: String,
    run_id: Id,
    total_sources: Option<i64>,
    completed_sources: i64,
    channels_imported: i64,
    sections_imported: i64,
    pending_attachments: i64,
    report: Option<Report>,
    error: Option<String>,
}

impl Job {
    pub(super) fn status(&self, run_id: Id) -> Status {
        let interrupted = self.state == "running" && self.run_id != run_id;
        Status {
            id: self.import.id,
            state: if interrupted {
                "failed".into()
            } else {
                self.state.clone()
            },
            server_id: self.import.server.filter(|_| self.report.is_some()),
            name: self.options.name.clone(),
            total_sources: self.total_sources,
            completed_sources: self.completed_sources,
            channels_imported: self.channels_imported,
            sections_imported: self.sections_imported,
            history_requested: self.options.history.copies_messages(),
            pending_attachments: self.pending_attachments,
            report: self.report.clone(),
            error: if interrupted {
                Some("discord-interrupted".into())
            } else {
                self.error.clone()
            },
        }
    }

    fn find(db: &Connection, id: Id, actor: Id) -> Result<Option<Self>, DbError> {
        Ok(db.query_row(
            "SELECT i.\"ServerId\", i.\"DiscordServerId\", i.\"DiscordServerName\", i.\"StartedAt\",
                    j.\"Options\", j.\"RunId\", j.\"Status\", j.\"TotalSources\", j.\"Report\", j.\"Error\",
                    (SELECT COUNT(*) FROM \"DiscordImportChannels\" c
                     WHERE c.\"ServerId\" = i.\"ServerId\" AND c.\"CompletedAt\" IS NOT NULL),
                    (SELECT COUNT(*) FROM \"DiscordImportChannels\" c
                     WHERE c.\"ServerId\" = i.\"ServerId\" AND c.\"Kind\" = ?3),
                    (SELECT COUNT(*) FROM \"DiscordImportSections\" s
                     WHERE s.\"ServerId\" = i.\"ServerId\"),
                    (SELECT COUNT(*) FROM \"DiscordImportMessages\" d WHERE d.\"ImportId\" = i.\"Id\"
                     AND d.\"AttachmentsPending\" = 1 AND (d.\"Outcome\" = 2 OR d.\"MessageId\" IS NOT NULL))
             FROM \"DiscordImportJobs\" j JOIN \"DiscordImports\" i ON i.\"Id\" = j.\"ImportId\"
             WHERE j.\"ImportId\" = ?1 AND i.\"StartedById\" = ?2 AND i.\"ServerId\" IS NOT NULL",
            params![id, actor, RecordKind::Channel],
            |row| {
                let options: String = row.get(4)?;
                let report: Option<String> = row.get(8)?;
                let decode_error = |column, err| {
                    rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, Box::new(err))
                };
                Ok(Job {
                    import: Import {
                        id,
                        server: row.get(0)?,
                        discord_server_id: row.get(1)?,
                        discord_server_name: row.get(2)?,
                        started_by: Some(actor),
                        started_at: row.get(3)?,
                    },
                    options: serde_json::from_str(&options).map_err(|err| decode_error(4, err))?,
                    run_id: row.get(5)?,
                    state: row.get(6)?,
                    total_sources: row.get(7)?,
                    report: report.as_deref().map(serde_json::from_str).transpose()
                        .map_err(|err| decode_error(8, err))?,
                    error: row.get(9)?,
                    completed_sources: row.get(10)?,
                    channels_imported: row.get(11)?,
                    sections_imported: row.get(12)?,
                    pending_attachments: row.get(13)?,
                })
            },
        ).optional()?)
    }

    pub(super) fn published(&self) -> bool {
        self.report.is_some()
    }

    pub(super) fn has_pending_attachments(&self) -> bool {
        self.pending_attachments > 0
    }

    pub(super) async fn still_owned(&self, state: &AppState, actor: Id) -> Result<bool, DbError> {
        let server = self.import.server;
        state.database().read(move |db| -> Result<_, DbError> {
            Ok(db.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2 AND \"OwnerSlot\" IS NOT NULL)",
                params![server, actor],
                |row| row.get(0),
            )?)
        }).await
    }
}

impl Jobs {
    pub(super) fn run_id(&self) -> Id {
        self.run_id
    }

    pub(super) fn reserve(&self) -> Option<OwnedSemaphorePermit> {
        self.gate.clone().try_acquire_owned().ok()
    }

    pub(super) async fn find(state: &AppState, id: Id, actor: Id) -> Result<Option<Job>, DbError> {
        state
            .database()
            .read(move |db| Job::find(db, id, actor))
            .await
    }

    pub(super) async fn latest(state: &AppState, actor: Id) -> Result<Option<Job>, DbError> {
        state.database().read(move |db| -> Result<Option<Job>, DbError> {
            let id = db.query_row(
                "SELECT i.\"Id\" FROM \"DiscordImports\" i JOIN \"DiscordImportJobs\" j ON i.\"Id\" = j.\"ImportId\"
                 LEFT JOIN \"ServerMembers\" m ON m.\"ServerId\" = i.\"ServerId\" AND m.\"UserId\" = ?1
                 WHERE i.\"StartedById\" = ?1 AND i.\"ServerId\" IS NOT NULL
                   AND (j.\"Report\" IS NULL OR m.\"OwnerSlot\" IS NOT NULL)
                 ORDER BY i.\"StartedAt\" DESC, i.\"Id\" DESC LIMIT 1",
                [actor], |row| row.get(0)
            ).optional()?;
            match id {
                Some(id) => Job::find(db, id, actor),
                None => Ok(None),
            }
        }).await
    }

    pub(super) async fn create(
        state: &AppState,
        id: Id,
        actor: Id,
        options: Options,
        starter: Starter,
        snapshot: &ServerSnapshot,
    ) -> Result<Job, DbError> {
        let run_id = state.discord_imports().run_id;
        let encoded =
            serde_json::to_string(&options).map_err(|err| DbError::Task(err.to_string()))?;
        let source_name = names::clamp(&snapshot.name, 100);
        let has_text = snapshot
            .channels
            .iter()
            .any(|channel| channel.kind == ChannelKind::Text);
        let state = state.clone();

        state.database().clone().write(move |tx| {
            let now = Timestamp::now();
            let added = crate::instance::starter::add(tx, &starter, state.messages(), now)?;
            if has_text {
                tx.execute("DELETE FROM \"Channels\" WHERE \"Id\" = ?1", [added.first_text_channel])?;
            }
            Import {
                id,
                server: Some(added.id),
                discord_server_id: options.source_id,
                discord_server_name: source_name,
                started_by: Some(actor),
                started_at: now,
            }
            .record(tx)?;
            tx.execute(
                "INSERT INTO \"DiscordImportJobs\" (\"ImportId\", \"Options\", \"RunId\", \"Status\")
                 VALUES (?1, ?2, ?3, 'running')",
                params![id, encoded, run_id],
            )?;
            Job::find(tx, id, actor)?.ok_or_else(|| DbError::Task("import was not created".into()))
        }).await
    }

    pub(super) async fn retry(state: &AppState, mut job: Job) -> Result<Job, DbError> {
        let id = job.import.id;
        let run_id = state.discord_imports().run_id;
        state.database().write(move |tx| -> Result<(), DbError> {
            tx.execute(
                "UPDATE \"DiscordImportJobs\" SET \"RunId\" = ?2, \"Status\" = 'running', \"Error\" = NULL
                 WHERE \"ImportId\" = ?1",
                params![id, run_id],
            )?;
            Ok(())
        }).await?;
        job.run_id = run_id;
        job.state = "running".into();
        job.error = None;
        Ok(job)
    }

    pub(super) async fn discard(state: &AppState, job: Job) -> Result<(), DbError> {
        state
            .database()
            .write(move |tx| -> Result<(), DbError> {
                if let Some(server) = job.import.server {
                    ownership::delete_server(tx, server)?;
                }
                tx.execute(
                    "DELETE FROM \"DiscordImports\" WHERE \"Id\" = ?1",
                    [job.import.id],
                )?;
                Ok(())
            })
            .await
    }

    pub(super) fn launch(
        state: AppState,
        permit: OwnedSemaphorePermit,
        job: Job,
        bot: BotReader,
        snapshot: ServerSnapshot,
    ) {
        tokio::spawn(async move {
            let _permit = permit;
            let result = if job.published() {
                run_media(&state, &job, &bot, snapshot).await
            } else {
                run(&state, &job, &bot, snapshot).await
            };
            if let Err(code) = result {
                let id = job.import.id;
                if let Err(err) = state
                    .database()
                    .write(move |tx| -> Result<(), DbError> {
                        tx.execute(
                            "UPDATE \"DiscordImportJobs\" SET \"Status\" = 'failed', \"Error\" = ?2
                         WHERE \"ImportId\" = ?1",
                            params![id, code],
                        )?;
                        Ok(())
                    })
                    .await
                {
                    tracing::error!(%id, %err, "could not record the Discord import failure");
                }
            }
        });
    }
}

pub(super) fn discord_error(error: DiscordError) -> &'static str {
    match error.kind() {
        FailureKind::InvalidToken => "discord-invalid-token",
        FailureKind::MissingAccess | FailureKind::UnknownResource => "discord-bot-access",
        FailureKind::MissingIntent => "discord-message-content",
        FailureKind::RateLimited => "discord-rate-limited",
        FailureKind::InvalidArgument => "discord-invalid-selection",
        FailureKind::Unreachable | FailureKind::UnexpectedResponse => "discord-unavailable",
    }
}

pub(super) fn storage_error(error: impl std::fmt::Display) -> &'static str {
    tracing::error!(%error, "Discord import could not update its local state");
    "discord-write-failed"
}

fn report_json(tally: super::Tally) -> Result<String, &'static str> {
    serde_json::to_string(&Report {
        messages: tally.written,
        skipped_messages: tally.skipped.values().sum::<usize>() + tally.awaiting_attachments,
        messages_missing_attachments: tally.attachments_pending + tally.awaiting_attachments,
    })
    .map_err(storage_error)
}

async fn run_media(
    state: &AppState,
    job: &Job,
    bot: &BotReader,
    snapshot: ServerSnapshot,
) -> Result<(), &'static str> {
    let server = job.import.server.ok_or("discord-write-failed")?;
    let writer = ImportWriter::new(state.database().clone());
    let result = media::copy(
        state,
        &writer,
        &job.import,
        bot,
        snapshot,
        job.options.include_threads,
    )
    .await;
    let report = report_json(writer.tally(server).await.map_err(storage_error)?)?;
    let id = job.import.id;
    let status = if result.is_ok() {
        "completed"
    } else {
        "failed"
    };
    let error = result.err();
    state.database().write(move |tx| -> Result<(), DbError> {
        tx.execute(
            "UPDATE \"DiscordImportJobs\" SET \"Status\" = ?2, \"Report\" = ?3, \"Error\" = ?4 WHERE \"ImportId\" = ?1",
            params![id, status, report, error],
        )?;
        Ok(())
    }).await.map_err(storage_error)?;
    Ok(())
}

async fn run(
    state: &AppState,
    job: &Job,
    bot: &BotReader,
    mut snapshot: ServerSnapshot,
) -> Result<(), &'static str> {
    let server = job.import.server.ok_or("discord-write-failed")?;
    let actor = job.import.started_by.ok_or("discord-write-failed")?;
    let id = job.import.id;
    let history = job.options.history;
    if job.options.include_threads && history.copies_messages() {
        plan::add_threads(bot, &mut snapshot)
            .await
            .map_err(discord_error)?;
    }
    let threads: std::collections::HashSet<_> = snapshot
        .threads
        .iter()
        .map(|thread| thread.id.clone())
        .collect();
    let retained = state
        .database()
        .read(move |db| -> Result<bool, DbError> {
            let mut query = db.prepare(
                "SELECT \"DiscordId\" FROM \"DiscordImportChannels\"
             WHERE \"ServerId\" = ?1 AND \"Kind\" = ?2",
            )?;
            for row in query.query_map(params![server, RecordKind::Thread], |row| {
                row.get::<_, String>(0)
            })? {
                if !threads.contains(&row?) {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .await
        .map_err(storage_error)?;
    if !retained {
        return Err("discord-selection-changed");
    }
    let writer = ImportWriter::new(state.database().clone());
    writer
        .write_structure(&job.import, &snapshot)
        .await
        .map_err(storage_error)?;
    let sources: Vec<_> = if history.copies_messages() {
        snapshot
            .channels
            .iter()
            .filter(|channel| channel.kind == ChannelKind::Text)
            .map(|channel| (channel.id.as_str(), plan::has_own_messages(channel)))
            .chain(
                snapshot
                    .threads
                    .iter()
                    .map(|thread| (thread.id.as_str(), true)),
            )
            .collect()
    } else {
        Vec::new()
    };
    let total = sources.len() as i64;
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            tx.execute(
                "UPDATE \"DiscordImportJobs\" SET \"TotalSources\" = ?2 WHERE \"ImportId\" = ?1",
                params![id, total],
            )?;
            Ok(())
        })
        .await
        .map_err(storage_error)?;

    for (source, fetch_messages) in sources {
        if fetch_messages {
            copy_history(state, &writer, &job.import, &snapshot, bot, source, history).await?;
        }
        writer
            .complete_channel(&job.import, source)
            .await
            .map_err(storage_error)?;
    }

    // The new server is visible only after the access check and final write.
    // If a channel became restricted during the copy, leave it staged.
    let current = plan::catalog(bot, &snapshot.source_id)
        .await
        .map_err(discord_error)?;
    plan::select(current, &job.options)?;
    let report = report_json(writer.tally(server).await.map_err(storage_error)?)?;

    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            let owner_role = tx.query_row(
                "SELECT \"Id\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"IsOwner\" = 1",
                [server],
                |row| row.get(0),
            )?;
            ownership::admit(
                tx,
                server,
                actor,
                owner_role,
                false,
                Some(1),
                Timestamp::now(),
            )?;
            tx.execute(
                "UPDATE \"DiscordImportJobs\" SET \"Status\" = 'completed', \"Report\" = ?2
             WHERE \"ImportId\" = ?1",
                params![id, report],
            )?;
            Ok(())
        })
        .await
        .map_err(storage_error)
}

async fn copy_history(
    state: &AppState,
    writer: &ImportWriter,
    import: &Import,
    snapshot: &ServerSnapshot,
    bot: &BotReader,
    source: &str,
    history: History,
) -> Result<(), &'static str> {
    let server = import.server.ok_or("discord-write-failed")?;
    let source_id = source.to_owned();
    let record = state
        .database()
        .read(move |db| ChannelRecord::find(db, server, &source_id))
        .await
        .map_err(storage_error)?
        .ok_or("discord-write-failed")?;
    if record.completed_at.is_some() {
        return Ok(());
    }
    let since = match history {
        History::Since { since } => Some(since),
        _ => None,
    };
    let mut pages = bot
        .messages(source, record.oldest_message.as_deref())
        .map_err(discord_error)?;
    while let Some(mut page) = pages.next_page().await.map_err(discord_error)? {
        let reached_start = since.is_some_and(|since| {
            page.iter()
                .any(|message| moment(message.created_at) < since)
        });
        page.retain(|message| {
            let created = moment(message.created_at);
            created <= import.started_at && since.is_none_or(|since| created >= since)
        });
        if !page.is_empty() {
            writer
                .write_messages(import, snapshot, source, &page, None)
                .await
                .map_err(storage_error)?;
        }
        if reached_start {
            break;
        }
    }
    Ok(())
}
