//! Records that are well past use: those of signing in, and scheduled
//! messages that could not be sent. The durable history of an account's
//! security stays; only what had a lifetime of minutes or days is removed,
//! and only once it is long gone. A failed scheduled message is kept for its
//! author to see why; once it goes, its files are orphans like any other.

use std::time::Duration;

use super::Stopping;
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

const KINDS: [Kind; 8] = [
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
    Kind {
        table: "ScheduledMessages",
        expired: "\"FailedAt\" IS NOT NULL AND \"FailedAt\" < ?1",
        grace_days: crate::scheduled::FAILED_KEPT_DAYS,
    },
];

/// Deletes the expired records of every kind, each kind a thousand rows
/// at a time in short transactions of their own.
pub async fn delete(state: &AppState, now: Timestamp, stop: Stopping<'_>) -> Result<(), DbError> {
    for kind in KINDS {
        let before = now.minus(Duration::from_secs(kind.grace_days * 86_400));
        super::journal::delete_in_chunks(state, kind.table, "rowid", kind.expired, before, stop)
            .await?;
    }
    Ok(())
}
