//! Background workers: started with the server, told to stop, and what
//! becomes of one that panics or does not stop.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use stellarr_api::Services;
use stellarr_api::background::Worker;
use stellarr_api::database::DbError;
use stellarr_api::http::Endpoints;

use crate::support::{LogCapture, Server};

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..400 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{what} did not happen in ten seconds");
}

async fn started_with(workers: Vec<Worker>, grace: Option<Duration>) -> Server {
    let services = Services {
        workers,
        shutdown_grace: grace,
        ..Services::default()
    };
    Server::in_process_with(Endpoints::new(), services, &[]).await
}

#[tokio::test]
async fn a_worker_starts_with_the_server_reads_its_state_and_stops_when_told() {
    let ticks = Arc::new(AtomicUsize::new(0));
    let ended = Arc::new(AtomicBool::new(false));
    let (seen, over) = (Arc::clone(&ticks), Arc::clone(&ended));
    let worker = Worker::new("probe", move |state, stop| {
        let (seen, over) = (Arc::clone(&seen), Arc::clone(&over));
        async move {
            while !stop.is_stopping() {
                // The same read the handlers use.
                let accounts: i64 = state
                    .database()
                    .read(|connection| -> Result<i64, DbError> {
                        Ok(connection.query_row("SELECT COUNT(*) FROM \"Users\"", [], |r| r.get(0))?)
                    })
                    .await
                    .expect("the database can be read");
                assert_eq!(accounts, 0);
                seen.fetch_add(1, Ordering::SeqCst);
                stop.sleep(Duration::from_millis(20)).await;
            }
            over.store(true, Ordering::SeqCst);
        }
    });
    let server = started_with(vec![worker], None).await;

    until("the first iteration", || ticks.load(Ordering::SeqCst) > 0).await;
    assert!(!ended.load(Ordering::SeqCst));

    assert!(server.stop().await, "the server leaves cleanly");

    assert!(ended.load(Ordering::SeqCst), "the worker was told to stop and did");
}

#[tokio::test]
async fn a_panicking_worker_is_logged_and_started_again_and_the_others_go_on() {
    let log = LogCapture::start();
    let runs = Arc::new(AtomicUsize::new(0));
    let steady = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let (counted, ticked, once) = (Arc::clone(&runs), Arc::clone(&steady), Arc::clone(&finished));
    let fragile = Worker::new("fragile", move |_, stop| {
        let counted = Arc::clone(&counted);
        async move {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("the first run is bad luck");
            }
            stop.stopped().await;
        }
    });
    let steady_worker = Worker::new("steady", move |_, stop| {
        let ticked = Arc::clone(&ticked);
        async move {
            while stop.sleep(Duration::from_millis(20)).await {
                ticked.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    let short = Worker::new("short", move |_, _| {
        let once = Arc::clone(&once);
        async move {
            once.fetch_add(1, Ordering::SeqCst);
        }
    });
    let server = started_with(vec![fragile, steady_worker, short], None).await;

    until("the panic", || {
        log.lines().iter().any(|line| line.contains("fragile panicked"))
    })
    .await;
    let message = log
        .lines()
        .into_iter()
        .find(|line| line.contains("fragile panicked"))
        .unwrap();
    assert!(message.contains("the first run is bad luck"), "{message}");
    let before = steady.load(Ordering::SeqCst);
    until("the second run", || runs.load(Ordering::SeqCst) == 2).await;

    assert!(steady.load(Ordering::SeqCst) > before, "the other worker went on");
    assert_eq!(server.client().get("/health/live").await.status, 200);
    assert_eq!(finished.load(Ordering::SeqCst), 1, "a worker that returned is not started again");
    assert!(server.stop().await);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

struct Dropped(Arc<AtomicBool>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn a_worker_that_does_not_stop_is_abandoned_after_the_grace_and_named_in_the_log() {
    let log = LogCapture::start();
    let gone = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&gone);
    let stubborn = Worker::new("stubborn", move |_, _| {
        let held = Dropped(Arc::clone(&flag));
        async move {
            let _held = held;
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    });
    let server = started_with(vec![stubborn], Some(Duration::from_millis(300))).await;
    let asked = Instant::now();

    assert!(server.stop().await, "the server still leaves");

    assert!(asked.elapsed() < Duration::from_secs(5));
    let lines = log.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("stubborn") && line.contains("abandoned")),
        "{lines:?}"
    );
    assert!(gone.load(Ordering::SeqCst), "its task was aborted");
}
