//! The Stellarr API server.
//!
//! - [`settings`]: everything read at start.
//! - [`database`] and [`values`]: SQLite, and how a value is stored in it.
//! - [`http`]: the pipeline every request goes through, and the way JSON and
//!   problems are written.
//! - [`events`]: what handlers tell the realtime hub.
//! - [`realtime`]: the hub itself, its connections and its methods.
//! - [`state`]: what every handler can reach.
//! - [`background`]: work that runs beside the requests, listed in [`workers`].
//! - one folder per group of endpoints, listed in [`groups`].
//!
//! `api/README.md` says how to add a group of endpoints and how to test it.

pub mod auth;
pub mod background;
pub mod database;
pub mod encoding;
pub mod events;
pub mod http;
pub mod instance;
pub mod logging;
pub mod mail;
pub mod realtime;
pub mod seal;
pub mod secrets;
pub mod settings;
pub mod state;
pub mod values;

#[cfg(test)]
mod testing;

use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tracing::{error, info};

use crate::database::Database;
use crate::events::{EventFile, EventPublisher, Fanout, Presence, Unconnected};
use crate::http::Endpoints;
use crate::realtime::Realtime;
use crate::seal::Sealer;
use crate::settings::Settings;
use crate::state::AppState;

/// The groups of endpoints, joined: each is a folder under `src/` whose
/// `mod.rs` has one function, `pub fn routes() -> Endpoints`. To add a group,
/// write its name on a line of `groups.list.rs`; that is the only shared line
/// it touches.
macro_rules! groups {
    ($($name:ident),* $(,)?) => {
        $(pub mod $name;)*

        /// The routes of every group of the API.
        pub fn groups() -> Vec<Endpoints> {
            vec![$($name::routes()),*]
        }
    };
}

// The list itself is in its own file, one name to a line, so that groups
// added on different branches merge without a conflict.
include!("groups.list.rs");

/// The background workers, joined: each name is a module that has one
/// function, `pub fn workers() -> Vec<Worker>`. To add one, write the path of
/// its module on a line of `workers.list.rs`; that is the only shared line it
/// touches. The module itself is declared where it lives.
macro_rules! workers {
    // A path is written as its segments: a `path` fragment cannot be followed
    // by `::workers`.
    ($($($module:ident)::+),* $(,)?) => {
        /// The workers of every module of the API.
        pub fn workers() -> Vec<$crate::background::Worker> {
            #[allow(unused_mut)]
            let mut all = Vec::new();
            $(all.extend($($module)::+::workers());)*
            all
        }
    };
}

include!("workers.list.rs");

/// How long a stopping server waits for the requests in flight before it
/// gives up on them. The container's manager waits a little longer.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(30);

/// Why the server did not start, or stopped.
#[derive(Debug)]
pub struct StartError(pub String);

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StartError {}

/// Runs the API until `shutdown` resolves: prepares the data folder and the
/// database, listens on the configured addresses, serves `groups`, then
/// finishes the requests in flight.
///
/// `listening` is told the addresses once they are bound, which matters when
/// the port asked for was 0.
pub async fn serve(
    settings: Settings,
    groups: Vec<Endpoints>,
    listening: impl FnOnce(&[SocketAddr]),
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), StartError> {
    serve_with(settings, groups, Services::default(), listening, shutdown).await
}

/// What the server is plugged into: where handlers send events, and who says
/// whether a person is online. The hub provides both. Without them events go
/// nowhere, and nobody is online.
#[derive(Default, Clone)]
pub struct Services {
    pub events: Option<Arc<dyn EventPublisher>>,
    pub presence: Option<Arc<dyn Presence>>,
    /// The realtime hub. When there is one it serves `/hubs/app` and is the
    /// publisher and the source of presence, whatever `events` and `presence`
    /// say; in the Testing environment what is published is also written to
    /// the event file. [`run`] gives it one.
    pub realtime: Option<Arc<Realtime>>,
    /// The background workers to run. None by default, so that a server
    /// started for a test has no timers of its own; [`run`] gives it
    /// [`workers`].
    pub workers: Vec<background::Worker>,
    /// How long a stopping server waits for requests and workers; the
    /// built-in grace when `None`.
    pub shutdown_grace: Option<Duration>,
}

