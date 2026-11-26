//! Closes the connections of sessions that no longer count.
//!
//! A session can end in the database without this process being told: an
//! operator's command line revokes it, another process changes the account.
//! Once a second the watcher checks each session that has a connection open,
//! with the same check a request's cookie goes through, and closes the
//! connections of those that are revoked, expired, past their absolute end,
//! of a disabled account, of an account whose sessions were bumped, or
//! without the second step the instance now asks for.

use std::collections::HashSet;
use std::time::Duration;

use super::Realtime;
use crate::auth::session;
use crate::background::Worker;
use crate::database::DbError;
use crate::state::AppState;
use crate::values::Timestamp;

/// How often the sessions are looked at.
const INTERVAL: Duration = Duration::from_secs(1);

pub fn workers() -> Vec<Worker> {
    vec![Worker::new("realtime-sessions", |state, stop| async move {
        while stop.sleep(INTERVAL).await {
            let Some(realtime) = state.realtime() else {
                return;
            };
            // A pass that fails leaves every connection as it was and is
            // tried again a second later.
            if let Err(error) = sweep(&state, realtime).await {
                tracing::warn!(
                    target: "Stellarr.Api.Realtime",
                    "Checking revoked realtime sessions failed: {error}"
                );
            }
        }
    })]
}

/// One pass: closes what must be closed and says how many connections that
/// was.
pub async fn sweep(state: &AppState, realtime: &Realtime) -> Result<usize, DbError> {
    let connections = realtime.registry().snapshot();
    if connections.is_empty() {
        return Ok(0);
    }
    let ids: HashSet<_> = connections.iter().map(|entry| entry.session).collect();
    let ids: Vec<_> = ids.into_iter().collect();
    let active = state
        .database()
        .read(move |db| session::active_sessions(db, &ids, Timestamp::now()))
        .await?;
    let mut closed = 0;
    for connection in connections {
        if !active.contains(&(connection.user, connection.session, connection.version)) {
            realtime.end_connection(&connection.connection_id);
            closed += 1;
        }
    }
    Ok(closed)
}
