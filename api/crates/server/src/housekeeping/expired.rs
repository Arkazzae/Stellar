//! Records of signing in that are well past use. The durable history of an
//! account's security stays; only what had a lifetime of minutes or days is
//! removed, and only once it is long gone.

use std::time::Duration;

use rusqlite::params;

use super::Stopping;
use super::journal::CHUNK;
use crate::database::DbError;
use crate::state::AppState;
use crate::values::Timestamp;

/// One kind of record, and what makes one of them expired.
struct Kind {
    table: &'static str,
    /// A condition on the row, using `?1` for the moment it must be older than.
    expired: &'static str,
    /// How long after its end a record is kept: a day for a challenge, thirty
    /// for the rest.
    grace_days: u64,
}

const KINDS: [Kind; 7] = [
    Kind {
        table: "AuthChallenges",
        expired: "\"ExpiresAt\" < ?1",
        grace_days: 1,
    },
    Kind {
        table: "EmailOtpCodes",
        expired: "\"ExpiresAt\" < ?1",
        grace_days: 1,
    },
    Kind {
        table: "CliRecoveryCredentials",
        expired: "\"ExpiresAt\" < ?1",
        grace_days: 1,
    },
    Kind {
        table: "PendingExternalIdentities",
        expired: "\"ExpiresAt\" < ?1",
        grace_days: 1,
    },
    Kind {
        table: "TrustedDevices",
        expired: "\"ExpiresAt\" < ?1",
        grace_days: 30,
    },
    Kind {
        table: "RecoveryCodes",
        expired: "\"UsedAt\" IS NOT NULL AND \"UsedAt\" < ?1",
        grace_days: 30,
    },
    Kind {
        table: "SecurityNotifications",
        expired: "\"SentAt\" IS NOT NULL AND \"SentAt\" < ?1",
        grace_days: 30,
    },
];

/// Deletes the expired records of every kind, each kind a few thousand rows
/// at a time in short transactions of their own.
pub async fn delete(state: &AppState, now: Timestamp, stop: Stopping<'_>) -> Result<(), DbError> {
    for kind in KINDS {
        let before = now.minus(Duration::from_secs(kind.grace_days * 86_400));
        loop {
            if stop() {
                return Ok(());
            }
            let removed = state
                .database()
                .write(move |tx| -> Result<usize, DbError> {
                    Ok(tx.execute(
                        &format!(
                            "DELETE FROM \"{table}\" WHERE rowid IN
                               (SELECT rowid FROM \"{table}\" WHERE {expired} LIMIT ?2)",
                            table = kind.table,
                            expired = kind.expired
                        ),
                        params![before, CHUNK],
                    )?)
                })
                .await?;
            if (removed as i64) < CHUNK {
                break;
            }
        }
    }
    Ok(())
}
