//! Whether the web client offers GIFs, with the key when it does.
//!
//! The browser's key is a public web-integration credential. Serving it at run
//! time lets the operator set it and turn GIFs on or off in the panel, with no
//! rebuild of the web client and no restart.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::database::DbError;
use crate::http::{ApiResult, json_response};
use crate::secrets::{self, ResolvedSecrets};
use crate::state::AppState;

/// The answer: GIFs on or off, and the key only while they are on.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Giphy {
    pub enabled: bool,
    pub api_key: String,
}

impl std::fmt::Debug for Giphy {
    // The key is never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Giphy {{ enabled: {} }}", self.enabled)
    }
}

/// GIFs are on when a key is set and the operator has not turned them off. An
/// installation that never chose (`None`) keeps them on while it has a key. Off,
/// the key is not handed out.
pub fn resolve(chosen: Option<bool>, secrets: &ResolvedSecrets) -> Giphy {
    let key = secrets.get(&secrets::GIPHY_API_KEY);
    let enabled = key.is_set() && chosen.unwrap_or(true);
    Giphy {
        enabled,
        api_key: if enabled { key.expose().to_string() } else { String::new() },
    }
}

/// What the operator chose for the instance's own server.
pub fn chosen(connection: &Connection) -> rusqlite::Result<Option<bool>> {
    let Some(instance) = crate::permissions::membership::instance_server(connection)? else {
        return Ok(None);
    };
    Ok(connection
        .query_row(
            "SELECT \"GiphyEnabled\" FROM \"OperationsSettings\" WHERE \"ServerId\" = ?1",
            [instance],
            |row| row.get::<_, Option<bool>>(0),
        )
        .optional()?
        .flatten())
}

pub async fn config(State(state): State<AppState>) -> ApiResult<Response> {
    let reading = state.clone();
    let answer = state
        .database()
        .read(move |db| -> Result<Giphy, DbError> {
            let secrets = secrets::read(db, &reading.settings().secrets, reading.sealer())?;
            Ok(resolve(chosen(db)?, &secrets))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}
