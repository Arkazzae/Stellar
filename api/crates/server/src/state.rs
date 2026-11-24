//! What every handler can reach: the settings, the database, the catalogue of
//! messages and the other shared services of the process.
//!
//! To add a shared service: put it in a field of `Inner`, build it in `new`
//! and give it an accessor. Everything else reaches it through
//! `State(state): State<AppState>`.

use std::sync::Arc;

use stellarr_messages::Messages;

use crate::database::{Database, MutationGate};
use crate::events::EventPublisher;
use crate::http::{AttemptLimiter, Metrics};
use crate::seal::Sealer;
use crate::settings::Settings;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    settings: Settings,
    messages: Messages,
    database: Database,
    events: Arc<dyn EventPublisher>,
    metrics: Metrics,
    mutation_gate: MutationGate,
    pin_attempts: AttemptLimiter,
    sealer: Sealer,
}

impl AppState {
    pub fn new(
        settings: Settings,
        database: Database,
        events: Arc<dyn EventPublisher>,
        sealer: Sealer,
    ) -> AppState {
        AppState {
            inner: Arc::new(Inner {
                messages: settings.messages(),
                pin_attempts: AttemptLimiter::for_pin_unlock(&settings.rate_limits),
                settings,
                database,
                events,
                sealer,
                metrics: Metrics::default(),
                mutation_gate: MutationGate::default(),
            }),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.inner.settings
    }

    /// The texts the server says to people, in the language of the instance.
    pub fn messages(&self) -> &Messages {
        &self.inner.messages
    }

    pub fn database(&self) -> &Database {
        &self.inner.database
    }

    /// Where handlers announce what they changed; see [`crate::events`].
    pub fn events(&self) -> &dyn EventPublisher {
        &*self.inner.events
    }

    pub fn metrics(&self) -> &Metrics {
        &self.inner.metrics
    }

    /// One change of a server-wide invariant at a time goes through it.
    pub fn mutation_gate(&self) -> &MutationGate {
        &self.inner.mutation_gate
    }

    /// The tries of the encryption PIN each account has.
    pub fn pin_attempts(&self) -> &AttemptLimiter {
        &self.inner.pin_attempts
    }

    /// Seals values for clients and the database; see [`crate::seal`].
    pub fn sealer(&self) -> &Sealer {
        &self.inner.sealer
    }
}