/// [`serve`] with the services of the process given.
pub async fn serve_with(
    settings: Settings,
    groups: Vec<Endpoints>,
    services: Services,
    listening: impl FnOnce(&[SocketAddr]),
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), StartError> {
    let fail = |what: &str, error: &dyn fmt::Display| StartError(format!("{what}: {error}"));

    // The keys that protect sessions live in their own folder of the data
    // volume, so a restart or a new container does not sign everybody out.
    let keys = settings.data_root.join("keys");
    std::fs::create_dir_all(&keys)
        .map_err(|e| fail(&format!("cannot create {}", keys.display()), &e))?;
    if let Some(folder) = settings
        .database
        .path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        std::fs::create_dir_all(folder)
            .map_err(|e| fail(&format!("cannot create {}", folder.display()), &e))?;
    }

    let database_settings = settings.database.clone();
    let database = tokio::task::spawn_blocking(move || Database::open(&database_settings))
        .await
        .map_err(|e| fail("the database did not open", &e))?
        .map_err(|e| fail("the database did not open", &e))?;

    // The hub, when there is one, hears the events; in the Testing
    // environment they are also written to a file the tests read. A
    // publisher that was given instead stands in for the hub; with neither
    // there is nobody to tell.
    let realtime = services.realtime;
    let file = settings
        .event_log
        .as_ref()
        .map(|path| Arc::new(EventFile::new(path.clone())) as Arc<dyn EventPublisher>);
    let events: Arc<dyn EventPublisher> = match (&realtime, services.events, file) {
        (Some(hub), _, Some(file)) => Arc::new(Fanout(vec![hub.clone(), file])),
        (Some(hub), _, None) => hub.clone(),
        (None, Some(events), _) => events,
        (None, None, Some(file)) => file,
        (None, None, None) => Arc::new(Unconnected),
    };
    let grace = services.shutdown_grace.unwrap_or(SHUTDOWN_GRACE);
    let presence: Arc<dyn Presence> = match &realtime {
        Some(hub) => hub.clone(),
        None => services.presence.unwrap_or_else(|| Arc::new(Unconnected)),
    };
    let addresses = settings.listen.0.clone();
    let sealer = Sealer::load_or_create(&keys).map_err(|e| StartError(e.to_string()))?;
    let state = AppState::new(settings, database, events, presence, sealer);

    issue_setup_code(&state).await?;

    // The hub needs the state, which needs the hub as its publisher, so it is
    // built first and mounted now.
    let mut groups = groups;
    if let Some(hub) = &realtime {
        hub.restore_seed(state.settings());
        state.plug_in_realtime(hub.clone());
        groups.push(realtime::endpoints(state.clone(), hub.clone()));
    }

    let (router, rules) = groups
        .into_iter()
        .fold(Endpoints::new(), Endpoints::merge)
        .into_parts();
    let app = http::build(state.clone(), router, rules);

    let mut listeners = Vec::new();
    for address in addresses {
        let listener = TcpListener::bind(address)
            .await
            .map_err(|e| fail(&format!("cannot listen on {address}"), &e))?;
        listeners.push(listener);
    }
    let bound: Vec<SocketAddr> = listeners
        .iter()
        .map(TcpListener::local_addr)
        .collect::<Result<_, _>>()
        .map_err(|e| fail("cannot read the listening address", &e))?;
    let running = background::start(services.workers, &state);
    listening(&bound);
    for address in &bound {
        info!(target: "Stellarr.Api.Host", "Now listening on: http://{address}");
    }

    let (stop, stopped) = tokio::sync::watch::channel(false);
    let mut servers = tokio::task::JoinSet::new();
    for listener in listeners {
        let service = app
            .clone()
            .into_make_service_with_connect_info::<SocketAddr>();
        let mut stopped = stopped.clone();
        servers.spawn(async move {
            axum::serve(listener, service)
                .with_graceful_shutdown(async move {
                    let _ = stopped.wait_for(|stopping| *stopping).await;
                })
                .await
        });
    }

    let mut result = Ok(());
    tokio::select! {
        () = shutdown => {
            info!(target: "Stellarr.Api.Host", "Application is shutting down...");
        }
        Some(finished) = servers.join_next() => {
            // A server that ends by itself has failed.
            result = match finished {
                Ok(Ok(())) => Err(StartError("a server stopped on its own".to_string())),
                Ok(Err(e)) => Err(fail("the server failed", &e)),
                Err(e) => Err(fail("the server failed", &e)),
            };
        }
    }
    let _ = stop.send(true);
    // The clients are told the server is going and may start over, instead of
    // finding out from a connection that goes quiet.
    if let Some(hub) = &realtime {
        hub.hub().close_all();
    }
    let drained = tokio::time::timeout(grace, async {
        while servers.join_next().await.is_some() {}
    });
    let (drained, ()) = tokio::join!(drained, running.stop(grace));
    if drained.is_err() {
        error!("requests were still running after {grace:?}; they are dropped");
    }
    result
}

/// An instance without accounts is founded by whoever creates the first one.
/// The code that registration asks for is shown here, in the log, which only
/// the operator reads; a new one at every start until the owner exists.
async fn issue_setup_code(state: &AppState) -> Result<(), StartError> {
    let empty: bool = state
        .database()
        .read(|connection| -> Result<bool, crate::database::DbError> {
            Ok(
                connection.query_row("SELECT NOT EXISTS (SELECT 1 FROM \"Users\")", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .await
        .map_err(|e| StartError(format!("the database cannot be read: {e}")))?;
    if !empty {
        return Ok(());
    }
    let code = state.setup_code().issue();
    let public_url = &state.settings().operations.public_url;
    if public_url.is_empty() {
        tracing::warn!(target: "Stellarr.Api.Logging.StartupLog", "Setup code: {code}. Open /setup at the address of this instance to create the owner account.");
    } else {
        tracing::warn!(target: "Stellarr.Api.Logging.StartupLog", "Setup code: {code}. Open {public_url}/setup to create the owner account.");
    }
    Ok(())
}

/// Runs the real API, with every group of endpoints, until SIGTERM or SIGINT.
pub async fn run(settings: Settings) -> Result<(), StartError> {
    let services = Services {
        realtime: Some(Realtime::new()),
        workers: background::chosen(&settings, workers()),
        ..Services::default()
    };
    serve_with(settings, groups(), services, |_| {}, shutdown_signal()).await
}

/// Resolves on SIGTERM or SIGINT.
async fn shutdown_signal() {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = interrupt => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = interrupt.await;
}
