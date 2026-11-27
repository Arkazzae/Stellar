//! Messages an author schedules to send later, to a text channel or a
//! conversation. Nobody but the author learns that one exists: the routes
//! answer only for the author's own, and every event goes to the author's
//! connections alone.
//!
//! - `GET /api/scheduled` lists the caller's pending and failed messages, the
//!   soonest first; `?channelId=` or `?conversationId=` narrows it.
//! - `POST /api/channels/{channelId}/scheduled` and
//!   `POST /api/social/conversations/{conversationId}/scheduled` make one.
//! - `PATCH /api/scheduled/{id}` changes the text or the time of a pending
//!   one, `DELETE` removes it with the files nothing else holds, and
//!   `POST /api/scheduled/{id}/send` sends it now.
//!
//! A message is checked when it is made and when it is changed by the post
//! path itself, everything but slow mode, and again when its time comes,
//! since anything may have changed by then. The worker `scheduled-messages`
//! sends it through the same code as a post a person makes (for a channel,
//! the batched writer), and the row leaves the queue in the transaction that
//! stores the message, under the message's own id, so that no restart and no
//! second sender can post it twice. A refusal at that point marks it failed
//! with the reason; housekeeping removes a failed one after
//! [`FAILED_KEPT_DAYS`].

mod routes;
mod send;
pub mod store;
mod worker;

use std::time::Duration;

use serde::Serialize;

use crate::events::{Audience, Event};
use crate::http::Endpoints;
use crate::messages::Reason;
use crate::state::AppState;
use crate::values::Id;

pub use store::{Claim, ScheduledMessageDto};
pub use worker::{pass, workers};

/// How many messages an author may have waiting at once. Failed ones do not
/// count: they wait for the author, not for the worker.
pub const MAX_PENDING: i64 = 25;

/// The nearest a message may be scheduled. Anything sooner is a send.
pub const MIN_LEAD: Duration = Duration::from_secs(60);

/// The furthest ahead a message may be scheduled.
pub const MAX_LEAD_DAYS: u64 = 30;

/// How long a message that could not be sent is kept for its author to see.
pub const FAILED_KEPT_DAYS: u64 = 30;

/// Why a scheduled message could not be sent when its time came. The codes
/// are what the author's clients show a reason for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The author may no longer post there.
    NoAccess,
    /// The channel or the conversation is gone.
    TargetGone,
    /// The message it answers is gone.
    ThreadGone,
    /// The conversation was encrypted in the meantime.
    Encrypted,
    Other,
}

impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Failure::NoAccess => "no_access",
            Failure::TargetGone => "target_gone",
            Failure::ThreadGone => "thread_gone",
            Failure::Encrypted => "encrypted",
            Failure::Other => "other",
        }
    }

    pub fn parse(code: &str) -> Failure {
        match code {
            "no_access" => Failure::NoAccess,
            "target_gone" => Failure::TargetGone,
            "thread_gone" => Failure::ThreadGone,
            "encrypted" => Failure::Encrypted,
            _ => Failure::Other,
        }
    }

    /// The failure a refusal of the post path stands for.
    fn of(reason: Reason) -> Failure {
        match reason {
            Reason::Gone => Failure::TargetGone,
            Reason::NoAccess => Failure::NoAccess,
            Reason::ReplyGone => Failure::ThreadGone,
            Reason::Encrypted => Failure::Encrypted,
            Reason::Withdrawn => Failure::Other,
        }
    }
}

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/scheduled", routes::list)
        .authorized()
        .post("/api/channels/{channelId}/scheduled", routes::in_channel)
        .authorized()
        .json_body()
        .post(
            "/api/social/conversations/{conversationId}/scheduled",
            routes::in_conversation,
        )
        .authorized()
        .json_body()
        .patch("/api/scheduled/{id}", routes::edit)
        .authorized()
        .json_body()
        .delete("/api/scheduled/{id}", routes::delete)
        .authorized()
        .post("/api/scheduled/{id}/send", routes::send_now)
        .authorized()
}

/// The event that says a scheduled message is there as it now is: made,
/// changed, or failed.
const CHANGED: &str = "ScheduledMessageChanged";
/// The event that says one is gone: deleted, or sent.
const REMOVED: &str = "ScheduledMessageRemoved";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Removed {
    id: Id,
    /// True when it went out as a message, which has this same id.
    sent: bool,
}

fn announce_changed(state: &AppState, author: Id, message: &ScheduledMessageDto) {
    state
        .events()
        .publish(Event::new(Audience::Users(vec![author]), CHANGED, message));
}

fn announce_removed(state: &AppState, author: Id, id: Id, sent: bool) {
    state.events().publish(Event::new(
        Audience::Users(vec![author]),
        REMOVED,
        &Removed { id, sent },
    ));
}
