//! Imported previews are built when their transcript is read. The importer
//! only saves metadata Discord already returned for links in the message.

use std::time::Duration;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use stellarr_discord::EmbedSummary;
use stellarr_embeds::{
    Link, MAX_DESCRIPTION, MAX_TITLE, clean_text, extract_links, gif_animation_url,
};
use stellarr_outbound::Url;

use super::PreviewRequest;
use crate::database::DbError;
use crate::messages::dto::{MessageDto, json_ids};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const RETRY_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ImportedCard {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub animation_url: Option<String>,
}

pub(super) fn retry_before() -> Timestamp {
    Timestamp::now().minus(RETRY_AFTER)
}

pub(crate) fn remember_imported(
    db: &Connection,
    message: Id,
    content: &str,
    embeds: &[EmbedSummary],
) -> Result<(), DbError> {
    if embeds.is_empty() {
        return Ok(());
    }
    let links = extract_links(content, 10);
    let mut cards = Vec::new();
    for embed in embeds.iter().take(10) {
        let Some(link) = embed.url.as_deref().and_then(Link::parse) else {
            continue;
        };
        if !links
            .iter()
            .any(|posted| posted.fetch_address() == link.fetch_address())
        {
            continue;
        }
        let image_url = [embed.image_proxy_url.as_deref(), embed.image_url.as_deref()]
            .into_iter()
            .flatten()
            .find(|address| address.len() <= 4096 && discord_media(address));
        let card = ImportedCard {
            url: link.fetch_address().unwrap_or_else(|| link.to_string()),
            title: clean_text(embed.title.as_deref(), MAX_TITLE),
            description: clean_text(embed.description.as_deref(), MAX_DESCRIPTION),
            image_url: image_url.map(str::to_owned),
            animation_url: embed
                .video_url
                .as_deref()
                .filter(|address| address.len() <= 4096)
                .and_then(|address| gif_animation_url(&link, address)),
        };
        if card.title.is_some()
            || card.description.is_some()
            || card.image_url.is_some()
            || card.animation_url.is_some()
        {
            cards.push(card);
        }
    }
    if !cards.is_empty() {
        let data =
            serde_json::to_string(&cards).map_err(|error| DbError::Task(error.to_string()))?;
        db.execute(
            "INSERT INTO \"DiscordImportPreviews\" (\"MessageId\", \"Cards\") VALUES (?1, ?2)",
            params![message, data],
        )?;
    }
    Ok(())
}

/// Keep import hints to Discord's media endpoints. Fetcher still checks
/// addresses, redirects, size and the actual image format.
fn discord_media(address: &str) -> bool {
    let Ok(url) = Url::parse(address) else {
        return false;
    };
    url.is_https()
        && url.port() == 443
        && matches!(
            url.host(),
            "cdn.discordapp.com"
                | "media.discordapp.net"
                | "images-ext-1.discordapp.net"
                | "images-ext-2.discordapp.net"
        )
}

pub(crate) async fn enqueue_imported(state: &AppState, channel: Id, page: &[MessageDto]) {
    let settings = &state.settings().embeds;
    if !settings.enabled || settings.max_per_message == 0 {
        return;
    }
    let ids: Vec<Id> = page
        .iter()
        .rev()
        .filter(|message| {
            message.embeds.is_empty() && !extract_links(&message.content, 1).is_empty()
        })
        .map(|message| message.id)
        .collect();
    if ids.is_empty() {
        return;
    }
    let candidates = state
        .database()
        .read(move |db| -> Result<Vec<Id>, DbError> {
            let mut statement = db.prepare_cached(
                "SELECT i.\"MessageId\" FROM json_each(?1) j
             JOIN \"DiscordImportMessages\" i ON i.\"MessageId\" = j.value
             LEFT JOIN \"DiscordImportPreviews\" p ON p.\"MessageId\" = i.\"MessageId\"
             WHERE p.\"AttemptedAt\" IS NULL OR p.\"AttemptedAt\" < ?2
             ORDER BY j.key",
            )?;
            Ok(statement
                .query_map(params![json_ids(&ids), retry_before()], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await;
    match candidates {
        Ok(ids) => {
            for message in ids {
                state
                    .previews()
                    .enqueue_imported(PreviewRequest { channel, message });
            }
        }
        Err(error) => tracing::warn!("could not schedule imported link previews: {error}"),
    }
}
