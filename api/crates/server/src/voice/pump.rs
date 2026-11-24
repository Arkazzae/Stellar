//! Follows the media server's events and hands them to the browsers whose
//! seats hold the sessions: a `signal` becomes `MediaSignal`, a `closed`
//! session `MediaSessionClosed`.
//!
//! The API starts with no seats, so the pump first resets the media server. It
//! then long-polls the events of that run of the media server, in order, and
//! remembers the last sequence number it handed out.
//!
//! A new epoch means the media server restarted and lost its sessions: the
//! sessions of every other epoch end, while one already opened in the new
//! epoch stays. When the media server no longer keeps the events the pump asks
//! for, nothing tells which sessions they concerned, so the pump resets the
//! media server again and ends them all. While the media server cannot be
//! reached the pump tries again after a pause, and treats the first answer
//! that follows as a possible new epoch.
//!
//! What the pump knows lives in the function that runs it, which is all it
//! needs: a run that starts again begins with a reset, as the API does.

use super::control::{Batch, MediaEvent, Unavailable};
use super::plane::{end_sessions, post_closed, post_signal};
use super::protocol::MEDIA_SERVER_SIGNAL_KINDS;
use crate::background::Stop;
use crate::realtime::Realtime;
use crate::state::AppState;

pub(super) async fn run(state: AppState, stop: Stop) {
    let Some(realtime) = state.realtime().cloned() else {
        return;
    };
    let pause = state.voice().control().timing().retry_pause;
    let mut epoch: Option<String> = None;
    let mut after: i64 = 0;
    let mut failing = false;
    while !stop.is_stopping() {
        let outcome = tokio::select! {
            outcome = step(&state, &realtime, &mut epoch, &mut after, failing) => outcome,
            () = stop.stopped() => return,
        };
        match outcome {
            Ok(()) => {
                if failing {
                    tracing::info!("The media server's control API answers again.");
                    failing = false;
                }
            }
            Err(error) => {
                // A refusal, a timeout and nonsense all get the same pause, so
                // a media server in a bad state is never polled in a tight
                // loop.
                if !failing {
                    tracing::warn!(
                        "The media server's control API is unavailable; voice waits until it answers: {error}"
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

/// One reset or one poll.
async fn step(
    state: &AppState,
    realtime: &Realtime,
    epoch: &mut Option<String>,
    after: &mut i64,
    after_outage: bool,
) -> Result<(), Unavailable> {
    let voice = state.voice();
    match epoch.clone() {
        None => {
            // Set before waiting for the gate, so no new session starts behind
            // the reset only to be lost in it.
            voice.plane().set_ready(false);
            let _turn = voice.plane().enter().await;
            let reset = voice.control().reset().await?;
            end_sessions(state, realtime, |_| true);
            voice.plane().set_ready(true);
            tracing::info!(
                "The media server was reset in epoch {} at sequence {}.",
                reset.epoch,
                reset.seq
            );
            *epoch = Some(reset.epoch);
            *after = reset.seq;
        }
        Some(current) => {
            let batch = voice.control().events(&current, *after).await?;
            let _turn = voice.plane().enter().await;
            let (next, last) = take(state, realtime, batch, &current, *after, after_outage);
            *epoch = next;
            *after = last;
        }
    }
    Ok(())
}

/// Handles one answer of the long poll and returns the epoch and sequence
/// number to poll from next; no epoch asks for a reset. Runs under the gate.
fn take(
    state: &AppState,
    realtime: &Realtime,
    batch: Batch,
    epoch: &str,
    mut after: i64,
    after_outage: bool,
) -> (Option<String>, i64) {
    if batch.reset && batch.epoch == epoch {
        state.voice().plane().set_ready(false);
        tracing::warn!(
            "The media server no longer keeps the events after sequence {after}; it is reset and every media session ends."
        );
        return (None, 0);
    }

    let restarted = batch.reset || batch.epoch != epoch;
    if restarted || after_outage {
        end_sessions(state, realtime, |session| session.epoch != batch.epoch);
    }

    if restarted {
        // A reset answer carries no events; an answer from another run without
        // the flag is read the same way rather than trusted.
        tracing::warn!(
            "The media server runs a new epoch {}; the media sessions of earlier epochs ended.",
            batch.epoch
        );
        return (Some(batch.epoch), 0);
    }

    for event in &batch.events {
        if event.seq <= after {
            continue;
        }
        deliver(state, realtime, event);
        after = event.seq;
    }
    (Some(epoch.to_string()), after)
}

fn deliver(state: &AppState, realtime: &Realtime, event: &MediaEvent) {
    let presence = realtime.presence();
    match event.kind_of_event.as_str() {
        "signal" => {
            let (Some(kind), Some(payload)) = (event.kind.as_deref(), event.payload_object())
            else {
                ignored(event);
                return;
            };
            if !MEDIA_SERVER_SIGNAL_KINDS.contains(&kind) {
                ignored(event);
                return;
            }
            // A session that no seat holds any more has nobody to tell.
            if let Some(seated) = presence.find_media_session(&event.session_id) {
                post_signal(state, &seated, kind, payload);
            }
        }
        "closed" => {
            // Only the path that takes the session off its seat tells the
            // browser, so a session that the hub already found lost, or whose
            // seat ended, is not reported twice.
            if let Some(ended) = presence.forget_media_session(&event.session_id) {
                let reason = if event.reason.as_deref() == Some("transport-timeout") {
                    "transport-timeout"
                } else {
                    "error"
                };
                post_closed(state, &ended, reason);
            }
        }
        _ => ignored(event),
    }
}

fn ignored(event: &MediaEvent) {
    tracing::debug!(
        "Media event {} of type {} and kind {:?} was ignored.",
        event.seq,
        event.kind_of_event,
        event.kind
    );
}
