//! Where the log goes, and what is kept out of it.
//!
//! Everything is logged through `tracing`. The levels come from the
//! `Logging:LogLevel:*` settings: a category is a tracing target, so
//! `Logging__LogLevel__Stellarr.Api.Requests=Warning` silences the line of
//! every request. Warnings and errors from everything and the information of
//! the API's own code are logged by default.
//!
//! The container's log is the first sink. A second one, which keeps the recent
//! lines for the operator panel, is a `tracing` layer handed to [`init`]; it
//! gets the same filter as the first.

use std::future::Future;
use std::io::IsTerminal;

use tracing::Subscriber;
use tracing_subscriber::filter::{EnvFilter, FilterExt, dynamic_filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, Registry};

use crate::settings::LogSettings;

/// The target of the line every request writes.
pub const REQUEST_TARGET: &str = "Stellarr.Api.Requests";

tokio::task_local! {
    static PROBE: bool;
}

/// Whether `path` is one of the health endpoints.
pub fn is_probe(path: &str) -> bool {
    path.eq_ignore_ascii_case("/health/ready") || path.eq_ignore_ascii_case("/health/live")
}

/// Keeps the health probes out of the log. The container's process manager
/// asks `/health/ready` every ten seconds, and each request would otherwise
/// write its line. While a probe is served nothing below a warning is logged,
/// whatever its target; a warning or an error still is, and every other
/// request logs as before.
pub fn serving<F: Future>(path: &str, work: F) -> impl Future<Output = F::Output> + use<F> {
    let probe = is_probe(path);
    async move { PROBE.scope(probe, work).await }
}

fn during_probe() -> bool {
    PROBE.try_with(|probe| *probe).unwrap_or(false)
}

/// A second place for log lines, as a layer over the registry.
pub type Sink = Box<dyn Layer<Registry> + Send + Sync>;

fn filter<S>(
    settings: &LogSettings,
) -> Result<impl tracing_subscriber::layer::Filter<S> + 'static, String>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let levels = EnvFilter::try_new(settings.directives())
        .map_err(|error| format!("the log levels are not usable: {error}"))?;
    let quiet = dynamic_filter_fn(
        |metadata: &tracing::Metadata<'_>, _: &tracing_subscriber::layer::Context<'_, S>| {
            !during_probe() || *metadata.level() <= tracing::Level::WARN
        },
    );
    Ok(levels.and(quiet))
}

/// Starts logging to the standard output and to each extra sink. Call it once.
pub fn init(settings: &LogSettings, sinks: Vec<Sink>) -> Result<(), String> {
    let mut filtered = Vec::new();
    for sink in sinks {
        filtered.push(sink.with_filter(filter::<Registry>(settings)?).boxed());
    }
    // In the container the log goes to a pipe, where colour codes are noise.
    let console = tracing_subscriber::fmt::layer()
        .with_ansi(std::io::stdout().is_terminal())
        .with_filter(filter(settings)?);
    // An empty list of layers would tell `tracing` that nothing is wanted.
    let sinks = (!filtered.is_empty()).then_some(filtered);
    tracing_subscriber::registry()
        .with(sinks)
        .with(console)
        .try_init()
        .map_err(|error| format!("logging was started twice: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_health_endpoints_are_the_probes() {
        assert!(is_probe("/health/ready"));
        assert!(is_probe("/Health/Live"));
        assert!(!is_probe("/health"));
        assert!(!is_probe("/health/ready/more"));
        assert!(!is_probe("/api/session"));
    }

    /// Collects the messages of what is logged.
    #[derive(Clone, Default)]
    struct Collected(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    impl<S: Subscriber> Layer<S> for Collected {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Message(String);
            impl tracing::field::Visit for Message {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if field.name() == "message" {
                        self.0 = format!("{value:?}");
                    }
                }
            }
            let mut message = Message(String::new());
            event.record(&mut message);
            self.0.lock().unwrap().push(message.0);
        }
    }

    #[tokio::test]
    async fn during_a_probe_only_warnings_and_errors_are_logged() {
        let settings = LogSettings {
            default_level: "Information".to_string(),
            categories: Vec::new(),
        };
        let collected = Collected::default();
        let subscriber = tracing_subscriber::registry()
            .with(collected.clone().with_filter(filter(&settings).unwrap()));
        let _installed = tracing::subscriber::set_default(subscriber);

        serving("/health/ready", async {
            tracing::info!("probe information");
            tracing::warn!("probe warning");
        })
        .await;
        serving("/health", async { tracing::info!("another path") }).await;
        tracing::info!("no request");

        assert_eq!(
            *collected.0.lock().unwrap(),
            ["probe warning", "another path", "no request"]
        );
    }

    #[test]
    fn a_category_has_a_level_of_its_own() {
        let quiet = LogSettings {
            default_level: "Information".to_string(),
            categories: vec![(REQUEST_TARGET.to_string(), "Warning".to_string())],
        };
        let collected = Collected::default();
        let subscriber = tracing_subscriber::registry()
            .with(collected.clone().with_filter(filter(&quiet).unwrap()));

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "Stellarr.Api.Requests", "GET /x 200 in 1 ms");
            tracing::warn!(target: "Stellarr.Api.Requests", "slow");
            tracing::info!("elsewhere");
        });

        assert_eq!(*collected.0.lock().unwrap(), ["slow", "elsewhere"]);
    }

    #[tokio::test]
    async fn only_the_work_of_a_probe_is_quiet() {
        assert!(!during_probe());
        assert!(serving("/health/ready", async { during_probe() }).await);
        assert!(!serving("/api/session", async { during_probe() }).await);
        assert!(!during_probe());
    }
}
