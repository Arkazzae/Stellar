//! Turns the links of one message into the cards under it.
//!
//! Cards are derived data: the message body is all a person wrote. Building
//! replaces the whole set of a message, so running it twice on the same
//! message ends in the same state and never in duplicates.

use rusqlite::{OptionalExtension, params};
use stellarr_embeds::{Fetcher, PageCard, Preview, PreviewKind, extract_links};
use tracing::{debug, warn};

use super::imported::{ImportedCard, retry_before};
use super::thumbnails::{NOBODY, discard_thumbnails, roll_back};
use crate::database::DbError;
use crate::http::AttemptLimiter;
use crate::messages::dto::MessageEmbedDto;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// A card, and the picture that was stored for it.
struct Built {
    embed: MessageEmbedDto,
    position: i64,
    created_at: Timestamp,
    picture: Option<Stored>,
}

struct Stored {
    id: Id,
    key: String,
    name: String,
    content_type: &'static str,
    size: i64,
}

struct Source {
    content: String,
    author: Id,
    edited_at: Option<Timestamp>,
    cards: Option<String>,
}

/// Rebuilds the cards of a message and returns them. Empty when the message
/// has no link worth a card, or does not exist any more.
pub async fn build(
    state: &AppState,
    fetcher: &Fetcher,
    message: Id,
    unfurls: &AttemptLimiter,
    imported: bool,
) -> Result<Option<Vec<MessageEmbedDto>>, DbError> {
    let source = state
        .database()
        .read(move |db| -> Result<Option<Source>, DbError> {
            Ok(db
                .query_row(
                    "SELECT m.\"Content\", m.\"AuthorId\", m.\"EditedAt\", p.\"Cards\"
                     FROM \"Messages\" m LEFT JOIN \"DiscordImportPreviews\" p ON p.\"MessageId\" = m.\"Id\"
                     WHERE m.\"Id\" = ?1
                       AND (NOT ?2 OR ((p.\"AttemptedAt\" IS NULL OR p.\"AttemptedAt\" < ?3)
                         AND NOT EXISTS (SELECT 1 FROM \"MessageEmbeds\" e WHERE e.\"MessageId\" = m.\"Id\")))",
                    params![message, imported, retry_before()],
                    |row| Ok(Source { content: row.get(0)?, author: row.get(1)?, edited_at: row.get(2)?, cards: row.get(3)? }),
                )
                .optional()?)
        })
        .await?;
    let Some(source) = source else {
        return Ok(None);
    };

    let mut previews = Vec::new();
    let links = extract_links(&source.content, fetcher.settings().max_per_message);
    let cards: Vec<ImportedCard> = source
        .cards
        .as_deref()
        .and_then(|data| serde_json::from_str(data).ok())
        .unwrap_or_default();
    for (position, link) in links.iter().enumerate() {
        // The history queue is already limited across all imported authors.
        if !imported && !unfurls.try_acquire(source.author) {
            break;
        }
        let address = link.fetch_address();
        let card = cards
            .iter()
            .find(|card| Some(card.url.as_str()) == address.as_deref())
            .map(|card| PageCard {
                title: card.title.clone(),
                description: card.description.clone(),
                image_url: card.image_url.clone(),
                fallback_image_url: None,
                animation_url: card.animation_url.clone(),
            });
        let result = match card {
            Some(card) => match fetcher.preview_with_metadata(link, Some(card)).await {
                Ok(preview) if preview.thumbnail.is_some() || preview.animation.is_some() => {
                    Ok(preview)
                }
                preferred => fetcher.preview(link).await.or(preferred),
            },
            None => fetcher.preview(link).await,
        };
        match result {
            Ok(preview) => previews.push((position, preview)),
            // A page that says nothing about itself is worse than the link the
            // reader can already see.
            Err(reason) => debug!("no card for a link: {reason}"),
        }
    }

    let bytes = previews
        .iter()
        .filter_map(|(_, preview)| {
            preview
                .animation
                .as_ref()
                .map(Vec::len)
                .or_else(|| preview.thumbnail.as_ref().map(|image| image.size()))
        })
        .map(|size| i64::try_from(size).unwrap_or(i64::MAX))
        .fold(0i64, i64::saturating_add);
    // Reserve all pictures for one message with one slot, through the commit.
    let lease = if bytes > 0 {
        state
            .admission()
            .enter_previews(state.database(), bytes)
            .await?
            .ok()
    } else {
        None
    };
    let mut built = Vec::with_capacity(previews.len());
    for (position, mut preview) in previews {
        if lease.is_none() {
            preview.thumbnail = None;
            preview.animation = None;
        }
        built.push(make(state, preview, position).await);
    }
    let keys: Vec<String> = built
        .iter()
        .filter_map(|card| card.picture.as_ref().map(|stored| stored.key.clone()))
        .collect();
    let replaced = replace(state, message, source, built, imported).await;
    drop(lease);
    match replaced {
        Ok(Some((embeds, replaced))) => {
            discard_thumbnails(state, replaced).await;
            Ok(Some(embeds))
        }
        other => {
            // Images already stored belong to no row if the message changed,
            // disappeared or the transaction failed.
            roll_back(state, keys).await;
            other.map(|_| None)
        }
    }
}

/// One card from what was read of its link, with its picture copied into our
/// own storage: putting the other host's address in an `img` tag would tell it
/// who reads this channel, and when.
async fn make(state: &AppState, preview: Preview, position: usize) -> Built {
    let picture = match (&preview.animation, &preview.thumbnail) {
        (Some(bytes), _) => store(state, bytes, "preview.mp4".into(), "video/mp4").await,
        (None, Some(image)) => {
            store(state, &image.bytes, image.file_name(), image.content_type()).await
        }
        _ => None,
    };
    let media_url = picture
        .as_ref()
        .map(|stored| format!("/assets/{}", stored.id));
    let (thumbnail_url, animation_url) = if preview.animation.is_some() {
        (None, media_url)
    } else {
        (media_url, None)
    };
    let id = Id::random();
    Built {
        embed: MessageEmbedDto {
            id,
            url: preview.url,
            kind: if preview.kind == PreviewKind::Video {
                "video"
            } else {
                "link"
            },
            provider: preview.provider,
            provider_id: preview.provider_id,
            start_seconds: preview.start_seconds.map(i64::from),
            domain: preview.domain,
            title: preview.title,
            description: preview.description,
            thumbnail_url,
            animation_url,
        },
        position: i64::try_from(position).unwrap_or(i64::MAX),
        created_at: Timestamp::now(),
        picture,
    }
}

async fn store(
    state: &AppState,
    mut bytes: &[u8],
    name: String,
    content_type: &'static str,
) -> Option<Stored> {
    let id = Id::random();
    let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    let extension = name.rfind('.').map_or("", |dot| &name[dot..]);
    let now = Timestamp::now().to_db_text();
    let key = format!(
        "embeds/{}/{}/{}{extension}",
        &now[0..4],
        &now[5..7],
        id.to_string().replace('-', "")
    );
    if let Err(error) = state.storage().put(&key, &mut bytes).await {
        warn!("a preview picture could not be stored: {error}");
        return None;
    }
    Some(Stored {
        id,
        key,
        name,
        content_type,
        size,
    })
}

/// Swaps the cards of a message for `built` in one transaction, so that nobody
/// reads it with the old set gone and the new one not there yet. Gives the
/// cards and the pictures the old ones used, unless the message changed while
/// the links were being fetched.
async fn replace(
    state: &AppState,
    message: Id,
    source: Source,
    built: Vec<Built>,
    imported: bool,
) -> Result<Option<(Vec<MessageEmbedDto>, Vec<Id>)>, DbError> {
    state
        .database()
        .write(move |tx| -> Result<_, DbError> {
            let current: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"Messages\"
                 WHERE \"Id\" = ?1 AND \"Content\" = ?2 AND \"EditedAt\" IS ?3)",
                params![message, source.content, source.edited_at],
                |row| row.get(0),
            )?;
            if !current {
                return Ok(None);
            }
            let replaced = super::thumbnails::clear_embeds(tx, message)?;
            let mut embeds = Vec::with_capacity(built.len());
            for card in built {
                if let Some(stored) = &card.picture {
                    tx.execute(
                        "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\",
                             \"SizeBytes\", \"Width\", \"Height\", \"DurationSeconds\", \"IsVoiceMessage\", \"PosterObjectKey\",
                             \"PosterContentType\", \"PosterSizeBytes\", \"PreviewObjectKey\", \"PreviewContentType\",
                             \"PreviewSizeBytes\", \"Placeholder\", \"IsEncrypted\", \"CreatedAt\")
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0, ?7)",
                        params![stored.id, NOBODY, stored.key, stored.name, stored.content_type, stored.size, card.created_at],
                    )?;
                }
                let embed = &card.embed;
                tx.execute(
                    "INSERT INTO \"MessageEmbeds\" (\"Id\", \"MessageId\", \"Url\", \"Kind\", \"Provider\", \"ProviderId\", \"StartSeconds\",
                         \"Domain\", \"Title\", \"Description\", \"ThumbnailAttachmentId\", \"Position\", \"CreatedAt\")
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        embed.id,
                        message,
                        embed.url,
                        i64::from(embed.kind == "video"),
                        embed.provider,
                        embed.provider_id,
                        embed.start_seconds,
                        embed.domain,
                        embed.title,
                        embed.description,
                        card.picture.as_ref().map(|stored| stored.id),
                        card.position,
                        card.created_at,
                    ],
                )?;
                embeds.push(card.embed);
            }
            if imported {
                tx.execute(
                    "INSERT INTO \"DiscordImportPreviews\" (\"MessageId\", \"AttemptedAt\") VALUES (?1, ?2)
                     ON CONFLICT (\"MessageId\") DO UPDATE SET \"AttemptedAt\" = excluded.\"AttemptedAt\"",
                    params![message, Timestamp::now()],
                )?;
            }
            Ok(Some((embeds, replaced)))
        })
        .await
}
