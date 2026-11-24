//! What every handler can reach: the settings, the database, the catalogue of
//! messages and the other shared services of the process.
//!
//! To add a shared service: put it in a field of `Inner`, build it in `new`
//! and give it an accessor. Everything else reaches it through
//! `State(state): State<AppState>`.

use std::sync::{Arc, OnceLock};

use stellarr_messages::Messages;
use stellarr_storage::ObjectStorage;

use crate::admission::AdmissionGate;
use crate::auth::setup_code::SetupCode;
use crate::database::{Database, MutationGate};
use crate::events::{EventPublisher, Presence};
use crate::http::{AttemptLimiter, Metrics};
use crate::link_previews::PreviewQueue;
use crate::mail::MailDelivery;
use crate::realtime::Realtime;
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
    mail: MailDelivery,
    presence: Arc<dyn Presence>,
    setup_code: SetupCode,
    registration_gate: tokio::sync::Mutex<()>,
    realtime: OnceLock<Arc<Realtime>>,
    storage: Arc<dyn ObjectStorage>,
    admission: AdmissionGate,
    previews: PreviewQueue,
}

impl AppState {
    pub fn new(
        settings: Settings,
        database: Database,
        events: Arc<dyn EventPublisher>,
        presence: Arc<dyn Presence>,
        sealer: Sealer,
    ) -> AppState {
        AppState {
            inner: Arc::new(Inner {
                storage: crate::object_store::open(&settings),
                admission: AdmissionGate::new(&settings.uploads, settings.data_root.clone()),
                messages: settings.messages(),
                pin_attempts: AttemptLimiter::for_pin_unlock(&settings.rate_limits),
                settings,
                database,
                events,
                sealer,
                mail: MailDelivery::new(),
                presence,
                setup_code: SetupCode::default(),
                registration_gate: tokio::sync::Mutex::new(()),
                realtime: OnceLock::new(),
                previews: PreviewQueue::default(),
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

    /// Sends a built message by the transport the operator chose; see
    /// [`crate::mail`].
    pub fn mail(&self) -> &MailDelivery {
        &self.inner.mail
    }

    /// Whether a person is online, as the hub sees them. Without a hub,
    /// nobody is.
    pub fn presence_of(&self, user: crate::values::Id) -> String {
        self.inner.presence.presence_of(user)
    }

    /// What the hub knows of who is connected, for the rest of what a
    /// handler asks of it (what other people see, who has a connection open).
    pub fn presence(&self) -> &dyn Presence {
        &*self.inner.presence
    }

    /// The realtime hub, when this process runs one. The workers that look
    /// after its connections reach it here; handlers talk to it through
    /// [`events`](Self::events) and [`presence`](Self::presence).
    pub fn realtime(&self) -> Option<&Arc<Realtime>> {
        self.inner.realtime.get()
    }

    /// Plugs the hub in, once, when the server starts.
    pub(crate) fn plug_in_realtime(&self, realtime: Arc<Realtime>) {
        let _ = self.inner.realtime.set(realtime);
    }

    /// Where uploaded files are kept: the folder of the data volume or the
    /// bucket the settings name, made ready before the server listens.
    pub fn storage(&self) -> &dyn ObjectStorage {
        &*self.inner.storage
    }

    /// A shared handle on the store, for work that outlives the borrow of the
    /// state.
    pub fn storage_handle(&self) -> Arc<dyn ObjectStorage> {
        Arc::clone(&self.inner.storage)
    }

    /// Reserves room for an upload; see [`crate::admission`].
    pub fn admission(&self) -> &AdmissionGate {
        &self.inner.admission
    }

    /// The messages whose links wait for their cards.
    pub fn previews(&self) -> &PreviewQueue {
        &self.inner.previews
    }

    /// The code the owner account is created with, while there is none.
    pub fn setup_code(&self) -> &SetupCode {
        &self.inner.setup_code
    }

    /// Lets one registration at a time decide whether the instance is still
    /// empty and commit its account. Until the first account exists the
    /// database has no row that could enforce "only one first registration",
    /// so the process does.
    pub fn registration_gate(&self) -> &tokio::sync::Mutex<()> {
        &self.inner.registration_gate
    }
}
