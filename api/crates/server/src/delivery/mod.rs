//! Telling people who are away: Web Push and mail, and the three routes a
//! browser subscribes with.
//!
//! - `GET /api/me/notification-delivery` says what this instance can offer the
//!   signed-in person: whether mail is set up, whether their address is
//!   verified, and the key a browser subscribes to push with.
//! - `PUT` and `DELETE /api/me/push-subscriptions` add and remove a browser.
//! - [`outbox`] writes the rows that say "this person is owed a notification
//!   about this message". The routes that post a message call it inside their
//!   own transaction, after the message is stored; nothing is sent there.
//! - [`dispatcher`] is what the worker `notification-delivery` runs every
//!   thirty seconds: it leases the rows that are due, checks again that each
//!   is still wanted, sends it by push or mail, and records what became of it.
//!
//! Push is on when the operator configured a contact and a key pair
//! (`STELLARR_PUSH_*`); a missing or wrong pair makes push unavailable and
//! stops nothing else. The instance's key is the operator's to keep: nothing
//! is stored in the database for it.
//!
//! The address policy of [`sender`] (only public addresses) is fixed in the
//! code: no setting changes it, and the only way a test reaches a fake push
//! service on the loopback is a [`sender::Wiring`] that names the policy for
//! that one sender, which the running server never builds.

pub mod dispatcher;
pub mod outbox;
pub mod sender;
mod subscriptions;
mod target;
mod vapid;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use tracing::warn;

use crate::background::Worker;
use crate::http::Endpoints;
use crate::values::Timestamp;

pub use sender::Sender;
pub use vapid::keys as vapid_keys;
pub(crate) use vapid::provision;

const TARGET: &str = "Stellarr.Api.Notifications";

/// How long the worker waits between two looks at the outbox.
pub const INTERVAL: Duration = Duration::from_secs(30);

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/me/notification-delivery", subscriptions::capabilities)
        .authorized()
        .get("/api/me/notification-delivery/{id}", target::resolve)
        .authorized()
        .put("/api/me/push-subscriptions", subscriptions::save)
        .json_body()
        .authorized()
        .delete("/api/me/push-subscriptions", subscriptions::remove)
        .json_body()
        .authorized()
}

pub fn workers() -> Vec<Worker> {
    workers_with(None)
}

/// The worker with a sender chosen by the caller: a test hands it one wired to
/// a fake push service. `None` is the sender the settings describe. For tests
/// only: the server runs [`workers`].
#[doc(hidden)]
pub fn workers_with(sender: Option<Arc<Sender>>) -> Vec<Worker> {
    vec![Worker::new("notification-delivery", move |state, stop| {
        let sender = sender.clone();
        async move {
            let sender = sender.unwrap_or_else(|| Arc::new(Sender::new(&state.settings().push)));
            loop {
                // A look that fails is logged, and the next one tries again.
                let result =
                    dispatcher::run(&state, &sender, || stop.is_stopping(), Timestamp::now()).await;
                if let Err(error) = result {
                    warn!(target: TARGET, "The activity notification queue could not be processed: {error}");
                }
                if !stop.sleep(INTERVAL).await {
                    break;
                }
            }
        }
    })]
}
