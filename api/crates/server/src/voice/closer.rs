//! Closes, in the media server, the sessions whose seats ended. A seat ends at
//! once and in memory (see the presence tracker); the request to the media
//! server follows from here, so neither a database lookup nor a media server
//! that is down can keep a person seated.
//!
//! A close that fails is tried again after a pause until the media server
//! takes it. Giving up would leave a person who was kicked receiving media,
//! and closing a session the media server no longer has is harmless: it
//! answers 204 for an unknown session too. The queue is the tracker's, so it
//! outlives a restart of this worker.

use crate::background::Stop;
use crate::state::AppState;

pub(super) async fn run(state: AppState, stop: Stop) {
    let Some(realtime) = state.realtime().cloned() else {
        return;
    };
    let presence = realtime.presence();
    let voice = state.voice();
    let pause = voice.control().timing().retry_pause;
    let mut failing = false;
    loop {
        let Some(session) = presence.next_close() else {
            tokio::select! {
                () = presence.closes_queued() => continue,
                () = stop.stopped() => return,
            }
        };
        let closed = tokio::select! {
            closed = voice.control().close_session(&session.session_id) => closed,
            () = stop.stopped() => return,
        };
        match closed {
            Ok(()) => {
                if failing {
                    tracing::info!("Closing media sessions works again.");
                    failing = false;
                }
                presence.finish_close(&session);
            }
            Err(error) => {
                // Whatever went wrong, the loop has to survive it.
                if !failing {
                    tracing::warn!(
                        "Closing media session {} failed; it stays queued and is tried again: {error}",
                        session.session_id
                    );
                    failing = true;
                }
                if !stop.sleep(pause).await {
                    return;
                }
            }
        }
    }
}
