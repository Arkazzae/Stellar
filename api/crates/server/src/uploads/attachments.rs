//! The rows that name stored files, and what happens to the objects behind
//! them when a row is not written or no longer wanted.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, Serializer};
use tracing::warn;

use super::form::Staged;
use super::inspect::file_name_of;
use crate::database::DbError;
use crate::messages::dto::{AttachmentDto, Seconds};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// A stored file, as its row has it.
pub struct Attachment {
    pub id: Id,
    pub uploaded_by: Id,
    pub object_key: String,
    pub file_name: String,
    pub content_type: String,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub duration_seconds: Option<f64>,
    pub is_voice_message: bool,
    pub poster: Option<Extra>,
    pub preview: Option<Extra>,
    pub placeholder: Option<String>,
    pub is_encrypted: bool,
}

/// A poster or a preview stored beside a file.
pub struct Extra {
    pub key: String,
    pub content_type: String,
    pub size: i64,
}

impl Attachment {
    pub(super) fn plain_image(id: Id, user: Id, key: String, file: &Staged, content_type: &str) -> Self {
        Self {
            id,
            uploaded_by: user,
            object_key: key,
            file_name: file_name_of(&file.file_name).to_string(),
            content_type: content_type.to_string(),
            size: file.length as i64,
            width: None,
            height: None,
            duration_seconds: None,
            is_voice_message: false,
            poster: None,
            preview: None,
            placeholder: None,
            is_encrypted: false,
        }
    }


    pub fn insert(&self, db: &Connection) -> rusqlite::Result<()> {
        db.execute(
            "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\",
                \"Width\", \"Height\", \"DurationSeconds\", \"IsVoiceMessage\", \"PosterObjectKey\", \"PosterContentType\",
                \"PosterSizeBytes\", \"PreviewObjectKey\", \"PreviewContentType\", \"PreviewSizeBytes\", \"Placeholder\",
                \"IsEncrypted\", \"CreatedAt\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                self.id,
                self.uploaded_by,
                self.object_key,
                self.file_name,
                self.content_type,
                self.size,
                self.width,
                self.height,
                self.duration_seconds,
                self.is_voice_message,
                self.poster.as_ref().map(|extra| &extra.key),
                self.poster.as_ref().map(|extra| &extra.content_type),
                self.poster.as_ref().map(|extra| extra.size),
                self.preview.as_ref().map(|extra| &extra.key),
                self.preview.as_ref().map(|extra| &extra.content_type),
                self.preview.as_ref().map(|extra| extra.size),
                self.placeholder,
                self.is_encrypted,
                Timestamp::now(),
            ],
        )?;
        Ok(())
    }

    pub fn dto(&self) -> AttachmentDto {
        let id = self.id;
        AttachmentDto {
            id,
            name: self.file_name.clone(),
            content_type: self.content_type.clone(),
            size: self.size,
            width: self.width.map(i64::from),
            height: self.height.map(i64::from),
            url: format!("/assets/{id}"),
            duration_seconds: self.duration_seconds.map(Seconds),
            poster_url: self.poster.as_ref().map(|_| format!("/assets/{id}/poster")),
            is_voice_message: self.is_voice_message,
            preview_url: self.preview.as_ref().map(|_| format!("/assets/{id}/preview")),
            placeholder: self.placeholder.clone(),
        }
    }
}

/// Writes the same JSON representation as an attachment duration.
pub(crate) fn number<S: Serializer>(value: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
    value.map(Seconds).serialize(serializer)
}

/// `2026/11`, the month an upload is filed under.
pub fn month() -> String {
    Timestamp::now().to_db_text()[..7].replace('-', "/")
}

/// An id as a key spells it: 32 digits, no hyphens.
pub fn compact(id: Id) -> String {
    id.to_string().replace('-', "")
}

/// Removes objects whose row never made it into the database. One that cannot
/// be deleted now is queued, so that it is retried instead of leaked.
pub async fn roll_back(state: &AppState, keys: Vec<String>) {
    let mut failed = Vec::new();
    for key in keys {
        if let Err(error) = state.storage().delete(&key).await {
            warn!("an object could not be removed after a failed upload: {error}");
            failed.push(key);
        }
    }
    if failed.is_empty() {
        return;
    }
    let queued = state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            for key in &failed {
                crate::housekeeping::queue_deletion(tx, key, Timestamp::now())?;
            }
            Ok(())
        })
        .await;
    if let Err(error) = queued {
        warn!("objects to remove could not be queued: {error}");
    }
}

/// Deletes the attachment behind an `/assets/{id}` address, with its poster and
/// preview, unless something still points at it. With `uploaded_by`, only one
/// that account uploaded qualifies. Says whether it was removed and its
/// objects queued for deletion.
pub async fn retire_if_unreferenced(
    state: &AppState,
    asset_url: Option<String>,
    uploaded_by: Option<Id>,
) -> bool {
    let Some(url) = asset_url else { return false };
    let Some(id) = url.strip_prefix("/assets/").and_then(Id::parse) else {
        return false;
    };
    let retired = state
        .database()
        .write(move |tx| -> Result<bool, DbError> {
            let found: Option<(String, Option<String>, Option<String>)> = tx
                .query_row(
                    "SELECT \"ObjectKey\", \"PosterObjectKey\", \"PreviewObjectKey\" FROM \"ImageAttachments\"
                     WHERE \"Id\" = ?1 AND (?2 IS NULL OR \"UploadedById\" = ?2)",
                    params![id, uploaded_by],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((key, poster, preview)) = found else {
                return Ok(false);
            };
            if is_referenced(tx, id, &url)? {
                return Ok(false);
            }
            for key in [Some(key), poster, preview].into_iter().flatten() {
                crate::housekeeping::queue_deletion(tx, &key, Timestamp::now())?;
            }
            tx.execute("DELETE FROM \"ImageAttachments\" WHERE \"Id\" = ?1", [id])?;
            Ok(true)
        })
        .await;
    match retired {
        Ok(retired) => retired,
        // A reference may have been added since the check. The cleanup is
        // optional, so keeping the attachment is the safe outcome.
        Err(error) => {
            warn!("an attachment is still referenced and was kept: {error}");
            false
        }
    }
}

fn is_referenced(db: &Connection, id: Id, url: &str) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"MessageAttachments\" WHERE \"AttachmentId\" = ?1)
             OR EXISTS (SELECT 1 FROM \"MessageEmbeds\" WHERE \"ThumbnailAttachmentId\" = ?1)
             OR EXISTS (SELECT 1 FROM \"SoundboardSounds\" WHERE \"AttachmentId\" = ?1)
             OR EXISTS (SELECT 1 FROM \"Users\" WHERE \"AvatarUrl\" = ?2 OR \"BannerUrl\" = ?2 OR \"WallpaperUrl\" = ?2)
             OR EXISTS (SELECT 1 FROM \"Servers\" WHERE \"IconUrl\" = ?2)",
        params![id, url],
        |row| row.get(0),
    )
}
