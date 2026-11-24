//! Whether the API is up. The container's process manager asks `/health/ready`
//! every ten seconds, so these routes write nothing to the log.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::database::DbError;
use crate::http::{Endpoints, Json, Problem};
use crate::state::AppState;

pub fn routes() -> Endpoints {
    Endpoints::new().get("/health/live", live).get("/health/ready", ready)
}

#[derive(Serialize)]
struct Status {
    status: &'static str,
}

async fn live() -> Json<Status> {
    Json(Status { status: "live" })
}

/// Ready once the database answers.
async fn ready(State(state): State<AppState>) -> Response {
    let answered = state
        .database()
        .read(|connection| -> Result<i64, DbError> { Ok(connection.query_row("SELECT 1", [], |row| row.get(0))?) })
        .await;
    match answered {
        Ok(_) => Json(Status { status: "ready" }).into_response(),
        Err(_) => Problem::new(StatusCode::SERVICE_UNAVAILABLE).title("Database unavailable").into_response(),
    }
}
