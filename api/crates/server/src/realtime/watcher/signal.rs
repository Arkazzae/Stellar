//! What wakes the watcher: the API's own access changes, in memory, and those
//! of other processes, through the token file they write (see
//! [`crate::database::AccessSignal`]).
//!
//! The file is read every quarter of a second while sockets are open. That is
//! one small read, no SQL, and it works the same wherever the data folder is
//! mounted; an operator's change reaches the sockets within that time.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::watch;

use crate::database::Database;

/// How often the token file of other processes is read.
const POLL: Duration = Duration::from_millis(250);

/// Who woke the listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Wake {
    /// This process, after its transaction committed.
    Local,
    /// Another process, which may still be committing.
    File,
}

pub(super) struct SignalListener {
    local: Option<watch::Receiver<()>>,
    database: Database,
    file: PathBuf,
    token: Vec<u8>,
}

impl SignalListener {
    /// Starts listening. Everything committed after this wakes
    /// [`SignalListener::changed`]; what came before is for the caller to read.
    pub fn new(database: &Database) -> Self {
        let file = database.wake_file();
        Self {
            local: database.access_signal.subscribe(),
            database: database.clone(),
            token: read_token(&file).unwrap_or_default(),
            file,
        }
    }

    /// Returns once access may have changed since the last return.
    pub async fn changed(&mut self) -> Wake {
        loop {
            let local = async {
                let open = match &mut self.local {
                    Some(local) => local.changed().await.is_ok(),
                    None => false,
                };
                if !open {
                    // The database is closing: only the file is left.
                    std::future::pending::<()>().await;
                }
            };
            tokio::select! {
                () = local => return Wake::Local,
                () = tokio::time::sleep(POLL) => {}
            }
            if self.token_changed().await {
                return Wake::File;
            }
        }
    }

    async fn token_changed(&mut self) -> bool {
        let database = self.database.clone();
        let file = self.file.clone();
        let read = tokio::task::spawn_blocking(move || match read_token(&file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Removed under a running API: made again, so that the next
                // operator's change can be signalled.
                if let Err(error) = database.prepare_wake_file() {
                    tracing::warn!(target: "Stellarr.Api.Realtime", %error, "Cannot recreate the access signal file");
                }
                Ok(Vec::new())
            }
            read => read,
        })
        .await;
        match read {
            // Empty while somebody truncates the file to write it.
            Ok(Ok(token)) if !token.is_empty() && token != self.token => {
                self.token = token;
                true
            }
            _ => false,
        }
    }
}

fn read_token(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut token = Vec::new();
    std::fs::File::open(path)?
        .take(128)
        .read_to_end(&mut token)?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DbError;
    use crate::testing::new_database;

    async fn quiet(listener: &mut SignalListener) -> bool {
        tokio::time::timeout(Duration::from_millis(600), listener.changed())
            .await
            .is_err()
    }

    #[tokio::test]
    async fn only_access_changes_and_a_new_token_wake_the_listener() {
        let (_folder, db) = new_database();
        let mut listener = SignalListener::new(&db);

        db.write(|tx| tx.execute_batch("CREATE TABLE Noise(value); INSERT INTO Noise VALUES (1)"))
            .await
            .unwrap();
        assert!(quiet(&mut listener).await, "an unrelated write woke it");

        db.write(|tx| {
            crate::auth::fixtures::add_user(tx, None);
            Ok::<_, DbError>(())
        })
        .await
        .unwrap();
        let woke = tokio::time::timeout(Duration::from_millis(100), listener.changed()).await;
        assert_eq!(
            woke,
            Ok(Wake::Local),
            "the API's own access change wakes it at once"
        );
        assert!(quiet(&mut listener).await);

        std::fs::write(db.wake_file(), b"maintenance-1").unwrap();
        let woke = tokio::time::timeout(Duration::from_secs(2), listener.changed()).await;
        assert_eq!(woke, Ok(Wake::File), "another process's token wakes it");
        assert!(quiet(&mut listener).await, "the same token woke it twice");
    }
}
