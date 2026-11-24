//! Counting what an import has met, from the bookkeeping alone.

use rusqlite::params;

use super::{ImportError, ImportWriter, Outcome, SkipReason, Tally};
use crate::values::Id;

impl ImportWriter {
    /// Everything the imports of a server have met so far: what was written, what
    /// waits for attachments, what was left out and why, and how many placeholders
    /// the imports made. The numbers are counted afresh, so they are the same
    /// however the import was interrupted and continued.
    pub async fn tally(&self, server: Id) -> Result<Tally, ImportError> {
        Ok(self
            .database
            .read(move |db| -> rusqlite::Result<Tally> {
                let mut statement = db.prepare_cached(
                    "SELECT \"Outcome\", \"Reason\", \"DiscordType\", \"AttachmentsPending\", COUNT(*)
                       FROM \"DiscordImportMessages\" WHERE \"ServerId\" = ?1
                      GROUP BY \"Outcome\", \"Reason\", \"DiscordType\", \"AttachmentsPending\"",
                )?;
                let groups: Vec<(Outcome, i64, i64, bool, i64)> = statement
                    .query_map(params![server], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                let mut tally = Tally::default();
                for (outcome, reason, kind, pending, count) in groups {
                    let count = count as usize;
                    match outcome {
                        Outcome::Written => {
                            tally.written += count;
                            if pending {
                                tally.attachments_pending += count;
                            }
                        }
                        Outcome::AwaitingAttachments => tally.awaiting_attachments += count,
                        Outcome::Skipped => {
                            let reason = SkipReason::from_number(reason);
                            *tally.skipped.entry(reason).or_insert(0) += count;
                            if reason == SkipReason::Notice {
                                *tally.notices_by_type.entry(kind as i32).or_insert(0) += count;
                            }
                        }
                    }
                }
                tally.placeholders = db.query_row::<i64, _, _>(
                    "SELECT COUNT(*) FROM \"DiscordImportAccounts\" a
                      WHERE EXISTS (SELECT 1 FROM \"DiscordImports\" i WHERE i.\"Id\" = a.\"ImportId\" AND i.\"ServerId\" = ?1)",
                    params![server],
                    |row| row.get(0),
                )? as usize;
                Ok(tally)
            })
            .await?)
    }
}
