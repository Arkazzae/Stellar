//! Voice rooms and the media sessions behind them.
//!
//! A room is either a voice channel of a server or a direct conversation
//! between two people, and works the same way in both: a connection takes a
//! seat, the browsers in the room tell each other their state through the hub,
//! and their media goes through the media server, which takes commands from
//! this API alone.
//!
//! - [`control`] is the API's client of the media server's control API.
//! - [`plane`] is whether the API is in step with the media server, and what it
//!   tells a browser; [`pump`] follows the media server's events and
//!   [`closer`] closes the sessions of seats that ended. Both run as workers
//!   (`voice-media-events` and `voice-media-closer`).
//! - The seats themselves are the presence tracker's.
//! - The hub methods, and what a connection that starts or ends does to the
//!   seats, are in [`hub`].
//! - What other groups ask of voice is below: [`Voice`] for the media plane's
//!   state, [`rooms::broadcast_snapshot`] to tell a room who is in it,
//!   [`turn::public_endpoint`] and [`turn::credentials`] for the bootstrap.

pub mod closer;
pub mod control;
mod document;
pub mod gate;
pub mod hub;
pub mod json;
mod media;
mod moderation;
pub mod plane;
pub mod protocol;
pub mod pump;
pub mod rooms;
pub mod sounds;
pub mod turn;

use crate::background::Worker;
use crate::settings::Settings;

pub use self::rooms::{announce_rooms, end_direct_call, leave_server_rooms};
use self::control::Control;
use self::gate::PlaybackGate;
use self::plane::Plane;

/// What the media plane keeps in memory: the client of the control API, the
/// gate and readiness of the plane, and the cooldowns of the soundboard. The
/// seats and the queue of sessions to close are the presence tracker's.
pub struct Voice {
    control: Control,
    plane: Plane,
    playback: PlaybackGate,
}

impl Voice {
    pub fn new(settings: &Settings) -> Voice {
        Voice {
            control: Control::new(&settings.rtc.media),
            plane: Plane::default(),
            playback: PlaybackGate::default(),
        }
    }

    pub fn control(&self) -> &Control {
        &self.control
    }

    pub fn plane(&self) -> &Plane {
        &self.plane
    }

    pub fn playback(&self) -> &PlaybackGate {
        &self.playback
    }
}

/// The workers that keep the API and the media server in step.
pub fn workers() -> Vec<Worker> {
    vec![
        Worker::new("voice-media-events", pump::run),
        Worker::new("voice-media-closer", closer::run),
    ]
}

/// The routes of voice: removing a member from a voice channel and moving them
/// to another, which are the moderator's.
pub fn routes() -> crate::http::Endpoints {
    crate::http::Endpoints::new()
        .delete(
            "/api/admin/channels/{channelId}/members/{userId}",
            moderation::kick_from_voice,
        )
        .authorized()
        .patch(
            "/api/admin/channels/{channelId}/members/{userId}",
            moderation::move_between_voice_channels,
        )
        .authorized()
        .json_body()
}
