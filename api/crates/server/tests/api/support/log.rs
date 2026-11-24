use std::cell::RefCell;
use std::sync::{Arc, Mutex, Once};

use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{Layer, registry};

type Lines = Arc<Mutex<Vec<String>>>;

thread_local! {
    static SINK: RefCell<Option<Lines>> = const { RefCell::new(None) };
}

/// Collects what the server logs while the test runs.
///
/// It is for a server started with `Server::in_process`, which runs on the
/// test's own thread (`#[tokio::test]` runs on one), so that what it logs
/// reaches the test that started it and no other. It is not for the binary,
/// whose log is `Server::log`.
///
/// One subscriber is installed for the whole test process, the first time a
/// capture starts; a subscriber installed for each test would leave events
/// switched off for the callsites that another test had reached first.
pub struct LogCapture {
    lines: Lines,
}

struct Route;

impl<S: Subscriber> Layer<S> for Route {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        struct Message(String);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        SINK.with(|sink| {
            if let Some(lines) = sink.borrow().as_ref() {
                lines.lock().unwrap().push(message.0);
            }
        });
    }
}

impl LogCapture {
    pub fn start() -> LogCapture {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            tracing::subscriber::set_global_default(registry().with(Route))
                .expect("no other subscriber");
        });
        let lines = Lines::default();
        SINK.with(|sink| *sink.borrow_mut() = Some(Arc::clone(&lines)));
        LogCapture { lines }
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
}

impl Drop for LogCapture {
    fn drop(&mut self) {
        SINK.with(|sink| *sink.borrow_mut() = None);
    }
}
