//! Turns the links of one message into the cards under it.
//!
//! Cards are derived data: the message body is all a person wrote. Building
//! replaces the whole set of a message, so running it twice on the same
//! message ends in the same state and never in duplicates.

use rusqlite::{OptionalExtension, params};
use stellarr_embeds::{Fetcher, Preview, PreviewKind, extract_links};
use tracing::{debug, warn};

use super::thumbnails::{NOBODY, discard_thumbnails, roll_back};
use crate::database::DbError;
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

/// Rebuilds the cards of a message and returns them. Empty when the message
/// has no link worth a card, or does not exist any more.
pub async fn build(
    state: &AppState,
    fetcher: &Fetcher,
    message: Id,
) -> Result<Vec<MessageEmbedDto>, DbError> {
    let content = state
        .database()
        .read(move |db| -> Result<Option<String>, DbError> {
            Ok(db
                .query_row(
                    "SELECT \"Content\" FROM \"Messages\" WHERE \"Id\" = ?1",
                    [message],
                    |row| row.get(0),
                )
                .optional()?)
        })
        .await?;
    let Some(content) = content else {
        return Ok(Vec::new());
    };

    let mut built: Vec<Built> = Vec::new();
    let links = extract_links(&content, fetcher.settings().max_per_message);
    for (position, link) in links.iter().enumerate() {
        match fetcher.preview(link).await {
            Ok(preview) => built.push(make(state, preview, position).await),
            // A page that says nothing about itself is worse than the link the
            // reader can already see.
            Err(reason) => debug!("no card for a link: {reason}"),
        }
    }

    let kept: Vec<(Id, i64)> = built
        .iter()
        .map(|card| (card.embed.id, card.position))
        .collect();
    let _ = kept;
    let replaced = replace(state, message, built).await;
    let (embeds, replaced) = match replaced {
        Ok(done) => done,
        Err((error, keys)) => {
            // Images already stored belong to no row, and would stay forever.
            roll_back(state, keys).await;
            return Err(error);
        }
    };
    discard_thumbnails(state, replaced).await;
    Ok(embeds)
}

/// One card from what was read of its link, with its picture copied into our
/// own storage: putting the other host's address in an `img` tag would tell it
/// who reads this channel, and when.
async fn make(state: &AppState, preview: Preview, position: usize) -> Built {
    let picture = match &preview.thumbnail {
        Some(image) => store(state, image).await,
        None => None,
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
            thumbnail_url: picture
                .as_ref()
                .map(|stored| format!("/assets/{}", stored.id)),
        },
        position: i64::try_from(position).unwrap_or(i64::MAX),
        created_at: Timestamp::now(),
        picture,
    }
}

async fn store(state: &AppState, image: &stellarr_embeds::FetchedImage) -> Option<Stored> {
    let id = Id::random();
    let name = image.file_name();
    let extension = name.rfind('.').map_or("", |dot| &name[dot..]);
    let now = Timestamp::now().to_db_text();
    let key = format!(
        "embeds/{}/{}/{}{extension}",
        &now[0..4],
        &now[5..7],
        id.to_string().replace('-', "")
    );
    let mut bytes = image.bytes.as_slice();
    if let Err(error) = state.storage().put(&key, &mut bytes).await {
        warn!("a preview picture could not be stored: {error}");
        return None;
    }
    Some(Stored {
        id,
        key,
        name,
        content_type: image.content_type(),
        size: i64::try_from(image.size()).unwrap_or(i64::MAX),
    })
}

/// Swaps the cards of a message for `built` in one transaction, so that nobody
/// reads it with the old set gone and the new one not there yet. Gives the
/// cards, and the pictures the old ones used; on failure, the keys of the
/// pictures that were stored for nothing.
async fn replace(
    state: &AppState,
    message: Id,
    built: Vec<Built>,
) -> Result<(Vec<MessageEmbedDto>, Vec<Id>), (DbError, Vec<String>)> {
    let keys: Vec<String> = built
        .iter()
        .filter_map(|card| card.picture.as_ref().map(|stored| stored.key.clone()))
        .collect();
    let written = state
        .database()
        .write(move |tx| -> Result<(Vec<MessageEmbedDto>, Vec<Id>), DbError> {
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
            Ok((embeds, replaced))
        })
        .await;
    written.map_err(|error| (error, keys))
}
