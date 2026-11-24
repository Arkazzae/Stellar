//! The store the uploaded files are kept in, chosen by the settings, and the
//! folder files are staged in on their way to it.
//!
//! Handlers and workers reach it as `state.storage()`. It is opened when the
//! state is built and made ready by [`prepare`] before the server listens, as
//! the first server did: a bucket on another host may still be starting, so a
//! store that is not ready is asked again every second, thirty times.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use stellarr_storage::{
    BoxFuture, ByteRange, Error, ObjectStorage, Result, StoreFault, StoredObject,
    open as open_store,
};
use tokio::io::AsyncRead;
use tracing::warn;

use crate::settings::Settings;

/// The folder of the data root that holds files on their way into the store
/// and does nothing else. What is in it when the server starts was left by a
/// request that did not finish.
pub const STAGING_FOLDER: &str = "incoming";

/// How often a store that is not ready is asked, and how long between.
const ATTEMPTS: u32 = 30;
const PAUSE: Duration = Duration::from_secs(1);

pub fn staging_dir(settings: &Settings) -> PathBuf {
    settings.data_root.join(STAGING_FOLDER)
}

/// The store the settings name. One that cannot be opened is replaced by a
/// store that says why on every call, so that the server refuses to start
/// with the reason in front of it instead of failing somewhere else.
pub fn open(settings: &Settings) -> Arc<dyn ObjectStorage> {
    match open_store(
        &settings.storage.options(),
        &settings.data_root,
        stellarr_outbound::Client::new(),
    ) {
        Ok(store) => store,
        Err(error) => Arc::new(Unusable(error.to_string())),
    }
}

/// Makes the store ready: creates what it needs, and clears the staging
/// folder of what an earlier run left. Skipped for a store that tests stand in
/// for (`Storage:SkipInitialization`).
pub async fn prepare(
    store: &dyn ObjectStorage,
    settings: &Settings,
) -> std::result::Result<(), String> {
    let staging = staging_dir(settings);
    clear_staging(&staging)
        .await
        .map_err(|error| format!("cannot prepare {}: {error}", staging.display()))?;
    if settings.storage.skip_initialization {
        return Ok(());
    }
    ask_until_ready(store, ATTEMPTS, PAUSE).await
}

async fn ask_until_ready(
    store: &dyn ObjectStorage,
    attempts: u32,
    pause: Duration,
) -> std::result::Result<(), String> {
    let mut attempt = 1;
    loop {
        match store.prepare().await {
            Ok(()) => return Ok(()),
            Err(error) if attempt < attempts && is_worth_asking_again(&error) => {
                warn!(
                    target: crate::logging::STORE_NOT_READY_TARGET,
                    "The object store is not ready (attempt {attempt} of {attempts}): {error}"
                );
                tokio::time::sleep(pause).await;
                attempt += 1;
            }
            Err(error) => return Err(format!("the object store is not usable: {error}")),
        }
    }
}

/// The first server asked again after every failure; the ones that cannot get
/// better (a refusal of the credentials, an address that is no address) are
/// not worth thirty seconds. A folder that cannot be made may be a volume that
/// is still being mounted, so it is asked again.
fn is_worth_asking_again(error: &Error) -> bool {
    error.is_retryable() || matches!(error, Error::Io(_))
}

async fn clear_staging(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    tokio::fs::create_dir_all(path).await
}

/// What stands in for a store that could not be opened.
struct Unusable(String);

impl Unusable {
    fn error(&self) -> Error {
        Error::Store {
            kind: StoreFault::Rejected,
            message: self.0.clone(),
        }
    }
}

impl ObjectStorage for Unusable {
    fn prepare(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { Err(self.error()) })
    }

    fn put<'a>(
        &'a self,
        _key: &'a str,
        _data: &'a mut (dyn AsyncRead + Send + Unpin),
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { Err(self.error()) })
    }

    fn get<'a>(
        &'a self,
        _key: &'a str,
        _range: Option<ByteRange>,
    ) -> BoxFuture<'a, Result<StoredObject>> {
        Box::pin(async move { Err(self.error()) })
    }

    fn delete<'a>(&'a self, _key: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { Err(self.error()) })
    }

    fn list(&self) -> stellarr_storage::Listing {
        stellarr_storage::Listing::of(Vec::new())
    }

    fn probe(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { Err(self.error()) })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// A store that is not ready for `failures` calls, with the fault given.
    struct Slow {
        calls: AtomicU32,
        failures: u32,
        fault: StoreFault,
    }

    impl ObjectStorage for Slow {
        fn prepare(&self) -> BoxFuture<'_, Result<()>> {
            Box::pin(async move {
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                if call < self.failures {
                    Err(Error::Store {
                        kind: self.fault,
                        message: "not yet".to_string(),
                    })
                } else {
                    Ok(())
                }
            })
        }
        fn put<'a>(
            &'a self,
            _: &'a str,
            _: &'a mut (dyn AsyncRead + Send + Unpin),
        ) -> BoxFuture<'a, Result<()>> {
            unused_by_the_test()
        }
        fn get<'a>(
            &'a self,
            _: &'a str,
            _: Option<ByteRange>,
        ) -> BoxFuture<'a, Result<StoredObject>> {
            unused_by_the_test()
        }
        fn delete<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<()>> {
            unused_by_the_test()
        }
        fn list(&self) -> stellarr_storage::Listing {
            stellarr_storage::Listing::of(Vec::new())
        }
        fn probe(&self) -> BoxFuture<'_, Result<()>> {
            unused_by_the_test()
        }
    }

    fn unused_by_the_test<T: Send + 'static>() -> BoxFuture<'static, Result<T>> {
        Box::pin(async { Err(Error::NotFound("not used by this test".to_string())) })
    }

    fn slow(failures: u32, fault: StoreFault) -> Slow {
        Slow {
            calls: AtomicU32::new(0),
            failures,
            fault,
        }
    }

    #[tokio::test]
    async fn a_store_that_is_not_ready_is_asked_again() {
        let store = slow(3, StoreFault::NotReady);

        ask_until_ready(&store, 5, Duration::from_millis(1))
            .await
            .unwrap();

        assert_eq!(store.calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn a_store_that_never_gets_ready_is_given_up_on_after_the_attempts() {
        let store = slow(100, StoreFault::Unreachable);

        let refused = ask_until_ready(&store, 5, Duration::from_millis(1))
            .await
            .unwrap_err();

        assert_eq!(store.calls.load(Ordering::SeqCst), 5);
        assert!(refused.contains("not yet"), "{refused}");
    }

    #[tokio::test]
    async fn a_refusal_that_cannot_get_better_is_not_asked_again() {
        let store = slow(100, StoreFault::Denied);

        ask_until_ready(&store, 5, Duration::from_millis(1))
            .await
            .unwrap_err();

        assert_eq!(store.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_staging_folder_is_emptied_at_start() {
        let root =
            std::env::temp_dir().join(format!("stellarr-staging-{}", crate::values::Id::random()));
        let staging = root.join(STAGING_FOLDER);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("left-behind"), b"x").unwrap();

        clear_staging(&staging).await.unwrap();

        assert!(staging.is_dir());
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
