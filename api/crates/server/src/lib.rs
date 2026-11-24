//! The Stellarr API server.
//!
//! - [`settings`]: everything read at start.
//! - [`database`] and [`values`]: SQLite, and how a value is stored in it.
//! - [`http`]: the pipeline every request goes through, and the way JSON and
//!   problems are written.
//! - [`events`]: what handlers tell the realtime hub.
//! - [`state`]: what every handler can reach.
//! - one folder per group of endpoints, listed in [`groups`].
//!
//! `api/README.md` says how to add a group of endpoints and how to test it.

pub mod database;
pub mod encoding;
pub mod events;
pub mod http;
pub mod logging;
pub mod seal;
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
use crate::events::{EventFile, EventPublisher, Unconnected};
use crate::http::Endpoints;
use crate::seal::Sealer;
use crate::settings::Settings;
use crate::state::AppState;

/// The groups of endpoints, joined: each is a folder under `src/` whose
/// `mod.rs` has one function, `pub fn routes() -> Endpoints`. To add a group,
/// write its name on a line of `groups.list.rs`; that is the only shared line
/// it touches.
macro_rules! groups {
    ($($name:ident),* $(,)?) => {
        $(mod $name;)*

        /// The routes of every group of the API.
        pub fn groups() -> Vec<Endpoints> {
            vec![$($name::routes()),*]
        }
    };
}

// The list itself is in its own file, one name to a line, so that groups
// added on different branches merge without a conflict.
include!("groups.list.rs");

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

    let events: Arc<dyn EventPublisher> = match &settings.event_log {
        Some(path) => Arc::new(EventFile::new(path.clone())),
        None => Arc::new(Unconnected),
    };
    let addresses = settings.listen.0.clone();
    let sealer = Sealer::load_or_create(&keys).map_err(|e| StartError(e.to_string()))?;
    let state = AppState::new(settings, database, events, sealer);

    let (router, rules) = groups
        .into_iter()
        .fold(Endpoints::new(), Endpoints::merge)
        .into_parts();
    let app = http::build(state, router, rules);

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
    let drained = tokio::time::timeout(SHUTDOWN_GRACE, async {
        while servers.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        error!("requests were still running after {SHUTDOWN_GRACE:?}; they are dropped");
    }
    result
}

/// Runs the real API, with every group of endpoints, until SIGTERM or SIGINT.
pub async fn run(settings: Settings) -> Result<(), StartError> {
    serve(settings, groups(), |_| {}, shutdown_signal()).await
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
