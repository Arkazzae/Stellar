//! The desktop versions the instance asks for, and the packages it offers.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use stellarr_messages::{MessageKey, Messages};

use super::settings::Row;
use super::bind;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Problem, json_response};
use crate::panel::{self, operations};
use crate::permissions::membership::InstanceOwner;
use crate::releases::{self, ANDROID, DESKTOP, ReleaseDto};
use crate::state::AppState;
use crate::values::Timestamp;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Releases {
    minimum_version: String,
    recommended_version: String,
    published: Option<ReleaseDto>,
    android_published: Option<ReleaseDto>,
}

/// The versions in force, with the packages published for download. A broken
/// release manifest turns into a 503 that says what to repair.
async fn describe(
    state: &AppState,
    minimum: Option<String>,
    recommended: Option<String>,
    broken: &str,
) -> ApiResult<Response> {
    let settings = state.settings();
    let (minimum, recommended) = (
        releases::desktop_version(minimum.as_deref(), settings.desktop.minimum_version.as_deref()),
        releases::desktop_version(
            recommended.as_deref(),
            settings.desktop.recommended_version.as_deref(),
        ),
    );
    let root = settings.data_root.clone();
    let found = tokio::task::spawn_blocking(move || {
        Ok::<_, releases::Failure>((
            releases::current(&DESKTOP, &root)?,
            releases::current(&ANDROID, &root)?,
        ))
    })
    .await
    .map_err(ApiError::internal)?;
    match found {
        Ok((desktop, android)) => Ok(json_response(
            StatusCode::OK,
            &Releases {
                minimum_version: minimum,
                recommended_version: recommended,
                published: desktop.map(|release| release.dto()),
                android_published: android.map(|release| release.dto()),
            },
        )),
        Err(releases::Failure::Untrusted) => {
            Err(Problem::new(StatusCode::SERVICE_UNAVAILABLE)
                .title(broken)
                .into())
        }
        Err(releases::Failure::Disk(error)) => Err(ApiError::internal(error)),
    }
}

pub(super) async fn read(
    State(state): State<AppState>,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Response> {
    let reading = state.clone();
    let row = state
        .database()
        .read(move |db| -> Result<Row, DbError> { Ok(Row::load(db, &reading, server)?) })
        .await?;
    let broken = state.messages().get(MessageKey::CockpitReleaseManifestBroken, &[])?;
    describe(&state, row.desktop_minimum, row.desktop_recommended, &broken).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Versions {
    minimum_version: Option<String>,
    recommended_version: Option<String>,
}

fn problems(
    versions: &Versions,
    messages: &Messages,
) -> Result<Vec<(String, Vec<String>)>, ApiError> {
    let minimum = versions.minimum_version.as_deref().unwrap_or("").trim();
    let recommended = versions.recommended_version.as_deref().unwrap_or("").trim();
    let mut problems = Vec::new();
    let low = releases::parse_stable(minimum);
    let high = releases::parse_stable(recommended);
    if low.is_none() {
        problems.push(("minimumVersion".to_string(), vec![messages.get(MessageKey::CockpitVersionInvalid, &[])?]));
    }
    if high.is_none() {
        problems.push(("recommendedVersion".to_string(), vec![messages.get(MessageKey::CockpitVersionInvalid, &[])?]));
    }
    if let (Some(low), Some(high)) = (low, high)
        && low > high
    {
        problems.push((
            "recommendedVersion".to_string(),
            vec![messages.get(MessageKey::CockpitRecommendedBelowMinimum, &[])?],
        ));
    }
    Ok(problems)
}

pub(super) async fn update(State(state): State<AppState>, request: Request) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, bind::<Versions>).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    let messages = *state.messages();
    let found = problems(&body, &messages)?;
    if !found.is_empty() {
        return Err(Problem::validation(found).into());
    }
    let minimum = body.minimum_version.as_deref().unwrap_or("").trim().to_string();
    let recommended = body.recommended_version.as_deref().unwrap_or("").trim().to_string();
    let writing = state.clone();
    let (saved_minimum, saved_recommended) = (minimum.clone(), recommended.clone());
    state
        .database()
        .write(move |tx| -> Result<(), ApiError> {
            let created: Timestamp = tx.query_row(
                "SELECT \"CreatedAt\" FROM \"Servers\" WHERE \"Id\" = ?1",
                [server],
                |row| row.get(0),
            )?;
            operations::ensure(tx, writing.settings(), server, created)?;
            tx.execute(
                "UPDATE \"OperationsSettings\" SET \"DesktopMinimumVersion\" = ?2, \"DesktopRecommendedVersion\" = ?3, \"UpdatedAt\" = ?4 WHERE \"ServerId\" = ?1",
                params![server, saved_minimum, saved_recommended, Timestamp::now()],
            )?;
            panel::record(
                tx,
                server,
                caller.user,
                messages.get(
                    MessageKey::AuditDesktopVersionsSet,
                    &[("minimum", &saved_minimum), ("recommended", &saved_recommended)],
                )?,
            )?;
            Ok(())
        })
        .await?;
    let broken = messages.get(MessageKey::CockpitVersionsSavedManifestBroken, &[])?;
    describe(&state, Some(minimum), Some(recommended), &broken).await
}
