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

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::future::Future;
use std::io::IsTerminal;
use std::sync::{Arc, Mutex};

use tracing::Subscriber;
use tracing_subscriber::filter::{EnvFilter, FilterExt, dynamic_filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, Registry};

use crate::http::trace;
use crate::settings::LogSettings;
use crate::values::Timestamp;

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

/// How many lines the panel's buffer keeps; each new one pushes out the oldest.
pub const LOG_CAPACITY: usize = 10_000;

/// The longest message the buffer keeps whole, in bytes. A request line holds
/// the path, which a client can make thousands of bytes long, and the buffer's
/// size is bounded by its line count only if each line is.
const MAX_MESSAGE_BYTES: usize = 2048;

/// The target of the line that tells the operator the setup code. It goes to
/// the process's output, where the operator reads it, and never into the
/// panel's buffer, which people with a session read.
pub const OPERATOR_ONLY_TARGET: &str = "Stellarr.Api.Logging.StartupLog";

/// The target of the warning that the object store is not ready yet. Also kept
/// out of the panel's buffer: the text is the store's own error, and a bucket
/// store's answer to a refused request can name the access key it was given,
/// which is half of a credential. The operator reads it in the process output.
pub const STORE_NOT_READY_TARGET: &str = "Stellarr.Api.Logging.ObjectStore";

/// What a line says for its trace when no request wrote it.
pub const BACKGROUND_TRACE: &str = "background";

/// The level of a kept line: the buffer keeps information and above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineLevel {
    Info,
    Warn,
    Error,
}

impl LineLevel {
    /// The word the panel shows and filters by.
    pub fn name(self) -> &'static str {
        match self {
            LineLevel::Info => "info",
            LineLevel::Warn => "warn",
            LineLevel::Error => "error",
        }
    }
}

/// One kept line. `sequence` numbers every line the process has logged, from
/// 0, so it stays a stable cursor while older lines fall out of the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub sequence: u64,
    pub at: Timestamp,
    pub level: LineLevel,
    pub category: String,
    pub message: String,
    pub trace_id: String,
}

/// A page of lines, newest first, and whether older matching lines remain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPage {
    pub lines: Vec<LogLine>,
    pub has_more: bool,
}

#[derive(Default)]
struct Ring {
    lines: VecDeque<LogLine>,
    written: u64,
}

/// The most recent lines the process logged, for the operator panel. It is a
/// [`Sink`]: whatever is logged at information level or above, under the same
/// levels as the container's log, lands here. Memory is bounded by
/// [`LOG_CAPACITY`] lines of at most a few kilobytes each.
#[derive(Default)]
pub struct LogBuffer {
    ring: Mutex<Ring>,
}

impl LogBuffer {
    pub fn new() -> Arc<LogBuffer> {
        Arc::new(LogBuffer::default())
    }

    /// The layer that fills this buffer, to hand to [`init`].
    pub fn sink(self: &Arc<LogBuffer>) -> Sink {
        Box::new(BufferLayer {
            buffer: self.clone(),
        })
    }

    fn append(&self, level: LineLevel, category: &str, message: String) {
        let at = Timestamp::now();
        let trace_id = trace::try_current().unwrap_or_else(|| BACKGROUND_TRACE.to_string());
        let mut ring = self
            .ring
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sequence = ring.written;
        if ring.lines.len() == LOG_CAPACITY {
            ring.lines.pop_front();
        }
        ring.lines.push_back(LogLine {
            sequence,
            at,
            level,
            category: category.to_string(),
            message,
            trace_id,
        });
        ring.written += 1;
    }

    /// Up to `limit` lines `matches` accepts, newest first, all older than the
    /// line numbered `before`, or the newest ones when it is none.
    pub fn page(
        &self,
        before: Option<u64>,
        limit: usize,
        matches: impl Fn(&LogLine) -> bool,
    ) -> LogPage {
        let ring = self
            .ring
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lines = Vec::with_capacity(limit.min(ring.lines.len()));
        let oldest = ring
            .lines
            .front()
            .map_or(ring.written, |line| line.sequence);
        let newest_allowed = before.unwrap_or(ring.written).min(ring.written);
        // The ring holds consecutive sequences from `oldest` to the last.
        let mut sequence = newest_allowed;
        while sequence > oldest {
            sequence -= 1;
            let line = &ring.lines[usize::try_from(sequence - oldest).unwrap_or(usize::MAX)];
            if !matches(line) {
                continue;
            }
            if lines.len() == limit {
                return LogPage {
                    lines,
                    has_more: true,
                };
            }
            lines.push(line.clone());
        }
        LogPage {
            lines,
            has_more: false,
        }
    }

    /// Every line still kept, newest first, for counting.
    pub fn all(&self) -> Vec<LogLine> {
        let ring = self
            .ring
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ring.lines.iter().rev().cloned().collect()
    }
}

struct BufferLayer {
    buffer: Arc<LogBuffer>,
}

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        if matches!(
            metadata.target(),
            OPERATOR_ONLY_TARGET | STORE_NOT_READY_TARGET
        ) {
            return;
        }
        let level = match *metadata.level() {
            tracing::Level::ERROR => LineLevel::Error,
            tracing::Level::WARN => LineLevel::Warn,
            tracing::Level::INFO => LineLevel::Info,
            _ => return,
        };
        let mut text = Text::default();
        event.record(&mut text);
        self.buffer.append(level, metadata.target(), text.finish());
    }
}

