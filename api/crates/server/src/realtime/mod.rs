//! The realtime hub: what the web client keeps open to be told of changes,
//! and the calls it makes over that connection.
//!
//! One connection is one WebSocket at `/hubs/app`, let in by the session
//! cookie like any request. It belongs to groups: the account's own, the
//! server it follows, and the account's own on that server. Nothing else is a
//! group; whoever may see an event is worked out when the event is published
//! (see [`crate::events`]), so a person who loses sight of something stops
//! hearing about it with the next event.
//!
//! - [`Realtime`] is what exists before the rest of the server does: the
//!   connections, who is online, and the registry of sessions. It is the
//!   `EventPublisher` and `Presence` the handlers talk to.
//! - [`endpoints`] mounts the hub once the state exists; the pipeline's steps
//!   (client address, rate limit, authentication) run before it.
//! - [`watcher`] looks at the database every second and closes the
//!   connections of sessions that no longer count.
//! - [`method`] is how a group adds methods and hooks, and `methods.list.rs`
//!   is the list of the groups that have.
//!
//! Methods that need a channel or a voice room are not here yet: a call to
//! one is answered as a call to a method that does not exist.

mod application;
pub mod audience;
mod caller;
pub mod method;
pub mod names;
mod origin;
pub mod presence;
mod publisher;
pub mod registry;
pub mod seed;
pub mod throttle;
pub(crate) mod watcher;

use std::sync::Arc;

use axum::response::IntoResponse;
use stellarr_hub::{Config, Hub};

use self::application::AppHub;
pub use self::method::{Context, Method, Registration, Who};
use self::presence::PresenceTracker;
use self::registry::Registry;
use self::throttle::ThrottleGate;
use crate::auth::cookies;
use crate::auth::session::{self, Ticket};
use crate::http::{ApiError, Endpoints, QueryValues};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The hub's modules that add methods or hooks, joined: each is a module that
/// has one function, `pub fn registration() -> Registration`. To add one, write
/// its path on a line of `methods.list.rs`; that is the only shared line it
/// touches.
macro_rules! methods {
    ($($($module:ident)::+),* $(,)?) => {
        /// What every module adds to the hub.
        fn registrations() -> Vec<Registration> {
            vec![$($($module)::+::registration()),*]
        }
    };
}

include!("methods.list.rs");

/// The path the hub is served at.
const PATH: &str = "/hubs/app";

/// The connections, who is online, and what ties a connection to a session.
/// Built before the state, because it is what handlers publish through.
pub struct Realtime {
    hub: Hub,
    registry: Registry,
    presence: PresenceTracker,
    throttle: ThrottleGate,
}

impl Realtime {
    pub fn new() -> Arc<Realtime> {
        Arc::new(Realtime {
            hub: Hub::new(&Config::default()),
            registry: Registry::default(),
            presence: PresenceTracker::default(),
            throttle: ThrottleGate::default(),
        })
    }

    /// The connections and groups, and what is sent through them.
    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn presence(&self) -> &PresenceTracker {
        &self.presence
    }

    pub fn throttle(&self) -> &ThrottleGate {
        &self.throttle
    }

    /// Ends the connections a session opened. The clients are told not to
    /// start over: they would only be refused. Returns how many there were.
    pub fn end_session(&self, session: Id) -> usize {
        let ended = self.registry.take_session(session);
        for connection in &ended {
            self.hub.end_session(connection);
        }
        ended.len()
    }

    /// Ends one connection, as for a session that no longer counts.
    pub fn end_connection(&self, connection_id: &str) {
        self.registry.unregister(connection_id);
        self.hub.end_session(connection_id);
    }

    /// Reads the presence a development seed left in the data folder.
    pub fn restore_seed(&self, settings: &crate::settings::Settings) {
        seed::apply(&settings.environment, &settings.data_root, &self.presence);
    }
}

/// The hub's WebSocket endpoint at `/hubs/app`.
pub fn endpoints(state: AppState, realtime: Arc<Realtime>) -> Endpoints {
    let application = Arc::new(AppHub::new(state.clone(), Arc::clone(&realtime)));
    let router = stellarr_hub::router(
        "/",
        application,
        realtime.hub().clone(),
        Config::default(),
        move |parts| {
            let state = state.clone();
            async move { admit(&state, &parts).map_err(IntoResponse::into_response) }
        },
    );
    Endpoints::new().service(PATH, router)
}

/// Lets in whoever the pipeline found signed in: the session cookie has been
/// read and checked against the database before the request got here. Anyone
/// else is refused as any route that needs a session refuses them.
fn admit(state: &AppState, parts: &axum::http::request::Parts) -> Result<Who, ApiError> {
    let refuse = ApiError::unauthorized;
    let caller = parts
        .extensions
        .get::<session::Caller>()
        .copied()
        .ok_or_else(refuse)?;
    if !origin::allows(state.settings(), parts) {
        return Err(ApiError::forbidden());
    }
    // The pipeline has already opened the cookie; opening it again costs no
    // database and is how the hub learns which version of the account's
    // sessions the connection was let in with.
    let name = session::cookie_name(&state.settings().environment);
    let ticket = cookies::read(&parts.headers, name)
        .and_then(|value| Ticket::open(state.sealer(), &value, Timestamp::now()))
        .ok_or_else(refuse)?;
    let requested_server =
        QueryValues::parse(parts.uri.query(), state.messages()).text("serverId")?;
    Ok(Who {
        user: caller.user,
        session: caller.session,
        version: ticket.version(),
        requested_server,
    })
}
