//! The outer layer for account key backups. The full ZIP contains the
//! database and the ordinary instance master key. This independent key lives
//! outside every directory that the archiver reads or restore replaces.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::store::MAX_FAILED_ATTEMPTS;
use super::wire::{BackupStatusDto, NewBackup};
use crate::database::DbError;
use crate::seal::Sealer;
use crate::values::{Id, Timestamp};

const KEY_FOLDER: &str = "pin-recovery";
const PURPOSE: &str = "e2ee/account-key-backup/v1";

pub(super) struct Backup {
    pub payload: NewBackup,
    failed_attempts: i32,
    updated_at: Timestamp,
}

impl Backup {
    pub fn remaining_attempts(&self) -> i32 {
        (MAX_FAILED_ATTEMPTS - self.failed_attempts).max(0)
    }

    pub fn status(&self) -> BackupStatusDto {
        BackupStatusDto {
            iterations: self.payload.iterations,
            failed_attempts: self.failed_attempts,
            remaining_attempts: self.remaining_attempts(),
            updated_at: self.updated_at,
        }
    }
}

pub(super) fn uses_code(db: &Connection, user: Id) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM CodeRecoveryAccounts WHERE UserId = ?1)",
        [user],
        |r| r.get(0),
    )
}

pub(super) fn archived_key(db: &Connection, user: Id) -> rusqlite::Result<Option<Id>> {
    Ok(db
        .query_row(
            "SELECT ArchivedKeyId FROM CodeRecoveryAccounts WHERE UserId = ?1",
            [user],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

fn open(sealer: &Sealer, user: Id, sealed: &str) -> Result<NewBackup, DbError> {
    let bytes = sealer
        .open(PURPOSE, user.as_bytes(), sealed, Timestamp::now())
        .map_err(|_| DbError::Task("cannot authenticate the sealed E2EE key backup".into()))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| DbError::Task("invalid sealed E2EE key backup".into()))
}

pub(super) fn get(db: &Connection, sealer: &Sealer, user: Id) -> Result<Option<Backup>, DbError> {
    let row = db.query_row(
        "SELECT Sealed, FailedAttempts, UpdatedAt FROM ProtectedAccountKeyBackups WHERE UserId = ?1",
        [user],
        |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)),
    ).optional()?;
    row.map(|(sealed, failed_attempts, updated_at)| {
        Ok(Backup {
            payload: open(sealer, user, &sealed)?,
            failed_attempts,
            updated_at,
        })
    })
    .transpose()
}

pub(super) fn put(
    db: &Connection,
    sealer: &Sealer,
    user: Id,
    backup: &NewBackup,
    failed_attempts: i32,
    updated_at: Timestamp,
) -> Result<(), DbError> {
    if backup.version == 2 {
        db.execute(
            "INSERT OR IGNORE INTO CodeRecoveryAccounts (UserId) VALUES (?1)",
            [user],
        )?;
    }
    let payload = serde_json::to_vec(backup).map_err(|e| DbError::Task(e.to_string()))?;
    let sealed = sealer.seal(PURPOSE, user.as_bytes(), None, &payload);
    db.execute(
        "INSERT INTO ProtectedAccountKeyBackups (UserId, Sealed, FailedAttempts, UpdatedAt)
         VALUES (?1, ?2, ?3, ?4) ON CONFLICT(UserId) DO UPDATE SET
         Sealed = excluded.Sealed, FailedAttempts = excluded.FailedAttempts, UpdatedAt = excluded.UpdatedAt",
        params![user, sealed, failed_attempts, updated_at],
    )?;
    Ok(())
}

