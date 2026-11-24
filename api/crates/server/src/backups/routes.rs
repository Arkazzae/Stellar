//! The panel's routes for backups: the list, making one, verifying one,
//! downloading one, and the settings of the schedule and of the offsite
//! destination.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use serde::Serialize;
use serde_json::Value;
use stellarr_messages::MessageKey;

use super::jobs::Job;
use super::listing::{self, Entry};
use super::parts::{self, Parts};
use super::policy::{self, Destination, Schedule, Source};
use super::schedule::{self, Status};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::file::{self, Download};
use crate::http::{ApiError, ApiResult, Endpoints, Json, Path, Problem, json_response};
use crate::panel::operations::{self, MAX_RETENTION_DAYS};
use crate::panel::{self, FreshInstanceOwner};
use crate::permissions::membership::InstanceOwner;
use crate::settings::{self, BackupSettings};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/cockpit/backups", list)
        .authorized()
        .get("/api/cockpit/backups/settings", read_schedule)
        .authorized()
        .put("/api/cockpit/backups/settings", replace_schedule)
        .json_body()
        .authorized()
        .delete("/api/cockpit/backups/settings", reset_schedule)
        .authorized()
        .get("/api/cockpit/integrations/cloudflare/r2", read_destination)
        .authorized()
        .put(
            "/api/cockpit/integrations/cloudflare/r2",
            replace_destination,
        )
        .json_body()
        .authorized()
        .delete("/api/cockpit/integrations/cloudflare/r2", reset_destination)
        .authorized()
        .post("/api/cockpit/backups", create)
        .json_body()
        .authorized()
        .post("/api/cockpit/backups/{backup_id}/verify", verify)
        .authorized()
        .get("/api/cockpit/backups/{backup_id}/download", download)
        .authorized()
}

// ---------------------------------------------------------------------------
// What the panel is told.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JobDto {
    id: String,
    parts: Vec<&'static str>,
    started_at: Timestamp,
    phase: &'static str,
    objects_done: u32,
    objects_total: u32,
}

