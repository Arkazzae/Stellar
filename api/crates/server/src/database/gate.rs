use tokio::sync::{Mutex, MutexGuard};

/// Lets one change of a server-wide invariant at a time through, in this
/// process. A transfer of ownership frees the owner slot and then assigns it,
/// and the database alone cannot stop two of them from interleaving.
#[derive(Debug, Default)]
pub struct MutationGate(Mutex<()>);

impl MutationGate {
    /// Waits for the turn; dropping the result gives it up.
    pub async fn enter(&self) -> MutexGuard<'_, ()> {
        self.0.lock().await
    }
}
