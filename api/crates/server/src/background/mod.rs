//! Work that runs beside the requests, from the moment the server has its
//! database and state until it stops: the watcher that closes the connections
//! of ended sessions, the queue of mail, housekeeping every hour.
//!
//! A module declares a [`Worker`]; the list of modules that do is
//! `workers.list.rs`. `api/README.md` shows how to add one.
//!
//! # A worker
//!
//! ```
//! use std::time::Duration;
//!
//! use stellarr_api::background::Worker;
//! use stellarr_api::database::DbError;
//! use tracing::{info, warn};
//!
//! pub fn workers() -> Vec<Worker> {
//!     vec![Worker::new("census", |state, stop| async move {
//!         // `while stop.sleep(..)` is a timer that ends with the server.
//!         while stop.sleep(Duration::from_secs(3600)).await {
//!             let counted = state
//!                 .database()
//!                 .read(|connection| -> Result<i64, DbError> {
//!                     Ok(connection.query_row("SELECT COUNT(*) FROM \"Users\"", [], |row| row.get(0))?)
//!                 })
//!                 .await;
//!             // An iteration that fails is logged, and the next one tries again.
//!             match counted {
//!                 Ok(accounts) => info!("{accounts} accounts"),
//!                 Err(error) => warn!("the census failed: {error}"),
//!             }
//!         }
//!     })]
//! }
//! ```
//!
//! What the server promises a worker:
//!
//! - it starts after the database is open and the state is built, and after
//!   the server is listening; it never delays either;
//! - when the server stops it is told through [`Stop`], and it is awaited for
//!   as long as the shutdown grace allows, the same grace the requests in
//!   flight have; what has not ended by then is abandoned, which means its task
//!   is aborted at its next `.await`, and the name is logged;
//! - if it panics, the panic is logged with its name and the worker is started
//!   again after a pause that doubles from one second to a minute (and starts
//!   over once a run has lasted a minute). Other workers and the requests do
//!   not notice;
//! - if it returns before it was told to stop, it is finished and is not
//!   started again. A worker that is switched off by a setting returns at once.
//!
//! A restart is a new call of the worker's function, so what lived in the old
//! run is gone; what must last belongs in the database. An iteration that
//! fails should be handled by the worker: catch it,
//! log it, and go on to the next; the restart is the net under that, not the
//! way to handle an error.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::Instant;
use tracing::{debug, error, info};

use crate::settings::Settings;
use crate::state::AppState;

const TARGET: &str = "Stellarr.Api.Workers";

/// The pause before the first restart after a panic, and the longest pause.
const FIRST_RESTART: Duration = Duration::from_secs(1);
const LONGEST_RESTART: Duration = Duration::from_secs(60);

type Run = Arc<dyn Fn(AppState, Stop) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// A piece of work that runs for as long as the server does.
#[derive(Clone)]
pub struct Worker {
    name: &'static str,
    run: Run,
}

impl Worker {
    /// `name` is how the log and the Testing environment's `Testing__Workers`
    /// know it; keep it short and unique. `run` is called once at start, and
    /// again after a panic, and is expected to return soon after `stop` says
    /// the server is stopping.
    pub fn new<F, Fut>(name: &'static str, run: F) -> Worker
    where
        F: Fn(AppState, Stop) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Worker {
            name,
            run: Arc::new(move |state, stop| Box::pin(run(state, stop))),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// How a worker hears that the server is stopping.
#[derive(Clone)]
pub struct Stop(watch::Receiver<bool>);

impl Stop {
    /// Whether the server has been told to stop. Check it between the steps of
    /// a long iteration.
    pub fn is_stopping(&self) -> bool {
        *self.0.borrow()
    }

    /// Resolves when the server is told to stop (at once if it already was).
    /// Put it in a `select!` with whatever else the worker waits for.
    pub async fn stopped(&self) {
        let mut watch = self.0.clone();
        let _ = watch.wait_for(|stopping| *stopping).await;
    }

    /// Waits `time`, or less if the server stops first. Says whether the whole
    /// time passed, so that `while stop.sleep(interval).await { work }` is a
    /// timer that ends with the server.
    pub async fn sleep(&self, time: Duration) -> bool {
        tokio::select! {
            () = tokio::time::sleep(time) => true,
            () = self.stopped() => false,
        }
    }
}

/// Keeps the workers the Testing environment asked for (`Testing__Workers`, a
/// comma-separated list of names; none when it is not set), and all of them in
/// any other environment.
pub fn chosen(settings: &Settings, all: Vec<Worker>) -> Vec<Worker> {
    match &settings.testing_workers {
        Some(names) => all
            .into_iter()
            .filter(|worker| names.iter().any(|name| name == worker.name))
            .collect(),
        None => all,
    }
}

/// The workers that have been started.
pub struct Running {
    stop: watch::Sender<bool>,
    tasks: Vec<(&'static str, JoinHandle<()>)>,
}

/// Starts every worker, each in a task of its own.
pub fn start(workers: Vec<Worker>, state: &AppState) -> Running {
    let (stop, stopped) = watch::channel(false);
    let tasks = workers
        .into_iter()
        .map(|worker| {
            let name = worker.name;
            let task = tokio::spawn(supervise(worker, state.clone(), Stop(stopped.clone())));
            (name, task)
        })
        .collect();
    Running { stop, tasks }
}

impl Running {
    /// Tells every worker to stop and waits up to `grace` for all of them
    /// together; those still running are aborted and named in the log.
    pub async fn stop(self, grace: Duration) {
        let _ = self.stop.send(true);
        let deadline = Instant::now() + grace;
        for (name, mut task) in self.tasks {
            match tokio::time::timeout_at(deadline, &mut task).await {
                Ok(_) => debug!(target: TARGET, "Worker {name} stopped"),
                Err(_) => {
                    error!(target: TARGET, "Worker {name} did not stop within {grace:?} and is abandoned");
                    task.abort();
                }
            }
        }
    }
}

/// Aborts the task it holds when dropped, so that aborting the supervisor
/// ends the worker's own task too.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Runs a worker, and runs it again when it panics.
async fn supervise(worker: Worker, state: AppState, stop: Stop) {
    let name = worker.name;
    let mut pause = FIRST_RESTART;
    loop {
        let started = Instant::now();
        let run = tokio::spawn((worker.run)(state.clone(), stop.clone()));
        let _abort = AbortOnDrop(run.abort_handle());
        match run.await {
            Ok(()) => {
                debug!(target: TARGET, "Worker {name} finished");
                return;
            }
            Err(ended) if ended.is_panic() => {
                let cause = panic_text(ended.into_panic());
                if stop.is_stopping() {
                    error!(target: TARGET, "Worker {name} panicked while stopping: {cause}");
                    return;
                }
                if started.elapsed() >= LONGEST_RESTART {
                    pause = FIRST_RESTART;
                }
                error!(target: TARGET, "Worker {name} panicked: {cause}; it starts again in {pause:?}");
            }
            Err(_) => return,
        }
        if !stop.sleep(pause).await {
            return;
        }
        pause = (pause * 2).min(LONGEST_RESTART);
        info!(target: TARGET, "Worker {name} starts again");
    }
}

fn panic_text(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(text) => *text,
        Err(payload) => payload
            .downcast_ref::<&'static str>()
            .map_or_else(|| "no message".to_string(), |text| (*text).to_string()),
    }
}
