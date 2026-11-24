//! Uploads nobody refers to: a file that was uploaded and never attached to
//! a message, a card, a sound, a picture of a person or of a server.

use std::collections::HashSet;

use rusqlite::params;

use super::Stopping;
use crate::database::DbError;
use crate::state::AppState;
use crate::uploads::retire_if_unreferenced;
use crate::values::{Id, Timestamp};

/// How many candidates are read at a time.
const PAGE: i64 = 1_000;

/// Removes up to `batch` attachments older than `cutoff` that nothing refers
/// to, queueing their objects for deletion, and says how many went.
///
/// Candidates are chosen first and removed one at a time, each in a
/// transaction that looks again whether anything refers to it: a reference
/// made after the candidate was chosen keeps it. Pictures of people and of
/// servers are named by address, not by id, so they are set aside before the
/// batch is counted; otherwise old avatars could fill every batch and
/// indefinitely hide real orphans.
pub async fn sweep(
    state: &AppState,
    cutoff: Timestamp,
    batch: usize,
    stop: Stopping<'_>,
) -> Result<usize, DbError> {
    let protected = pictures_in_use(state).await?;
    let candidates = candidates(state, cutoff, batch, &protected).await?;
    let mut removed = 0;
    for id in candidates {
        if stop() {
            break;
        }
        if retire_if_unreferenced(state, Some(format!("/assets/{id}")), None).await {
            removed += 1;
        }
        super::step_aside().await;
    }
    Ok(removed)
}

/// The ids of every picture a person or a server shows.
async fn pictures_in_use(state: &AppState) -> Result<HashSet<Id>, DbError> {
    state
        .database()
        .read(|connection| -> Result<HashSet<Id>, DbError> {
            let mut found = HashSet::new();
            for query in [
                "SELECT \"AvatarUrl\" FROM \"Users\" UNION ALL SELECT \"BannerUrl\" FROM \"Users\"
                 UNION ALL SELECT \"WallpaperUrl\" FROM \"Users\" UNION ALL SELECT \"IconUrl\" FROM \"Servers\"",
            ] {
                let mut statement = connection.prepare(query)?;
                let rows = statement.query_map([], |row| row.get::<_, Option<String>>(0))?;
                for url in rows.filter_map(Result::ok).flatten() {
                    if let Some(id) = url.strip_prefix("/assets/").and_then(Id::parse) {
                        found.insert(id);
                    }
                }
            }
            Ok(found)
        })
        .await
}

/// The oldest attachments that no message, card, sound or direct message
/// holds, leaving out the pictures in use, up to `batch` of them.
async fn candidates(
    state: &AppState,
    cutoff: Timestamp,
    batch: usize,
    protected: &HashSet<Id>,
) -> Result<Vec<Id>, DbError> {
    let protected = protected.clone();
    state
        .database()
        .read(move |connection| -> Result<Vec<Id>, DbError> {
            let mut statement = connection.prepare(
                "SELECT \"Id\", \"CreatedAt\" FROM \"ImageAttachments\" a
                 WHERE a.\"CreatedAt\" < ?1 AND (a.\"CreatedAt\", a.\"Id\") > (?2, ?3)
                   AND NOT EXISTS (SELECT 1 FROM \"MessageAttachments\" l WHERE l.\"AttachmentId\" = a.\"Id\")
                   AND NOT EXISTS (SELECT 1 FROM \"DirectMessageAttachments\" l WHERE l.\"AttachmentId\" = a.\"Id\")
                   AND NOT EXISTS (SELECT 1 FROM \"MessageEmbeds\" e WHERE e.\"ThumbnailAttachmentId\" = a.\"Id\")
                   AND NOT EXISTS (SELECT 1 FROM \"SoundboardSounds\" s WHERE s.\"AttachmentId\" = a.\"Id\")
                 ORDER BY a.\"CreatedAt\", a.\"Id\" LIMIT ?4",
            )?;
            let mut chosen = Vec::new();
            let (mut after_time, mut after_id) = (String::new(), String::new());
            while chosen.len() < batch {
                let page: Vec<(Id, String, String)> = statement
                    .query_map(params![cutoff, after_time, after_id, PAGE], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get::<_, String>(0)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                let Some((_, last_time, last_id)) = page.last().cloned() else {
                    break;
                };
                (after_time, after_id) = (last_time, last_id);
                let page_length = page.len() as i64;
                for (id, _, _) in page {
                    if !protected.contains(&id) && chosen.len() < batch {
                        chosen.push(id);
                    }
                }
                if page_length < PAGE {
                    break;
                }
            }
            Ok(chosen)
        })
        .await
}
