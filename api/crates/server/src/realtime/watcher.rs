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
use super::registry::LiveConnection;
use crate::auth::session::{self, Authentication, Ticket};
use crate::background::Worker;
use crate::database::DbError;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

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
    let rejected = rejected_sessions(state, &connections).await?;
    let mut closed = 0;
    for connection in connections {
        if rejected.contains(&(connection.user, connection.session, connection.version)) {
            realtime.end_connection(&connection.connection_id);
            closed += 1;
        }
    }
    Ok(closed)
}

/// The sessions, each with the account and version a connection was let in
/// with, that the database no longer accepts. One read for all of them, and
/// one check for each, however many connections share it.
async fn rejected_sessions(
    state: &AppState,
    connections: &[LiveConnection],
) -> Result<HashSet<(Id, Id, i64)>, DbError> {
    let sessions: HashSet<(Id, Id, i64)> = connections
        .iter()
        .map(|connection| (connection.user, connection.session, connection.version))
        .collect();
    let checked = state.clone();
    state
        .database()
        .read(move |db| -> Result<HashSet<(Id, Id, i64)>, DbError> {
            let now = Timestamp::now();
            let mut rejected = HashSet::new();
            for (user, session, version) in sessions {
                let ticket = Ticket::of_connection(user, version, session);
                // What the check would write back (the time the session was
                // seen, a cookie to renew) is left alone: this is a look.
                let outcome = session::authenticate(
                    db,
                    checked.sealer(),
                    &checked.settings().environment,
                    false,
                    ticket,
                    now,
                )?;
                if !matches!(outcome, Authentication::SignedIn { .. }) {
                    rejected.insert((user, session, version));
                }
            }
            Ok(rejected)
        })
        .await
}