/// The message of an event, and its other fields after it as `name=value`.
#[derive(Default)]
struct Text {
    message: String,
    fields: String,
}

impl Text {
    fn finish(self) -> String {
        let mut text = self.message;
        if !self.fields.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&self.fields);
        }
        if text.len() > MAX_MESSAGE_BYTES {
            let mut cut = MAX_MESSAGE_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('\u{2026}');
        }
        text
    }
}

impl tracing::field::Visit for Text {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        // Bounded as it is written: a field's text is not trusted to be short.
        let target = if field.name() == "message" {
            &mut self.message
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}=", field.name());
            &mut self.fields
        };
        if target.len() <= MAX_MESSAGE_BYTES {
            let _ = write!(target, "{value:?}");
        }
    }
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

    /// Logs `work` into a fresh buffer.
    fn buffered(work: impl FnOnce()) -> Arc<LogBuffer> {
        let buffer = LogBuffer::new();
        let subscriber = tracing_subscriber::registry().with(buffer.sink());
        tracing::subscriber::with_default(subscriber, work);
        buffer
    }

    #[test]
    fn the_buffer_keeps_information_and_above_with_their_category_and_fields() {
        let buffer = buffered(|| {
            tracing::debug!(target: "Stellarr.Api.Test", "too quiet");
            tracing::info!(target: "Stellarr.Api.Test", "plain");
            tracing::warn!(target: "Stellarr.Api.Other", count = 3, "with a field");
            tracing::error!("failed");
        });

        let lines = buffer.all();
        let kept: Vec<_> = lines
            .iter()
            .rev()
            .map(|line| {
                (
                    line.sequence,
                    line.level.name(),
                    line.category.as_str(),
                    line.message.as_str(),
                )
            })
            .collect();
        assert_eq!(kept[0], (0, "info", "Stellarr.Api.Test", "plain"));
        assert_eq!(
            kept[1],
            (1, "warn", "Stellarr.Api.Other", "with a field count=3")
        );
        assert_eq!(kept[2].1, "error");
        assert_eq!(kept.len(), 3);
        assert!(lines.iter().all(|line| line.trace_id == BACKGROUND_TRACE));
    }

    #[test]
    fn the_setup_code_is_not_kept_for_the_panel() {
        let buffer = buffered(|| {
            tracing::warn!(target: OPERATOR_ONLY_TARGET, "Setup code: ABCD-1234. Open /setup.");
            tracing::warn!(target: STORE_NOT_READY_TARGET, "The object store is not ready: its answer");
            tracing::warn!(target: "Stellarr.Api.Other", "kept");
        });

        let lines = buffer.all();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].message, "kept");
    }

    #[tokio::test]
    async fn a_line_written_while_serving_a_request_carries_its_identifier() {
        let buffer = LogBuffer::new();
        let subscriber = tracing_subscriber::registry().with(buffer.sink());
        let _installed = tracing::subscriber::set_default(subscriber);

        trace::scope("00-a-b-00".to_string(), async {
            tracing::info!("in a request")
        })
        .await;

        assert_eq!(buffer.all()[0].trace_id, "00-a-b-00");
    }

    #[test]
    fn the_oldest_line_falls_out_when_the_buffer_is_full_and_the_cursor_stays_stable() {
        let buffer = buffered(|| {
            for number in 0..LOG_CAPACITY + 5 {
                tracing::info!("line {number}");
            }
        });

        let all = buffer.all();
        assert_eq!(all.len(), LOG_CAPACITY);
        assert_eq!(all[0].sequence, (LOG_CAPACITY + 4) as u64);
        assert_eq!(all[LOG_CAPACITY - 1].sequence, 5);
        let first = buffer.page(None, 3, |_| true);
        assert_eq!(
            first.lines.iter().map(|l| l.sequence).collect::<Vec<_>>(),
            [10_004, 10_003, 10_002]
        );
        assert!(first.has_more);
        let next = buffer.page(Some(10_002), 2, |_| true);
        assert_eq!(
            next.lines.iter().map(|l| l.sequence).collect::<Vec<_>>(),
            [10_001, 10_000]
        );
        // A cursor older than the buffer reaches nothing.
        let gone = buffer.page(Some(5), 10, |_| true);
        assert!(gone.lines.is_empty() && !gone.has_more);
        // A cursor from the future is the newest line.
        assert_eq!(
            buffer.page(Some(u64::MAX), 1, |_| true).lines[0].sequence,
            10_004
        );
    }

    #[test]
    fn a_page_skips_what_the_filter_refuses_and_says_whether_more_remain() {
        let buffer = buffered(|| {
            for number in 0..10 {
                tracing::info!("n{number}");
            }
        });

        let even = buffer.page(None, 2, |line| line.sequence % 2 == 0);
        assert_eq!(
            even.lines.iter().map(|l| l.sequence).collect::<Vec<_>>(),
            [8, 6]
        );
        assert!(even.has_more);
        let all = buffer.page(None, 5, |line| line.sequence % 2 == 0);
        assert_eq!(all.lines.len(), 5);
        assert!(!all.has_more);
    }

    #[test]
    fn a_long_line_is_cut_on_a_character_boundary() {
        let buffer = buffered(|| tracing::info!("{}", "\u{00e9}".repeat(5000)));

        let line = &buffer.all()[0];
        assert!(line.message.len() <= MAX_MESSAGE_BYTES + '\u{2026}'.len_utf8());
        assert!(line.message.ends_with('\u{2026}'));
    }
}
