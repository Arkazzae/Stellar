//! Owner-approved, immutable activity releases and their voice-bound sessions.
mod artwork;
mod authority;
mod live;
pub(crate) mod hub;
mod progress;
mod releases;
pub(crate) mod updates;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use tokio::sync::Semaphore;

use crate::http::{ApiError, ApiResult, Endpoints, Json, Path, Problem};
use crate::panel::bind_then_gate;
use crate::permissions::membership::InstanceOwner;
use crate::state::AppState;
use crate::values::{Id, Timestamp};
pub(crate) use live::Sessions;

static DOWNLOADS: Semaphore = Semaphore::const_new(2);
fn invalid(message: impl Into<String>) -> ApiError {
    Problem::invalid_field("activity", message.into())
}
fn activity_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn hostname(s: &str) -> bool {
    s.len() <= 253
        && s.contains('.')
        && s.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

#[derive(Clone, Deserialize, Serialize)]
struct Connect {
    host: String,
    why: String,
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
enum LaunchMode {
    Voice,
    Personal,
}
fn default_launch_modes() -> Vec<LaunchMode> {
    vec![LaunchMode::Voice]
}
/// What a manifest may ask for; `sdk/dev/files.js` accepts the same.
const PERMISSIONS: [&str; 3] = ["media.microphoneLevel", "media.cameraTile", "media.gifs"];

#[derive(Clone, Deserialize, Serialize)]
struct Manifest {
    #[serde(default = "default_launch_modes", rename = "launchModes")]
    launch_modes: Vec<LaunchMode>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    cover: Option<String>,
    protocol: u16,
    id: String,
    name: String,
    description: String,
    version: String,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    connects: Vec<Connect>,
    #[serde(default)]
    devices: Option<Vec<String>>,
    #[serde(default)]
    seats: Option<u16>,
    #[serde(default)]
    authority: Option<String>,
}
impl Manifest {
    fn check(&self) -> ApiResult<()> {
        if self.launch_modes.is_empty()
            || self.launch_modes.len() != self.launch_modes.iter().collect::<HashSet<_>>().len()
        {
            return Err(invalid(
                "launchModes must list voice or personal at least once, without duplicates.",
            ));
        }
        if self.protocol != 1 {
            return Err(invalid(format!(
                "Protocol {} is unsupported. This instance supports protocol 1.",
                self.protocol
            )));
        }
        if let Some(name) = self
            .permissions
            .iter()
            .find(|p| !PERMISSIONS.contains(&p.as_str()))
        {
            return Err(invalid(format!("Unsupported permission: {name}")));
        }
        if !activity_id(&self.id)
            || self.name.trim().is_empty()
            || self.description.trim().is_empty()
            || self.name.len() > 240
            || self.description.len() > 4000
            || self.connects.len() > 64
            || self
                .connects
                .iter()
                .any(|c| !hostname(&c.host) || c.why.trim().is_empty() || c.why.len() > 1024)
            || self
                .authority
                .as_deref()
                .is_some_and(|a| !matches!(a, "shared" | "host"))
            || self.seats.is_some_and(|s| !(1..=128).contains(&s))
            || self.devices.as_ref().is_some_and(|d| {
                d.is_empty()
                    || d.iter()
                        .any(|s| !matches!(s.as_str(), "desktop" | "tablet" | "phone"))
            })
            || self.permissions.iter().collect::<HashSet<_>>().len() != self.permissions.len()
            || semver::Version::parse(&self.version).is_err()
        {
            return Err(invalid(
                "Invalid version, permissions, devices, authority, seats or external hosts in stellarr.json.",
            ));
        }
        Ok(())
    }
    fn supports(&self, device: &str) -> bool {
        matches!(device, "desktop" | "tablet" | "phone")
            && self
                .devices
                .as_ref()
                .is_none_or(|ds| ds.iter().any(|d| d == device))
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Installed {
    install: Id,
    source: String,
    enabled: bool,
    digest: String,
    revision: String,
    release: String,
    bytes: i64,
    manifest: Manifest,
    /// When the activity was first installed, and when its current version
    /// was; none for what was installed before either was kept.
    installed_at: Option<Timestamp>,
    updated_at: Option<Timestamp>,
    /// What the last look for a newer release than the current version found.
    check: Option<updates::Check>,
}

const LIBRARY: &str = "SELECT i.Id,i.Source,i.Enabled,v.Digest,v.Revision,v.Release,v.Bytes,v.Manifest,
    i.InstalledAt,v.InstalledAt,c.CheckedAt,c.Version,c.Release,c.Error
    FROM ActivityInstalls i JOIN ActivityVersions v ON v.Install=i.Id AND v.Digest=i.Digest
    LEFT JOIN ActivityUpdateChecks c ON c.Install=i.Id AND c.Digest=i.Digest";
fn installed(row: &rusqlite::Row<'_>) -> rusqlite::Result<Installed> {
    let text: String = row.get(7)?;
    let manifest = serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Installed {
        install: row.get(0)?,
        source: row.get(1)?,
        enabled: row.get(2)?,
        digest: row.get(3)?,
        revision: row.get(4)?,
        release: row.get(5)?,
        bytes: row.get(6)?,
        manifest,
        installed_at: row.get(8)?,
        updated_at: row.get(9)?,
        check: row
            .get::<_, Option<Timestamp>>(10)?
            .map(|checked_at| -> rusqlite::Result<_> {
                Ok(updates::Check {
                    checked_at,
                    version: row.get(11)?,
                    release: row.get(12)?,
                    error: row.get(13)?,
                })
            })
            .transpose()?,
    })
}
async fn library(state: &AppState) -> ApiResult<Vec<Installed>> {
    state
        .database()
        .read(|db| -> ApiResult<_> {
            Ok(db
                .prepare(&format!("{LIBRARY} ORDER BY i.Source,i.Activity"))?
                .query_map([], installed)?
                .collect::<Result<Vec<_>, _>>()?)
        })
        .await
}
async fn list(State(state): State<AppState>, _: InstanceOwner) -> ApiResult<Json<Value>> {
    let entries = library(&state)
        .await?
        .into_iter()
        .map(|entry| {
            let artwork = artwork::urls(
                &entry,
                &format!("/api/cockpit/activities/{}/art", entry.install),
            );
            let mut value = json!(entry);
            value["artwork"] = artwork;
            value
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "activities": entries,
        "hosting": state.settings().activity_domain.is_none() || live::config(&state).is_ok(),
        "checkHours": updates::INTERVAL.as_secs() / 3600,
    })))
}

#[derive(Deserialize)]
struct Lookup {
    url: String,
}
#[derive(Deserialize)]
struct Review {
    url: String,
    id: String,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    update: Option<Id>,
}

async fn discover(State(state): State<AppState>, request: Request) -> ApiResult<Json<Value>> {
    let input = bind_then_gate::<Lookup>(&state, request, false)
        .await?
        .body
        .ok_or_else(ApiError::bad_request)?;
    let _slot = DOWNLOADS
        .try_acquire()
        .map_err(|_| invalid("The installer is busy. Try again shortly."))?;
    Ok(Json(json!(releases::discover(&input.url, None).await?)))
}
async fn review(State(state): State<AppState>, request: Request) -> ApiResult<Json<Value>> {
    resolve(
        &state,
        bind_then_gate::<Review>(&state, request, false).await?.body,
        false,
    )
    .await
}
async fn install(State(state): State<AppState>, request: Request) -> ApiResult<Json<Value>> {
    resolve(
        &state,
        bind_then_gate::<Review>(&state, request, true).await?.body,
        true,
    )
    .await
}
async fn resolve(state: &AppState, input: Option<Review>, save: bool) -> ApiResult<Json<Value>> {
    let input = input.ok_or_else(ApiError::bad_request)?;
    let _slot = DOWNLOADS
        .try_acquire()
        .map_err(|_| invalid("The installer is busy. Try again shortly."))?;
    let selection = releases::discover(&input.url, Some(&input.id)).await?;
    let choice = selection.entries.first().ok_or_else(ApiError::not_found)?;
    let entry = &choice.entry;
    if let Some(expected) = input.update {
        let old = library(state)
            .await?
            .into_iter()
            .find(|v| {
                v.install == expected && v.source == selection.source && v.manifest.id == entry.id
            })
            .ok_or_else(|| invalid("This installation changed. Reload the library."))?;
        let old_version =
            semver::Version::parse(&old.manifest.version).map_err(ApiError::internal)?;
        let next_version = semver::Version::parse(&entry.version).map_err(ApiError::internal)?;
        if !next_version.cmp_precedence(&old_version).is_gt() {
            return Ok(Json(json!({"current":true})));
        }
    }
    let revision = releases::revision(&selection, choice).await?;
    if save
        && (input.digest.as_deref() != Some(entry.sha256.as_str())
            || input.revision.as_deref() != Some(revision.as_str()))
    {
        return Err(invalid(
            "The release changed. Review the new version before installation.",
        ));
    }
    let release = choice.release.clone();
    let package = releases::package(choice).await?;
    let previews = artwork::preview(&package.manifest, &package.files)?;
    let response = json!({"source":selection.source,"release":release,"revision":revision,
        "digest":entry.sha256,"bytes":entry.bytes,"manifest":package.manifest,"artwork":previews});
    if !save {
        return Ok(Json(response));
    }
    let entry = entry.clone();
    // A repository's link offers its newest stable release, so what it
    // installs is current as of now; a release's link may not be.
    let current = releases::names_repository(&input.url);
    let now = Timestamp::now();
    let _gate = state.activities().changes.write().await;
    state.database().write(move |tx| -> ApiResult<()> {
        let existing=tx.query_row(&format!("{LIBRARY} WHERE i.Source=?1 AND i.Activity=?2"),
            params![selection.source,entry.id],installed).optional()?;
        if input.update.is_some() && input.update != existing.as_ref().map(|v|v.install) {return Err(ApiError::conflict());}
        let id=if let Some(old)=existing {
            let current=semver::Version::parse(&old.manifest.version).map_err(ApiError::internal)?;
            let next=semver::Version::parse(&entry.version).map_err(ApiError::internal)?;
            if !next.cmp_precedence(&current).is_gt(){return Err(invalid("An update must have higher SemVer precedence than the installed version."));}
            tx.execute("UPDATE ActivityInstalls SET Digest=?1 WHERE Id=?2",params![entry.sha256,old.install])?;
            old.install
        } else {
            let id=Id::random();
            tx.execute("INSERT INTO ActivityInstalls(Id,Source,Activity,Digest,Enabled,InstalledAt) VALUES(?1,?2,?3,?4,1,?5)",params![id,selection.source,entry.id,entry.sha256,now])?;
            id
        };
        tx.execute("INSERT INTO ActivityVersions(Install,Digest,Revision,Release,Manifest,Bytes,InstalledAt) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![id,entry.sha256,revision,release,serde_json::to_string(&package.manifest).map_err(ApiError::internal)?,entry.bytes as i64,now])?;
        let mut put=tx.prepare_cached("INSERT INTO ActivityFiles(Install,Digest,Path,Content) VALUES(?1,?2,?3,?4)")?;
        for(path,bytes)in package.files{put.execute(params![id,entry.sha256,path,bytes])?;}
        if current {
            updates::keep_in(tx,id,&entry.sha256,&updates::Check::current(now),now.plus(updates::INTERVAL))?;
        }
        Ok(())
    }).await?;
    Ok(Json(response))
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Change {
    Enable,
    Disable,
    Uninstall,
}
#[derive(Deserialize)]
struct Set {
    action: Change,
}
async fn manage(
    State(state): State<AppState>,
    Path(id): Path<Id>,
    request: Request,
) -> ApiResult<Json<Value>> {
    let input = bind_then_gate::<Set>(&state, request, true)
        .await?
        .body
        .ok_or_else(ApiError::bad_request)?;
    let end = !matches!(input.action, Change::Enable);
    let _gate = state.activities().changes.write().await;
    state
        .database()
        .write(move |tx| -> ApiResult<()> {
            let count = match input.action {
                Change::Uninstall => {
                    tx.execute("DELETE FROM ActivityInstalls WHERE Id=?1", [id])?
                }
                Change::Enable | Change::Disable => tx.execute(
                    "UPDATE ActivityInstalls SET Enabled=?1 WHERE Id=?2",
                    params![!end, id],
                )?,
            };
            if count == 0 {
                return Err(ApiError::not_found());
            }
            Ok(())
        })
        .await?;
    if end {
        state.activities().remove(&state, id).await;
    }
    Ok(Json(json!({"ok":true})))
}
/// Looks for a newer release of one installation now, as the worker in
/// `updates` does by itself, and answers with what it found. GitHub's request
/// limit is a 429, so that Cockpit stops asking about the others.
async fn check(
    State(state): State<AppState>,
    Path(id): Path<Id>,
    request: Request,
) -> ApiResult<Json<Value>> {
    bind_then_gate::<Value>(&state, request, false).await?;
    let entry = library(&state)
        .await?
        .into_iter()
        .find(|entry| entry.install == id)
        .ok_or_else(ApiError::not_found)?;
    let _slot = DOWNLOADS
        .try_acquire()
        .map_err(|_| invalid("The installer is busy. Try again shortly."))?;
    let looked = updates::check(&state, &entry).await?;
    if looked.limited {
        let reason = looked.check.error.unwrap_or_default();
        return Err(Problem::new(StatusCode::TOO_MANY_REQUESTS)
            .detail(reason)
            .into());
    }
    Ok(Json(json!(looked.check)))
}
pub fn routes() -> Endpoints {
    Endpoints::new()
        .get(
            "/api/cockpit/activities/{id}/art/{digest}/{kind}",
            artwork::owner,
        )
        .authorized()
        .get(
            "/api/activities/{channel}/art/{id}/{digest}/{kind}",
            artwork::participant,
        )
        .authorized()
        .get("/api/cockpit/activities", list)
        .authorized()
        .post("/api/cockpit/activities/discover", discover)
        .json_body()
        .authorized()
        .post("/api/cockpit/activities/review", review)
        .json_body()
        .authorized()
        .post("/api/cockpit/activities/install", install)
        .json_body()
        .authorized()
        .post("/api/cockpit/activities/{id}/manage", manage)
        .json_body()
        .authorized()
        .post("/api/cockpit/activities/{id}/check", check)
        .json_body()
        .authorized()
        .post("/api/activities/{channel}/list", live::available)
        .json_body()
        .authorized()
        .post("/api/activities/{channel}/launch", live::launch)
        .json_body()
        .authorized()
        .post("/api/activities/{channel}/leave", live::leave)
        .json_body()
        .authorized()
        .post("/api/activities/{channel}/end", live::end)
        .json_body()
        .authorized()
        .get("/api/activities/live", live::running)
        .authorized()
        .post("/api/activities/{channel}/room", live::room)
        .json_body_up_to(384 * 1024)
        .authorized()
        .json_body_up_to(128 * 1024)
        .authorized()
        .get("/_activity/{asset_key}/{*path}", live::sandbox_asset)
        .get("/_stellarr/sdk/{name}", live::sandbox_sdk)
        .get("/_activity-files", live::asset_root)
        .get("/_activity-files/{*path}", live::asset)
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../../../sdk/test/fixtures/manifest-parity.json");

    #[test]
    fn production_manifest_matches_sdk_cases() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let base = fixture["base"].as_object().unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let mut fields = base.clone();
            fields.extend(case["fields"].as_object().unwrap().clone());
            let accepted = serde_json::from_value::<Manifest>(Value::Object(fields))
                .is_ok_and(|manifest| manifest.check().is_ok());
            assert_eq!(accepted, case["valid"].as_bool().unwrap(), "{}", case["label"]);
        }
    }

    #[test]
    fn production_raw_manifest_matches_sdk_duplicate_field_cases() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let base = fixture["base"].to_string();
        let prefix = base.strip_suffix('}').unwrap();
        for case in fixture["rawCases"].as_array().unwrap() {
            let members = case["members"].as_str().unwrap();
            let raw = format!("{prefix},{members}}}");
            let accepted = serde_json::from_str::<Manifest>(&raw)
                .is_ok_and(|manifest| manifest.check().is_ok());
            assert_eq!(accepted, case["valid"].as_bool().unwrap(), "{}", case["label"]);
        }
    }

    #[test]
    fn unsigned_manifest_fields_reject_decimal_and_exponential_tokens() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let base = fixture["base"].to_string();
        for protocol in ["1.0", "1e0", "1.00000000000000001"] {
            let manifest = base.replacen("\"protocol\":1", &format!("\"protocol\":{protocol}"), 1);
            assert!(serde_json::from_str::<Manifest>(&manifest).is_err(), "{protocol}");
        }
        for seats in ["2.0", "2e0", "2.00000000000000001"] {
            let manifest = base.replacen("\"protocol\":1", &format!("\"protocol\":1,\"seats\":{seats}"), 1);
            assert!(serde_json::from_str::<Manifest>(&manifest).is_err(), "{seats}");
        }
    }
}
