//! Restore the files of recorded Discord messages using fresh CDN URLs.
//! A single file is staged at a time. The Discord token stays with the API
//! reader and is never sent to the CDN or saved for a later attempt.

use std::collections::HashSet;
use std::time::Duration;

use axum::http::header::{CONTENT_ENCODING, CONTENT_TYPE};
use rusqlite::params;
use stellarr_discord::{
    AttachmentSnapshot, BotReader, MessageSnapshot, ServerSnapshot, is_cdn_url,
};
use stellarr_outbound::{AddressPolicy, Client, Request};
use stellarr_storage::validation::{max_bytes, spec_for};

use super::jobs::{discord_error, storage_error};
use super::journal::json_texts;
use super::{Import, ImportWriter, moment, names, plan};
use crate::database::DbError;
use crate::object_store::staging_dir;
use crate::state::AppState;
use crate::uploads::{self, Attachment, Staged};
use crate::values::{Id, Timestamp};

const HEAD_ROOM: u64 = 64 * 1024;

pub(super) async fn copy(
    state: &AppState,
    writer: &ImportWriter,
    import: &Import,
    bot: &BotReader,
    mut snapshot: ServerSnapshot,
    include_threads: bool,
) -> Result<(), &'static str> {
    if include_threads {
        plan::add_threads(bot, &mut snapshot)
            .await
            .map_err(discord_error)?;
    }
    let allowed: HashSet<&str> = snapshot
        .channels
        .iter()
        .filter(|channel| plan::has_own_messages(channel))
        .map(|channel| channel.id.as_str())
        .chain(snapshot.threads.iter().map(|thread| thread.id.as_str()))
        .collect();
    let import_id = import.id;
    let sources = state.database().read(move |db| -> Result<Vec<(String, Id, Timestamp)>, DbError> {
        let mut statement = db.prepare(
            "SELECT c.\"DiscordId\", c.\"ChannelId\", MIN(m.\"CreatedAt\")
             FROM \"DiscordImportMessages\" m
             JOIN \"DiscordImportChannels\" c ON c.\"ServerId\" = m.\"ServerId\" AND c.\"DiscordId\" = m.\"ChannelDiscordId\"
             WHERE m.\"ImportId\" = ?1 AND m.\"AttachmentsPending\" = 1
               AND (m.\"Outcome\" = 2 OR m.\"MessageId\" IS NOT NULL) AND c.\"ChannelId\" IS NOT NULL
             GROUP BY c.\"DiscordId\", c.\"ChannelId\" ORDER BY MAX(m.\"CreatedAt\") DESC",
        )?;
        Ok(statement.query_map([import_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }).await.map_err(storage_error)?;
    let client = Client::new();
    for (source, channel, oldest) in sources {
        if !allowed.contains(source.as_str()) {
            continue;
        }
        let mut pages = bot.messages(&source, None).map_err(discord_error)?;
        while let Some(page) = pages.next_page().await.map_err(discord_error)? {
            let candidates = json_texts(
                page.iter()
                    .filter(|message| {
                        message.channel_id == source && !message.attachments.is_empty()
                    })
                    .map(|message| message.id.as_str()),
            );
            let pending = state
                .database()
                .read(move |db| -> Result<HashSet<String>, DbError> {
                    let mut statement = db.prepare_cached(
                        "SELECT \"DiscordId\" FROM \"DiscordImportMessages\"
                     WHERE \"ImportId\" = ?1 AND \"AttachmentsPending\" = 1
                       AND (\"Outcome\" = 2 OR \"MessageId\" IS NOT NULL)
                       AND \"DiscordId\" IN (SELECT value FROM json_each(?2))",
                    )?;
                    Ok(statement
                        .query_map(params![import_id, candidates], |row| row.get(0))?
                        .collect::<rusqlite::Result<_>>()?)
                })
                .await
                .map_err(storage_error)?;
            for message in page
                .iter()
                .filter(|message| message.channel_id == source && pending.contains(&message.id))
            {
                let mut complete = true;
                for attachment in &message.attachments {
                    complete &=
                        copy_file(state, writer, import, &client, channel, message, attachment)
                            .await?;
                }
                if complete {
                    let discord_id = message.id.clone();
                    state.database().write(move |tx| -> Result<(), DbError> {
                        tx.execute(
                            "UPDATE \"DiscordImportMessages\" SET \"AttachmentsPending\" = 0
                             WHERE \"ImportId\" = ?1 AND \"DiscordId\" = ?2 AND \"MessageId\" IS NOT NULL",
                            params![import_id, discord_id],
                        )?;
                        Ok(())
                    }).await.map_err(storage_error)?;
                }
            }
            if page
                .iter()
                .any(|message| moment(message.created_at) < oldest)
            {
                break;
            }
        }
        writer
            .complete_channel(import, &source)
            .await
            .map_err(storage_error)?;
    }
    Ok(())
}

async fn copy_file(
    state: &AppState,
    writer: &ImportWriter,
    import: &Import,
    client: &Client,
    channel: Id,
    message: &MessageSnapshot,
    file: &AttachmentSnapshot,
) -> Result<bool, &'static str> {
    let actor = import.started_by.ok_or("discord-write-failed")?;
    let server = import.server.ok_or("discord-write-failed")?;
    let message_id = names::message_id(server, &message.id);
    let id = names::attachment_id(message_id, &file.id);
    let already_stored: bool = state.database().read(move |db| -> Result<_, DbError> {
        Ok(db.query_row(
            "SELECT EXISTS (SELECT 1 FROM \"MessageAttachments\" WHERE \"MessageId\" = ?1 AND \"AttachmentId\" = ?2)",
            params![message_id, id],
            |row| row.get(0),
        )?)
    }).await.map_err(storage_error)?;
    if already_stored {
        return Ok(true);
    }
    let options = state.settings().uploads.options();
    let maximum = [
        options.max_image_bytes,
        options.max_video_bytes,
        options.max_audio_bytes,
        options.max_file_bytes,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let limit = file
        .content_type
        .as_deref()
        .and_then(spec_for)
        .map_or(maximum, |spec| max_bytes(&spec, &options));
    if file.size <= 0 || file.size > limit || !is_cdn_url(Some(&file.url)) {
        return Ok(false);
    }
    let _lease = state
        .admission()
        .enter(state.database(), actor, file.size)
        .await
        .map_err(storage_error)?
        .map_err(|why| match why {
            crate::admission::Refusal::UserQuota => "discord-media-user-quota",
            _ => "discord-media-storage-limit",
        })?;
    let request = Request::get(&file.url)
        .map_err(|_| "discord-unavailable")?
        .policy(AddressPolicy::PublicOnly)
        .decompress(false)
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(90))
        .max_response_bytes(file.size as u64 + HEAD_ROOM);
    let Ok(response) = client.send(request).await else {
        return Ok(false);
    };
    if response.status.as_u16() != 200
        || response
            .headers
            .get(CONTENT_ENCODING)
            .is_some_and(|value| value != "identity")
    {
        return Ok(false);
    }
    let content_type = response
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let declared = file.content_type.as_deref();
    let Some(content_type) = content_type
        .filter(|value| spec_for(value).is_some())
        .or(declared
            .filter(|_| content_type.is_none_or(|value| value == "application/octet-stream")))
    else {
        return Ok(false);
    };
    let Some(spec) = spec_for(content_type) else {
        return Ok(false);
    };
    if file.size > max_bytes(&spec, &options) {
        return Ok(false);
    }
    let staged = Staged::from_download(
        &staging_dir(state.settings()),
        file.file_name.clone(),
        content_type.into(),
        file.size as u64,
        response.body,
    )
    .await
    .map_err(storage_error)?;
    let Some(staged) = staged else {
        return Ok(false);
    };
    let Ok(spec) = uploads::inspect_attachment(&staged, &options) else {
        return Ok(false);
    };
    let nonce = uploads::compact(Id::random());
    let key = format!(
        "{}/{}/{nonce}{}",
        spec.folder,
        uploads::month(),
        spec.extension
    );
    let asset = Attachment {
        id,
        uploaded_by: actor,
        object_key: key.clone(),
        file_name: uploads::safe_file_name(&file.file_name, &spec),
        content_type: spec.content_type.to_owned(),
        size: file.size,
        width: file.width.filter(|width| (1..=32_768).contains(width)),
        height: file.height.filter(|height| (1..=32_768).contains(height)),
        duration_seconds: file
            .duration_seconds
            .filter(|duration| duration.is_finite() && *duration > 0.0),
        is_voice_message: false,
        poster: None,
        preview: None,
        placeholder: None,
        is_encrypted: false,
    };
    let mut source = staged.open().await.map_err(storage_error)?;
    if let Err(error) = state.storage().put(&key, &mut source).await {
        uploads::roll_back(state, vec![key]).await;
        return Err(storage_error(error));
    }
    let result = writer.attach(import, channel, message, asset).await;
    if !matches!(result, Ok(true)) {
        uploads::roll_back(state, vec![key]).await;
    }
    result.map_err(storage_error)
}
