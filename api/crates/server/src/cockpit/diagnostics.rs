//! How the process is doing and what it logged.

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use serde::Serialize;
use stellarr_messages::MessageKey;

use super::runtime::{self, Process, Sample};
use crate::database::DbError;
use crate::http::{ApiResult, QueryValues, json_response};
use crate::logging::{LOG_CAPACITY, LineLevel, LogLine};
use crate::permissions::membership::InstanceOwner;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const DEFAULT_LOG_LIMIT: i32 = 100;
const MAX_LOG_LIMIT: i32 = 500;
const MAX_LOG_SOURCES: usize = 50;

#[derive(Serialize)]
struct Service {
    name: &'static str,
    state: &'static str,
    detail: String,
    id: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Runtime {
    started_at: Timestamp,
    uptime_seconds: i64,
    working_set_bytes: i64,
    gc_heap_bytes: i64,
    thread_count: i64,
    realtime_connections: usize,
    voice_rooms: usize,
    #[serde(serialize_with = "super::plain_number")]
    database_latency_ms: f64,
    services: Vec<Service>,
    #[serde(serialize_with = "super::plain_number")]
    cpu_percent: f64,
    sample_seconds: u64,
    history: Vec<Sample>,
}

/// The object store's state as the latest sample found it; healthy until a
/// sample has checked it.
fn last_storage_check(history: &[Sample]) -> Option<bool> {
    history.iter().rev().find_map(|sample| sample.storage_up)
}

pub(super) async fn runtime(
    State(state): State<AppState>,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Response> {
    let reading = state.clone();
    let connections = state
        .database()
        .read(move |db| -> Result<usize, DbError> {
            let mut statement =
                db.prepare("SELECT \"UserId\" FROM \"ServerMembers\" WHERE \"ServerId\" = ?1")?;
            let members = statement
                .query_map([server], |row| row.get::<_, Id>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(members
                .iter()
                .map(|id| reading.presence().connection_ids(*id).len())
                .sum())
        })
        .await?;
    let (database_ready, latency) = runtime::check_database(&state).await;
    let process = Process::now();
    let history = state.cockpit().history().samples();
    let messages = state.messages();
    let maintenance = state.settings().maintenance_enabled;
    let storage_inactive = state.settings().storage.skip_initialization;
    let storage_state = match last_storage_check(&history) {
        Some(false) => "error",
        _ => "healthy",
    };
    let storage_detail = match last_storage_check(&history) {
        Some(true) => MessageKey::CockpitStorageAnswering,
        Some(false) => MessageKey::CockpitStorageDown,
        None => MessageKey::CockpitStorageChecked,
    };
    let services = vec![
        Service {
            name: "API",
            state: "healthy",
            detail: format!("Rust {}", crate::version::server()),
            id: "api",
        },
        Service {
            name: "SQLite",
            state: if database_ready { "healthy" } else { "error" },
            detail: messages.get(
                if database_ready {
                    MessageKey::CockpitDatabaseHealthy
                } else {
                    MessageKey::CockpitDatabaseDown
                },
                &[],
            )?,
            id: "database",
        },
        Service {
            name: "SignalR",
            state: "healthy",
            detail: messages.get(
                MessageKey::CockpitRealtimeConnections,
                &[("count", &connections)],
            )?,
            id: "realtime",
        },
        Service {
            name: "Maintenance worker",
            state: if maintenance { "healthy" } else { "inactive" },
            detail: messages.get(
                if maintenance {
                    MessageKey::CockpitMaintenanceRunning
                } else {
                    MessageKey::CockpitMaintenanceOff
                },
                &[],
            )?,
            id: "maintenance",
        },
        Service {
            name: "Object storage",
            state: if storage_inactive { "inactive" } else { storage_state },
            detail: messages.get(
                if storage_inactive {
                    MessageKey::CockpitStorageInitializationOff
                } else {
                    storage_detail
                },
                &[],
            )?,
            id: "storage",
        },
    ];
    let answer = Runtime {
        started_at: state.cockpit().history().started_at(),
        uptime_seconds: state.cockpit().history().uptime_seconds().max(0),
        working_set_bytes: process.working_set_bytes,
        gc_heap_bytes: process.heap_bytes,
        thread_count: process.threads,
        realtime_connections: connections,
        // The rooms of the voice group, which is not part of this server yet.
        voice_rooms: 0,
        database_latency_ms: (latency * 100.0).round() / 100.0,
        services,
        cpu_percent: history.last().map_or(0.0, |sample| sample.cpu_percent),
        sample_seconds: runtime::INTERVAL.as_secs(),
        history,
    };
    Ok(json_response(StatusCode::OK, &answer))
}

/// The filters of the logs page; an empty value lets every line through.
struct Filter {
    levels: Option<Vec<&'static str>>,
    source: Option<String>,
    search: Option<String>,
}

impl Filter {
    fn from(level: Option<&str>, source: Option<&str>, search: Option<&str>) -> Filter {
        let mut levels: Vec<&'static str> = Vec::new();
        for word in level.unwrap_or("").split(',') {
            let word = word.trim().to_ascii_lowercase();
            if let Some(known) = ["info", "warn", "error"].into_iter().find(|k| *k == word)
                && !levels.contains(&known)
            {
                levels.push(known);
            }
        }
        let trimmed = |text: Option<&str>| {
            text.map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        Filter {
            levels: (!levels.is_empty()).then_some(levels),
            source: trimmed(source),
            // Compared in capitals, letter by letter, as the first server
            // compared without regard to case.
            search: trimmed(search).map(|text| crate::instance::names::to_upper(&text)),
        }
    }

    fn matches(&self, line: &LogLine) -> bool {
        self.levels
            .as_ref()
            .is_none_or(|levels| levels.contains(&line.level.name()))
            && self.matches_ignoring_level(line)
    }

    fn matches_ignoring_level(&self, line: &LogLine) -> bool {
        self.source.as_ref().is_none_or(|source| &line.category == source)
            && self.search.as_ref().is_none_or(|search| {
                [&line.message, &line.category, &line.trace_id]
                    .iter()
                    .any(|text| crate::instance::names::to_upper(text).contains(search.as_str()))
            })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    sequence: u64,
    at: Timestamp,
    level: &'static str,
    source: String,
    message: String,
    trace_id: String,
}

#[derive(Serialize)]
struct Levels {
    info: usize,
    warn: usize,
    error: usize,
}

#[derive(Serialize)]
struct Source {
    name: String,
    count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Logs {
    entries: Vec<Entry>,
    next_before: Option<u64>,
    levels: Levels,
    sources: Vec<Source>,
    capacity: usize,
}

/// A page of the log buffer, newest first, filtered by level (a
/// comma-separated list of info, warn and error), source and a search in the
/// message, source and trace id. `before` is the cursor of the previous page.
pub(super) async fn logs(
    State(state): State<AppState>,
    query: QueryValues,
    InstanceOwner(_): InstanceOwner,
) -> ApiResult<Response> {
    let before = query.long("before")?;
    let limit = query.int("limit")?;
    let filter = Filter::from(
        query.text("level").as_deref(),
        query.text("source").as_deref(),
        query.text("search").as_deref(),
    );
    let limit = limit
        .unwrap_or(DEFAULT_LOG_LIMIT)
        .clamp(1, MAX_LOG_LIMIT) as usize;
    let (page, all) = match state.cockpit().log() {
        Some(buffer) => {
            // A cursor below the first line reaches nothing.
            let before = before.map(|cursor| u64::try_from(cursor).unwrap_or(0));
            (buffer.page(before, limit, |line| filter.matches(line)), buffer.all())
        }
        None => (
            crate::logging::LogPage {
                lines: Vec::new(),
                has_more: false,
            },
            Vec::new(),
        ),
    };
    let mut levels = Levels {
        info: 0,
        warn: 0,
        error: 0,
    };
    for line in all.iter().filter(|line| filter.matches_ignoring_level(line)) {
        match line.level {
            LineLevel::Info => levels.info += 1,
            LineLevel::Warn => levels.warn += 1,
            LineLevel::Error => levels.error += 1,
        }
    }
    let mut counted: HashMap<&str, usize> = HashMap::new();
    for line in &all {
        *counted.entry(line.category.as_str()).or_default() += 1;
    }
    let mut sources: Vec<Source> = counted
        .into_iter()
        .map(|(name, count)| Source {
            name: name.to_string(),
            count,
        })
        .collect();
    sources.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    sources.truncate(MAX_LOG_SOURCES);
    let next_before = page.has_more.then(|| page.lines.last().map(|l| l.sequence)).flatten();
    let answer = Logs {
        entries: page
            .lines
            .into_iter()
            .map(|line| Entry {
                sequence: line.sequence,
                at: line.at,
                level: line.level.name(),
                source: line.category,
                message: line.message,
                trace_id: line.trace_id,
            })
            .collect(),
        next_before,
        levels,
        sources,
        capacity: LOG_CAPACITY,
    };
    Ok(json_response(StatusCode::OK, &answer))
}

/// Keeps a value inside its tab-separated cell and row.
fn single_line(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

/// A moment as the round-trip format writes it: seven digits of fraction.
fn round_trip(at: Timestamp) -> String {
    let text = at.to_wire_text();
    let body = text.trim_end_matches('Z');
    match body.split_once('.') {
        Some((whole, fraction)) => format!("{whole}.{fraction:0<7}Z"),
        None => format!("{body}.0000000Z"),
    }
}

/// Every line of the buffer that the filters keep, as tab-separated text,
/// oldest line first.
pub(super) async fn export(
    State(state): State<AppState>,
    query: QueryValues,
    InstanceOwner(_): InstanceOwner,
) -> ApiResult<Response> {
    let filter = Filter::from(
        query.text("level").as_deref(),
        query.text("source").as_deref(),
        query.text("search").as_deref(),
    );
    let lines = state
        .cockpit()
        .log()
        .map(|buffer| buffer.page(None, LOG_CAPACITY, |line| filter.matches(line)).lines)
        .unwrap_or_default();
    let mut text = String::from("at\tlevel\tsource\ttrace\tmessage\n");
    for line in lines.iter().rev() {
        text.push_str(&round_trip(line.at));
        text.push('\t');
        text.push_str(line.level.name());
        text.push('\t');
        text.push_str(&single_line(&line.category));
        text.push('\t');
        text.push_str(&single_line(&line.trace_id));
        text.push('\t');
        text.push_str(&single_line(&line.message));
        text.push('\n');
    }
    let name = {
        let now = Timestamp::now().to_wire_text();
        let digits: String = now.chars().filter(char::is_ascii_digit).take(14).collect();
        format!("stellarr-logs-{}-{}.tsv", &digits[..8], &digits[8..])
    };
    let mut response = Response::new(Body::from(text));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/tab-separated-values; charset=utf-8"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename={name}; filename*=UTF-8''{name}")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(level: LineLevel, category: &str, message: &str) -> LogLine {
        LogLine {
            sequence: 0,
            at: Timestamp::now(),
            level,
            category: category.into(),
            message: message.into(),
            trace_id: "00-abc-def-00".into(),
        }
    }

    #[test]
    fn filters_read_levels_loosely_and_search_without_regard_to_case() {
        let filter = Filter::from(Some(" WARN, bogus ,error,,warn"), Some(" A.B "), Some(" ßx hello "));
        assert_eq!(filter.levels, Some(vec!["warn", "error"]));
        assert_eq!(filter.source.as_deref(), Some("A.B"));
        let nothing = Filter::from(Some(" , ,x"), Some("  "), None);
        assert!(nothing.levels.is_none() && nothing.source.is_none() && nothing.search.is_none());

        let found = Filter::from(None, None, Some("HELLO wor"));
        assert!(found.matches(&line(LineLevel::Info, "x", "say Hello World")));
        assert!(found.matches(&line(LineLevel::Info, "hello wor", "")));
        assert!(!found.matches(&line(LineLevel::Info, "x", "hello")));
        let by_trace = Filter::from(None, None, Some("ABC-DEF"));
        assert!(by_trace.matches(&line(LineLevel::Info, "x", "m")));
        let warn = Filter::from(Some("warn"), None, None);
        assert!(!warn.matches(&line(LineLevel::Info, "x", "m")));
        assert!(warn.matches_ignoring_level(&line(LineLevel::Info, "x", "m")));
    }

    #[test]
    fn a_cell_stays_in_its_row() {
        assert_eq!(single_line("a\tb\r\nc<script>\u{7}"), "a b  c<script>\u{7}");
    }

    #[test]
    fn a_moment_has_seven_digits_of_fraction_in_the_export() {
        let at = Timestamp::parse("2026-11-24T10:00:00.5Z").unwrap();
        assert_eq!(round_trip(at), "2026-11-24T10:00:00.5000000Z");
        let whole = Timestamp::parse("2026-11-24T10:00:00Z").unwrap();
        assert_eq!(round_trip(whole), "2026-11-24T10:00:00.0000000Z");
    }
}
