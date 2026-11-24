//! What a client loads when it starts: the person, the server they are on and
//! the ones they belong to, what they see of its channels and sections, its
//! members, the newest messages of the first text channel, where they have read
//! up to, who sits in the voice rooms, how to take part in voice and what they
//! may do. And the one switch a client reads at run time: whether it offers
//! GIFs, with the key when it does.
//!
//! `GET /api/bootstrap` and `GET /api/config/giphy`. Everything is read in one
//! snapshot ([`snapshot`]). Page markers are written afterwards in a separate
//! short transaction, only for visible text channels the person has never opened.

mod giphy;
mod snapshot;
mod voice;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;

pub use giphy::{Giphy, resolve as resolve_giphy};
pub use snapshot::{Context, Read, read};

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiResult, Endpoints, json_response};
use crate::permissions::membership::ActiveServer;
use crate::state::AppState;
use crate::values::Timestamp;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/bootstrap", bootstrap)
        .authorized()
        .get("/api/config/giphy", giphy::config)
        .authorized()
}

async fn bootstrap(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let (user, messages) = (caller.user, *state.messages());
    let shared = state.clone();
    let mut read = state
        .database()
        .read(move |db| {
            let context = Context {
                messages: &messages,
                presence: shared.presence(),
                seats: shared.realtime().map(|realtime| realtime.presence()),
            };
            read(db, &context, user, server)
        })
        .await?;

    // Markers for the channels the person has never opened, as of now, so that
    // a new member does not start with the whole history unread. Another tab
    // may store some first: the stored ones stand, and what is answered is
    // what is stored.
    if !read.missing.is_empty() {
        let missing = std::mem::take(&mut read.missing);
        let now = Timestamp::now();
        let stored = state
            .database()
            .write(move |tx| -> Result<Vec<(crate::values::Id, Timestamp)>, DbError> {
                snapshot::store_markers(tx, user, &missing, now)
            })
            .await?;
        read.add_markers(stored);
    }

    let answer = read.finish(&state, Timestamp::now())?;
    Ok(json_response(StatusCode::OK, &answer))
}