fn job_dto(job: &Job) -> JobDto {
    JobDto {
        id: job.id.clone(),
        parts: job.parts.names(),
        started_at: job.started_at,
        phase: job.progress.phase(),
        objects_done: job.progress.objects_done(),
        objects_total: job.progress.objects_total(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureDto {
    id: String,
    message: String,
    at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupsDto {
    backups: Vec<Entry>,
    retention_days: i64,
    job: Option<JobDto>,
    last_failure: Option<FailureDto>,
    automation: Status,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyDto {
    enabled: bool,
    interval_hours: i64,
    retry_minutes: i64,
    initial_delay_minutes: i64,
    retention_days: i64,
    cleanup_enabled: bool,
    source: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DestinationDto {
    enabled: bool,
    mode: String,
    endpoint: String,
    region: String,
    bucket: String,
    prefix: String,
    source: &'static str,
}

fn destination_dto(destination: Destination, source: Source) -> DestinationDto {
    DestinationDto {
        enabled: destination.enabled,
        mode: destination.mode,
        endpoint: destination.endpoint,
        region: destination.region,
        bucket: destination.bucket,
        prefix: destination.prefix,
        source: source.name(),
    }
}

// ---------------------------------------------------------------------------
// The list.

async fn list(
    State(state): State<AppState>,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Json<BackupsDto>> {
    let sealer = state.sealer().clone();
    let settings = state.settings().clone();
    let (retention, schedule, remote) = state
        .database()
        .read(move |connection| -> Result<_, DbError> {
            let retention = operations::of_server(connection, server)?;
            let (schedule, _) = policy::current(connection, &sealer, &settings.backups)?;
            let (destination, _) =
                policy::current_destination(connection, &sealer, &settings.cloudflare_r2)?;
            Ok((retention.backup_days, schedule, destination.enabled))
        })
        .await?;
    let directory = listing::directory(&state.settings().data_root);
    let backups = tokio::task::spawn_blocking(move || listing::list(&directory))
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(BackupsDto {
        backups,
        retention_days: retention,
        job: state.backups().running().map(|job| job_dto(&job)),
        last_failure: state.backups().last_failure().map(|failure| FailureDto {
            id: failure.id,
            message: failure.message,
            at: failure.at,
        }),
        automation: schedule::status(&state.backups().checkpoint(), &schedule, remote),
    }))
}

// ---------------------------------------------------------------------------
// Making one.

/// Without a body: the database alone, copied at once into a plain SQLite
/// file. With one naming the files or the keys: an archive written in the
/// background, answered with 202 and followed through the backup list.
async fn create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body: wanted,
    } = panel::bind_then_gate(&state, request, false, |body| match body {
        None => Ok(Parts::DATABASE),
        Some(Value::Object(members)) => {
            let mut wanted = Parts::NONE;
            for (name, part) in [
                ("database", Parts::DATABASE),
                ("objects", Parts::OBJECTS),
                ("keys", Parts::KEYS),
                ("instance", Parts::INSTANCE),
            ] {
                if flag(member(&members, name))? {
                    wanted = wanted.with(part);
                }
            }
            Ok(wanted)
        }
        Some(_) => Err(ApiError::bad_request()),
    })
    .await?;
    if wanted.is_none() {
        let message = state
            .messages()
            .get(MessageKey::CockpitBackupNoParts, &[])?;
        return Err(Problem::validation(vec![("parts".into(), vec![message])]).into());
    }

    if wanted != Parts::DATABASE {
        return match state
            .backups()
            .try_start(&state, wanted, server, caller.user, false)
        {
            Some(started) => {
                let mut response = json_response(StatusCode::ACCEPTED, &job_dto(&started.job));
                put_location(&mut response, &started.job.id);
                Ok(response)
            }
            None => {
                let title = state
                    .messages()
                    .get(MessageKey::CockpitBackupRunning, &[])?;
                Err(Problem::new(StatusCode::CONFLICT).title(title).into())
            }
        };
    }

    let directory = listing::directory(&state.settings().data_root);
    let id = listing::new_id(Timestamp::now(), listing::DATABASE_EXTENSION);
    let path = directory.join(&id);
    let source = state.database().path().to_path_buf();
    let target = path.clone();
    let folder = directory.clone();
    // Copied from the database this server is using, not from the setting it
    // was configured with, so that a backup is always of the live data.
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&folder)?;
        super::snapshot::copy_database(&source, &target)
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;

    let sentence = state
        .messages()
        .get(MessageKey::AuditBackupCreated, &[("id", &id)])?;
    state
        .database()
        .write(move |transaction| -> Result<(), DbError> {
            panel::record(transaction, server, caller.user, sentence)?;
            Ok(())
        })
        .await?;
    let entry = tokio::task::spawn_blocking(move || {
        let size = std::fs::metadata(&path)?;
        Ok::<_, std::io::Error>((size.len(), listing::created(&size)))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    let created = Entry {
        id: id.clone(),
        size_bytes: i64::try_from(entry.0).unwrap_or(i64::MAX),
        created_at: entry.1,
        verified_at: None,
        parts: vec![parts::DATABASE.to_string()],
        missing_objects: 0,
    };
    let mut response = json_response(StatusCode::CREATED, &created);
    put_location(&mut response, &id);
    Ok(response)
}

fn put_location(response: &mut Response, id: &str) {
    if let Ok(value) = header::HeaderValue::from_str(&format!("/api/cockpit/backups/{id}")) {
        response.headers_mut().insert(header::LOCATION, value);
    }
}

// ---------------------------------------------------------------------------
// Verifying and downloading one.

async fn verify(
    State(state): State<AppState>,
    caller: Caller,
    InstanceOwner(server): InstanceOwner,
    Path(backup_id): Path<String>,
) -> ApiResult<Json<Entry>> {
    let directory = listing::directory(&state.settings().data_root);
    let Some(path) = listing::resolve(&directory, &backup_id).filter(|path| path.is_file()) else {
        return Err(ApiError::not_found());
    };
    let folder = directory.clone();
    let checked = path.clone();
    let verdict = tokio::task::spawn_blocking(move || {
        if listing::is_archive(&checked) {
            super::archive::verify(&checked, &folder)
        } else {
            super::snapshot::check_integrity(&checked)
        }
    })
    .await
    .map_err(ApiError::internal)?;
    if !verdict.eq_ignore_ascii_case("ok") {
        let title = state
            .messages()
            .get(MessageKey::CockpitBackupFailedIntegrity, &[])?;
        return Err(Problem::new(StatusCode::CONFLICT)
            .title(title)
            .detail(verdict)
            .into());
    }

    let marked = path.clone();
    tokio::task::spawn_blocking(move || listing::mark_verified(&marked, Timestamp::now()))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    let sentence = state
        .messages()
        .get(MessageKey::AuditBackupVerified, &[("id", &backup_id)])?;
    state
        .database()
        .write(move |transaction| -> Result<(), DbError> {
            panel::record(transaction, server, caller.user, sentence)?;
            Ok(())
        })
        .await?;
    let entry = tokio::task::spawn_blocking(move || listing::describe(&path))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    Ok(Json(entry))
}

async fn download(
    State(state): State<AppState>,
    FreshInstanceOwner(_): FreshInstanceOwner,
    headers: HeaderMap,
    Path(backup_id): Path<String>,
) -> ApiResult<Response> {
    let directory = listing::directory(&state.settings().data_root);
    let Some(path) = listing::resolve(&directory, &backup_id).filter(|path| path.is_file()) else {
        return Err(ApiError::not_found());
    };
    let opened = tokio::fs::File::open(&path).await;
    let Ok(opened) = opened else {
        return Err(ApiError::not_found());
    };
    let metadata = opened.metadata().await.map_err(ApiError::internal)?;
    let content_type = if listing::is_archive(&path) {
        "application/zip"
    } else {
        "application/vnd.sqlite3"
    };
    Ok(file::answer(
        &headers,
        Download {
            file: opened,
            length: metadata.len(),
            content_type,
            file_name: backup_id,
            modified: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|elapsed| Timestamp::from_unix_seconds(elapsed.as_secs() as i64))
                .unwrap_or_else(Timestamp::now),
        },
    )
    .await)
}

// ---------------------------------------------------------------------------
// The schedule.

/// A member of a JSON object by name, without regard to letter case, the last
/// of several winning, as the first server read request bodies.
fn member<'a>(members: &'a serde_json::Map<String, Value>, name: &str) -> Option<&'a Value> {
    members
        .iter()
        .rev()
        .find(|(have, _)| have.eq_ignore_ascii_case(name))
        .map(|(_, value)| value)
}

/// A whole number the way the first server read one from JSON: a number, or
/// text holding one, that fits 32 bits.
fn whole_number(value: Option<&Value>) -> Result<i64, ApiError> {
    let Some(value) = value else {
        return Ok(0);
    };
    let number = match value {
        Value::Number(number) => number.as_i64(),
        // Text holding a number counts, with nothing around it.
        Value::String(text) => {
            let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
            (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| text.parse::<i64>().ok())
                .flatten()
        }
        _ => None,
    };
    number
        .filter(|number| i32::try_from(*number).is_ok())
        .ok_or_else(ApiError::bad_request)
}

fn flag(value: Option<&Value>) -> Result<bool, ApiError> {
    match value {
        None => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(_) => Err(ApiError::bad_request()),
    }
}

struct ScheduleRequest {
    schedule: Schedule,
    retention_days: i64,
}

fn read_schedule_request(body: Value) -> Result<ScheduleRequest, ApiError> {
    let Value::Object(members) = body else {
        return Err(ApiError::bad_request());
    };
    Ok(ScheduleRequest {
        schedule: Schedule {
            enabled: flag(member(&members, "enabled"))?,
            interval_hours: whole_number(member(&members, "intervalHours"))?,
            retry_minutes: whole_number(member(&members, "retryMinutes"))?,
            initial_delay_minutes: whole_number(member(&members, "initialDelayMinutes"))?,
        },
        retention_days: whole_number(member(&members, "retentionDays"))?,
    })
}

async fn policy_of(state: &AppState, server: Id) -> Result<PolicyDto, ApiError> {
    let sealer = state.sealer().clone();
    let defaults = state.settings().backups;
    Ok(state
        .database()
        .read(move |connection| -> Result<PolicyDto, DbError> {
            let retention = operations::of_server(connection, server)?;
            let (schedule, source) = policy::current(connection, &sealer, &defaults)?;
            Ok(policy_dto(schedule, source, &retention))
        })
        .await?)
}

fn policy_dto(schedule: Schedule, source: Source, retention: &operations::Retention) -> PolicyDto {
    PolicyDto {
        enabled: schedule.enabled,
        interval_hours: schedule.interval_hours,
        retry_minutes: schedule.retry_minutes,
        initial_delay_minutes: schedule.initial_delay_minutes,
        retention_days: retention.backup_days,
        cleanup_enabled: retention.automatic_cleanup,
        source: source.name(),
    }
}

async fn read_schedule(
    State(state): State<AppState>,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Json<PolicyDto>> {
    Ok(Json(policy_of(&state, server).await?))
}

fn range_message(member: &str, (low, high): (i64, i64)) -> (String, Vec<String>) {
    (
        member.to_string(),
        vec![format!(
            "The field {member} must be between {low} and {high}."
        )],
    )
}

async fn replace_schedule(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> ApiResult<Json<PolicyDto>> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, |body| {
        body.map(read_schedule_request).transpose()
    })
    .await?;
    // Nothing to bind is a bad request, but only once the gate has let the
    // caller by.
    let request = body.ok_or_else(ApiError::bad_request)?;
    let schedule = request.schedule;
    let mut problems = Vec::new();
    for (member, value, range) in [
        (
            "IntervalHours",
            schedule.interval_hours,
            BackupSettings::INTERVAL_HOURS,
        ),
        (
            "RetryMinutes",
            schedule.retry_minutes,
            BackupSettings::RETRY_MINUTES,
        ),
        (
            "InitialDelayMinutes",
            schedule.initial_delay_minutes,
            BackupSettings::INITIAL_DELAY_MINUTES,
        ),
    ] {
        if !(range.0..=range.1).contains(&value) {
            problems.push(range_message(member, range));
        }
    }
    if !(1..=MAX_RETENTION_DAYS).contains(&request.retention_days) {
        let message = state.messages().get(
            MessageKey::CockpitBackupRetentionRange,
            &[("max", &MAX_RETENTION_DAYS)],
        )?;
        problems.push(("retentionDays".to_string(), vec![message]));
    }
    if !problems.is_empty() {
        return Err(Problem::validation(problems).into());
    }

    let sentence = panel::settings_updated(&state)?;
    let sealer = state.sealer().clone();
    let settings = state.settings().clone();
    let retention_days = request.retention_days;
    let written = state
        .database()
        .try_write(move |transaction| {
            let created = server_created_at(transaction, server)?;
            operations::ensure(transaction, &settings, server, created)?;
            operations::set_backup_retention(
                transaction,
                server,
                retention_days,
                Timestamp::now(),
            )?;
            policy::save_schedule(transaction, &sealer, &schedule)?;
            panel::record(transaction, server, caller.user, sentence)?;
            Ok(operations::of_server(transaction, server)?)
        })
        .await?;
    let Some(retention) = written else {
        return Err(ApiError::conflict());
    };
    Ok(Json(policy_dto(schedule, Source::Panel, &retention)))
}

fn server_created_at(connection: &rusqlite::Connection, server: Id) -> Result<Timestamp, DbError> {
    Ok(connection.query_row(
        "SELECT \"CreatedAt\" FROM \"Servers\" WHERE \"Id\" = ?1",
        [server],
        |row| row.get(0),
    )?)
}

async fn reset_schedule(
    State(state): State<AppState>,
    caller: Caller,
    FreshInstanceOwner(server): FreshInstanceOwner,
) -> ApiResult<Json<PolicyDto>> {
    let sentence = panel::settings_updated(&state)?;
    let written = state
        .database()
        .try_write(move |transaction| {
            policy::reset_schedule(transaction)?;
            panel::record(transaction, server, caller.user, sentence)?;
            Ok(())
        })
        .await?;
    if written.is_none() {
        return Err(ApiError::conflict());
    }
    Ok(Json(policy_of(&state, server).await?))
}

// ---------------------------------------------------------------------------
// The offsite destination.

async fn read_destination(
    State(state): State<AppState>,
    InstanceOwner(_): InstanceOwner,
) -> ApiResult<Json<DestinationDto>> {
    Ok(Json(current_destination(&state).await?))
}

async fn current_destination(state: &AppState) -> Result<DestinationDto, ApiError> {
    let sealer = state.sealer().clone();
    let defaults = state.settings().cloudflare_r2.clone();
    let (destination, source) = state
        .database()
        .read(move |connection| -> Result<_, DbError> {
            Ok(policy::current_destination(connection, &sealer, &defaults)?)
        })
        .await?;
    Ok(destination_dto(destination, source))
}

/// A string member of the request: none when it is missing or null, which the
/// route answers like one that is too long; anything but text is a bad request.
fn text_member(
    members: &serde_json::Map<String, Value>,
    name: &str,
) -> Result<Option<String>, ApiError> {
    match member(members, name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(ApiError::bad_request()),
    }
}

async fn replace_destination(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> ApiResult<Json<DestinationDto>> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, |body| match body {
        None => Ok(None),
        Some(Value::Object(members)) => Ok(Some((
            flag(member(&members, "enabled"))?,
            text_member(&members, "mode")?,
            text_member(&members, "endpoint")?,
            text_member(&members, "region")?,
            text_member(&members, "bucket")?,
            text_member(&members, "prefix")?,
        ))),
        Some(_) => Err(ApiError::bad_request()),
    })
    .await?;
    let (enabled, mode, endpoint, region, bucket, prefix) =
        body.ok_or_else(ApiError::bad_request)?;
    let units = |text: &str| text.encode_utf16().count();
    let fields_invalid = match (&mode, &endpoint, &region, &bucket, &prefix) {
        (Some(mode), Some(endpoint), Some(region), Some(bucket), Some(prefix)) => {
            units(mode) > 16
                || units(endpoint) > 2048
                || units(region) > 100
                || units(bucket) > 255
                || units(prefix) > 512
                || !settings::is_known_r2_mode(mode)
        }
        _ => true,
    };
    if fields_invalid {
        let message = state
            .messages()
            .get(MessageKey::CockpitCloudflareR2FieldsInvalid, &[])?;
        return Err(Problem::validation(vec![("settings".into(), vec![message])]).into());
    }
    let (Some(mode), Some(endpoint), Some(region), Some(bucket), Some(prefix)) =
        (mode, endpoint, region, bucket, prefix)
    else {
        return Err(ApiError::bad_request());
    };
    let destination = Destination {
        enabled,
        mode: mode.trim().to_string(),
        endpoint: endpoint.trim().to_string(),
        region: region.trim().to_string(),
        bucket: bucket.trim().to_string(),
        prefix: prefix.trim().to_string(),
    };

    let sealer = state.sealer().clone();
    let environment = state.settings().secrets.clone();
    let candidate = destination.clone();
    let missing_credentials = state
        .database()
        .read(move |connection| -> Result<bool, DbError> {
            let resolved = crate::secrets::read(connection, &environment, &sealer)?;
            let set = |definition| resolved.get(definition).is_set();
            Ok(match candidate.mode.as_str() {
                settings::R2_MODE_S3 => {
                    !set(&crate::secrets::CLOUDFLARE_R2_ACCESS_KEY)
                        || !set(&crate::secrets::CLOUDFLARE_R2_SECRET_KEY)
                }
                settings::R2_MODE_WORKER => !set(&crate::secrets::CLOUDFLARE_R2_WORKER_TOKEN),
                _ => true,
            })
        })
        .await?;
    if destination.enabled && (!destination.is_valid() || missing_credentials) {
        let message = state
            .messages()
            .get(MessageKey::CockpitCloudflareR2Invalid, &[])?;
        return Err(Problem::validation(vec![("destination".into(), vec![message])]).into());
    }

    let sentence = panel::settings_updated(&state)?;
    let sealer = state.sealer().clone();
    let saved = destination.clone();
    let written = state
        .database()
        .try_write(move |transaction| {
            policy::save_destination(transaction, &sealer, &saved)?;
            panel::record(transaction, server, caller.user, sentence)?;
            Ok(())
        })
        .await?;
    if written.is_none() {
        return Err(ApiError::conflict());
    }
    Ok(Json(destination_dto(destination, Source::Panel)))
}

async fn reset_destination(
    State(state): State<AppState>,
    caller: Caller,
    FreshInstanceOwner(server): FreshInstanceOwner,
) -> ApiResult<Json<DestinationDto>> {
    let sentence = panel::settings_updated(&state)?;
    let written = state
        .database()
        .try_write(move |transaction| {
            policy::reset_destination(transaction)?;
            panel::record(transaction, server, caller.user, sentence)?;
            Ok(())
        })
        .await?;
    if written.is_none() {
        return Err(ApiError::conflict());
    }
    Ok(Json(current_destination(&state).await?))
}
