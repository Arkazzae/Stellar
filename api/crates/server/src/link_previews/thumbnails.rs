//! The preview pictures of cards: copies in our own storage, uploaded by
//! nobody, that go when the cards that use them do.

use rusqlite::{Connection, OptionalExtension, params};
use tracing::warn;

use crate::database::DbError;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The account a preview picture is credited to: nobody. The row exists so
/// that the picture can be served, and naming the sender would charge a link
/// they pasted against their upload quota.
pub const NOBODY: Id = Id::from_bytes([0; 16]);

/// Removes the cards of a message and says which pictures they used, so that
/// the pictures nothing else uses can be discarded after the change is
/// committed.
pub fn clear_embeds(db: &Connection, message: Id) -> rusqlite::Result<Vec<Id>> {
    let pictures: Vec<Id> = {
        let mut statement = db.prepare_cached(
            "SELECT DISTINCT \"ThumbnailAttachmentId\" FROM \"MessageEmbeds\"
             WHERE \"MessageId\" = ?1 AND \"ThumbnailAttachmentId\" IS NOT NULL
             ORDER BY \"ThumbnailAttachmentId\"",
        )?;
        statement
            .query_map([message], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    db.prepare_cached("DELETE FROM \"MessageEmbeds\" WHERE \"MessageId\" = ?1")?
        .execute([message])?;
    Ok(pictures)
}

/// Deletes the preview pictures that no card, attachment, avatar or icon uses
/// any more, from the database and from storage.
pub async fn discard_thumbnails(state: &AppState, pictures: Vec<Id>) {
    for picture in pictures {
        // The object is queued for deletion together with its row, so that an
        // unreachable store cannot leave it behind. It is then deleted at
        // once and taken off the queue.
        let queued = state
            .database()
            .write(move |tx| -> Result<Option<String>, DbError> {
                Ok(queue_if_unreferenced(tx, picture)?)
            })
            .await;
        let key = match queued {
            Ok(Some(key)) => key,
            Ok(None) => continue,
            Err(error) => {
                warn!("a preview picture could not be discarded: {error}");
                continue;
            }
        };
        match state.storage().delete(&key).await {
            Ok(()) => {
                let removed = state
                    .database()
                    .write(move |tx| -> Result<(), DbError> {
                        tx.execute(
                            "DELETE FROM \"ObjectDeletions\" WHERE \"ObjectKey\" = ?1",
                            [&key],
                        )?;
                        Ok(())
                    })
                    .await;
                if let Err(error) = removed {
                    warn!("a deleted preview picture is still queued: {error}");
                }
            }
            Err(error) => warn!("a preview picture's object could not be deleted: {error}"),
        }
    }
}

/// Removes the row of a picture that only we stored, when nothing points at it,
/// and queues its objects for deletion. Gives the key of the picture's object.
fn queue_if_unreferenced(tx: &Connection, picture: Id) -> rusqlite::Result<Option<String>> {
    let Some((key, poster, preview)) = tx
        .query_row(
            "SELECT \"ObjectKey\", \"PosterObjectKey\", \"PreviewObjectKey\" FROM \"ImageAttachments\"
             WHERE \"Id\" = ?1 AND \"UploadedById\" = ?2",
            params![picture, NOBODY],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let url = format!("/assets/{picture}");
    let referenced: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"MessageAttachments\" WHERE \"AttachmentId\" = ?1)
             OR EXISTS (SELECT 1 FROM \"MessageEmbeds\" WHERE \"ThumbnailAttachmentId\" = ?1)
             OR EXISTS (SELECT 1 FROM \"SoundboardSounds\" WHERE \"AttachmentId\" = ?1)
             OR EXISTS (SELECT 1 FROM \"Users\" WHERE \"AvatarUrl\" = ?2 OR \"BannerUrl\" = ?2 OR \"WallpaperUrl\" = ?2)
             OR EXISTS (SELECT 1 FROM \"Servers\" WHERE \"IconUrl\" = ?2)",
        params![picture, url],
        |row| row.get(0),
    )?;
    if referenced {
        return Ok(None);
    }
    // A reference may have been added since the check; the cleanup is
    // optional, so keeping the picture is the safe outcome.
    match tx.execute(
        "DELETE FROM \"ImageAttachments\" WHERE \"Id\" = ?1",
        [picture],
    ) {
        Ok(_) => {}
        Err(error) if is_constraint(&error) => return Ok(None),
        Err(error) => return Err(error),
    }
    let now = Timestamp::now();
    let mut keys = vec![key.clone()];
    for extra in [poster, preview].into_iter().flatten() {
        if !keys.contains(&extra) {
            keys.push(extra);
        }
    }
    for object in keys {
        crate::housekeeping::queue_deletion(tx, &object, now)?;
    }
    Ok(Some(key))
}

fn is_constraint(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

/// Queues objects whose row never made it into the database, for the cleanup
/// to retry, after deleting what can be deleted now.
pub async fn roll_back(state: &AppState, keys: Vec<String>) {
    let mut failed = Vec::new();
    for key in keys {
        if let Err(error) = state.storage().delete(&key).await {
            warn!("an unused preview picture could not be deleted: {error}");
            failed.push(key);
        }
    }
    if failed.is_empty() {
        return;
    }
    let queued = state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            let now = Timestamp::now();
            for key in failed {
                crate::housekeeping::queue_deletion(tx, &key, now)?;
            }
            Ok(())
        })
        .await;
    if let Err(error) = queued {
        warn!("unused preview pictures could not be queued for deletion: {error}");
    }
}