/// A restore with protected rows requires the separately saved key. Check
/// before swapping live directories. A legacy archive can be migrated on boot.
pub(crate) fn matching_key(db: &Connection, root: &Path) -> Result<Option<Sealer>, DbError> {
    if !db.table_exists(None, "ProtectedAccountKeyBackups")? {
        return Ok(None);
    }
    let sample = db
        .query_row(
            "SELECT UserId, Sealed FROM ProtectedAccountKeyBackups LIMIT 1",
            [],
            |r| Ok((r.get::<_, Id>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((user, sealed)) = sample else {
        return Ok(None);
    };
    let folder = root.join(KEY_FOLDER);
    let required = || {
        DbError::Task(format!(
            "restore the matching {} from its separate backup to recover the backed-up account keys",
            folder.join("master.key").display(),
        ))
    };
    let sealer = Sealer::open_existing(&folder)
        .map_err(|e| DbError::Task(e.to_string()))?
        .ok_or_else(required)?;
    open(&sealer, user, &sealed).map_err(|_| required())?;
    Ok(Some(sealer))
}

/// An older snapshot may precede a user's migration. Do not re-enable that PIN on this instance.
pub(crate) fn check_restore(live: &Connection, staged: &Connection) -> Result<(), DbError> {
    if !live.table_exists(None, "RetiredPinAccounts")? {
        return Ok(());
    }
    let has_accounts = staged.table_exists(None, "AccountKeys")?;
    let marked = staged.table_exists(None, "CodeRecoveryAccounts")?;
    let mut retired = live.prepare("SELECT UserId FROM RetiredPinAccounts")?;
    for user in retired.query_map([], |r| r.get::<_, Id>(0))? {
        let user = user?;
        if has_accounts
            && staged.query_row(
                "SELECT EXISTS(SELECT 1 FROM AccountKeys WHERE UserId = ?1)",
                [user],
                |r| r.get::<_, bool>(0),
            )?
            && !(marked && uses_code(staged, user)?)
        {
            return Err(DbError::Task("this archive would reactivate a retired E2EE PIN identity; restore an archive made after recovery-code migration".into()));
        }
        // Carry retirement through a restore that predates the user's signup.
        // Otherwise a second restore could silently reactivate the PIN account.
        staged.execute_batch(
            "CREATE TABLE IF NOT EXISTS RetiredPinAccounts (UserId TEXT PRIMARY KEY)",
        )?;
        staged.execute(
            "INSERT OR IGNORE INTO RetiredPinAccounts (UserId) VALUES (?1)",
            [user],
        )?;
    }
    Ok(())
}

pub(crate) fn initialize(db: &mut Connection, root: &Path) -> Result<Sealer, DbError> {
    db.pragma_update(None, "secure_delete", true)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let sealer = match matching_key(&tx, root)? {
        Some(key) => key,
        None => {
            let folder = root.join(KEY_FOLDER);
            let path = folder.join("master.key");
            let creating = !path
                .try_exists()
                .map_err(|e| DbError::Task(e.to_string()))?;
            let key = Sealer::load_or_create(&folder).map_err(|e| DbError::Task(e.to_string()))?;
            // Persist both directory entries before committing records that need this key.
            for dir in [folder.as_path(), root] {
                std::fs::File::open(dir)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| DbError::Task(format!("cannot sync {}: {e}", dir.display())))?;
            }
            if creating {
                tracing::info!(target: "Stellarr.Api.Backups",
                    "Save {} separately to recover E2EE key backups when moving a ZIP to another host.",
                    path.display());
            }
            key
        }
    };

    let legacy = tx.table_exists(None, "AccountKeyBackups")?;
    if legacy {
        copy_legacy_backups(&tx, &sealer)?;
        tx.execute_batch(
            "DELETE FROM AccountKeyBackups;
             ALTER TABLE AccountKeyBackups RENAME TO RetiredAccountKeyBackups;",
        )?;
    }
    let scrub = legacy
        || tx.table_exists(None, "RetiredAccountKeyBackups")?
        || tx.table_exists(None, "RecoveryCodeUpgradeScrub")?;
    tx.commit()?;

    // Online snapshots copy free pages as well. Rebuild once to remove old PIN
    // material, including already replaced/deleted rows. The empty legacy table
    // is the retry marker if the process stops or runs out of space here.
    if scrub {
        db.execute_batch("VACUUM; DROP TABLE IF EXISTS RetiredAccountKeyBackups; DROP TABLE IF EXISTS RecoveryCodeUpgradeScrub;")?;
    }
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(sealer)
}

fn copy_legacy_backups(tx: &rusqlite::Transaction, sealer: &Sealer) -> Result<(), DbError> {
    let mut stmt = tx.prepare(
        "SELECT UserId, Salt, Iterations, Verifier, Iv, Ciphertext, FailedAttempts, UpdatedAt
         FROM AccountKeyBackups",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let user = row.get(0)?;
        // Import only an unmigrated account, including when recovering an interrupted upgrade.
        if uses_code(tx, user)? {
            continue;
        }
        let payload = NewBackup {
            version: 1,
            salt: row.get(1)?,
            iterations: row.get(2)?,
            verifier: row.get(3)?,
            iv: row.get(4)?,
            ciphertext: row.get(5)?,
        };
        put(tx, sealer, user, &payload, row.get(6)?, row.get(7)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restoring_an_empty_instance_cannot_clear_pin_retirement() {
        let migrated = Connection::open_in_memory().unwrap();
        migrated
            .execute_batch("CREATE TABLE RetiredPinAccounts (UserId TEXT PRIMARY KEY)")
            .unwrap();
        let user = Id::random();
        migrated
            .execute("INSERT INTO RetiredPinAccounts VALUES (?1)", [user])
            .unwrap();
        let before_signup = Connection::open_in_memory().unwrap();
        check_restore(&migrated, &before_signup).unwrap();

        let before_migration = Connection::open_in_memory().unwrap();
        before_migration
            .execute_batch("CREATE TABLE AccountKeys (UserId TEXT PRIMARY KEY)")
            .unwrap();
        before_migration
            .execute("INSERT INTO AccountKeys VALUES (?1)", [user])
            .unwrap();
        assert!(check_restore(&before_signup, &before_migration).is_err());
    }
}
