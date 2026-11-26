//! The API's side of the media plane: whether it is in step with the media
//! server, the one gate that keeps the creation of a session apart from the
//! moments in which sessions are forgotten or events handed out, and what the
//! plane tells a browser.
//!
//! The gate closes two gaps. An event of a session that the media server has
//! just created may come back on the long poll before the hub has given the
//! session to its seat; under the gate the pump hands it out only once the
//! seat holds it. And `POST /v1/reset` ends every session, including one
//! created a moment before, which the epoch of a session cannot reveal because
//! a reset keeps the epoch; under the gate no session is being created while a
//! reset runs. Signals and closes do not take the gate.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tokio::sync::{Mutex, MutexGuard};

use super::protocol::VERSION;
use crate::events::{Audience, Event};
use crate::realtime::Realtime;
use crate::realtime::presence::{MediaSession, SeatedMediaSession};
use crate::state::AppState;
use crate::values::Id;

/// What a browser is told when the media server lost or ended its session
/// without the browser having asked.
pub const MEDIA_RESTARTED: &str = "media-restarted";

/// How an attempt to open a media session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opening {
    Opened(String),
    /// The seat ended while the media server was creating the session.
    NotSeated,
    /// The media server could not be reached, or the API is putting it back in
    /// step.
    Unavailable,
}

/// Whether the API is in step with the media server, and its gate.
#[derive(Default)]
pub struct Plane {
    ready: AtomicBool,
    gate: Mutex<()>,
}

impl Plane {
    /// True once the API has reset the media server and until it finds that it
    /// lost track of the media server's events. Sessions are only created in
    /// between.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    pub(super) fn set_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::SeqCst);
    }

    /// Waits for the gate, for the event pump.
    pub(super) async fn enter(&self) -> MutexGuard<'_, ()> {
        self.gate.lock().await
    }
}

/// Creates the media session of the connection's seat in `room`, after
/// closing, silently, the session the connection held before.
///
/// The work runs as a task of its own: a call that is dropped half way, by a
/// connection that went away, must still attach the session it asked the media
/// server for or close it, or the media server would keep a session no seat
/// holds.
pub async fn open(state: &AppState, connection: &str, room: Id, user: Id) -> Opening {
    if let Some(rt) = state.realtime() {
        rt.presence().close_media_session(connection);
    }
    open_for(state, connection, room, user, None).await
}

/// Opens a session only while the activity still holds this specific grant.
pub async fn open_camera(
    state: &AppState,
    connection: &str,
    room: Id,
    user: Id,
    grant: Id,
) -> Opening {
    open_for(state, connection, room, user, Some(grant)).await
}

async fn open_for(
    state: &AppState,
    connection: &str,
    room: Id,
    user: Id,
    grant: Option<Id>,
) -> Opening {
    let Some(realtime) = state.realtime().cloned() else {
        return Opening::Unavailable;
    };
    let state = state.clone();
    let connection = connection.to_string();
    tokio::spawn(async move { create(&state, &realtime, &connection, room, user, grant).await })
        .await
        .unwrap_or(Opening::Unavailable)
}

async fn create(
    state: &AppState,
    realtime: &Realtime,
    connection: &str,
    room: Id,
    user: Id,
    grant: Option<Id>,
) -> Opening {
    let presence = realtime.presence();

    // A hub call must not hang on a pump that is stuck behind a slow media
    // server, so the wait is as long as a command may take.
    let voice = state.voice();
    let limit = voice.control().timing().command;
    let Ok(_turn) = tokio::time::timeout(limit, voice.plane().gate.lock()).await else {
        return Opening::Unavailable;
    };
    if !voice.plane().is_ready() {
        return Opening::Unavailable;
    }

    // A revoked attempt that waited in the queue must not cause the SFU
    // to replace a newer camera session for the same player.
    if grant.is_some_and(|grant| !presence.has_camera_grant(connection, room, grant)) {
        return Opening::NotSeated;
    }
    let created = match voice
        .control()
        .create_session(room, user, grant.is_some())
        .await
    {
        Ok(created) => created,
        Err(error) => {
            tracing::warn!("Opening a media session failed: {error}");
            return Opening::Unavailable;
        }
    };
    let session = MediaSession {
        session_id: created.session_id,
        epoch: created.epoch,
    };
    let attached = match grant {
        Some(grant) => presence.attach_camera(connection, room, grant, session.clone()),
        None => presence.attach_media_session(connection, room, session.clone()),
    };
    if attached {
        return Opening::Opened(session.session_id);
    }
    presence.close_session(session);
    Opening::NotSeated
}

/// Forgets a session the media server no longer has, as a `404
/// unknown-session` on a signal shows, and tells its browser.
pub fn lose_session(state: &AppState, realtime: &Realtime, session_id: &str) {
    if let Some(lost) = realtime.presence().forget_media_session(session_id) {
        post_closed(state, &lost, MEDIA_RESTARTED);
    }
}

/// Forgets every session `ended` picks, which the media server lost when it
/// restarted or was reset, and tells their browsers.
pub(super) fn end_sessions(
    state: &AppState,
    realtime: &Realtime,
    ended: impl Fn(&MediaSession) -> bool,
) {
    for session in realtime.presence().forget_media_sessions(ended) {
        post_closed(state, &session, MEDIA_RESTARTED);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaSignal<'a> {
    protocol_version: i64,
    channel_id: Id,
    session_id: &'a str,
    kind: &'a str,
    /// The media server's document, as text.
    payload: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaSessionClosed<'a> {
    channel_id: Id,
    session_id: &'a str,
    reason: &'a str,
}

/// One signal of the media server for the browser whose seat holds the
/// session. Delivery never waits, and one browser's signals arrive in the
/// order they were posted, which the negotiation of a session depends on.
pub(super) fn post_signal(state: &AppState, to: &SeatedMediaSession, kind: &str, payload: &str) {
    let signal = MediaSignal {
        protocol_version: VERSION,
        channel_id: to.channel,
        session_id: &to.session.session_id,
        kind,
        payload,
    };
    state.events().publish(Event::new(
        Audience::Connection(to.connection_id.clone()),
        "MediaSignal",
        &signal,
    ));
}

/// The media plane ended a session that its seat still needed.
pub(super) fn post_closed(state: &AppState, to: &SeatedMediaSession, reason: &str) {
    let closed = MediaSessionClosed {
        channel_id: to.channel,
        session_id: &to.session.session_id,
        reason,
    };
    state.events().publish(Event::new(
        Audience::Connection(to.connection_id.clone()),
        "MediaSessionClosed",
        &closed,
    ));
}
