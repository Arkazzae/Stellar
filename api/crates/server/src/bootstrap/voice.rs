//! The voice parts of the bootstrap: who sits in which room, and how a client
//! takes part. The seats come from the presence tracker the hub keeps, the
//! relay credential and the media endpoint from the voice group's own.

use serde::Serialize;

use crate::realtime::presence::PresenceTracker;
use crate::settings::Settings;
use crate::settings::relay::Relay;
use crate::values::{Id, Timestamp};
use crate::voice::turn;

/// Somebody sitting in a voice room.
pub struct Seat {
    pub user: Id,
    pub muted: bool,
}

/// Who sits in a room. Without a tracker (a server started without the hub)
/// nobody does.
pub fn seats(tracker: Option<&PresenceTracker>, channel: Id) -> Vec<Seat> {
    tracker
        .map(|tracker| {
            tracker
                .voice_users(channel)
                .into_iter()
                .map(|seat| Seat { user: seat.user, muted: seat.muted })
                .collect()
        })
        .unwrap_or_default()
}

/// The version of the signalling contract.
const PROTOCOL_VERSION: i32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rtc {
    protocol_version: i32,
    capabilities: Capabilities,
    ice_servers: Vec<IceServer>,
    signaling_hub: &'static str,
    media_server: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    audio_codecs: [&'static str; 1],
    video_codecs: [&'static str; 3],
    audio_processing: [&'static str; 5],
    camera: bool,
    screen_share: bool,
    screen_audio: bool,
}

#[derive(Serialize)]
struct IceServer {
    urls: Vec<String>,
    username: Option<String>,
    credential: Option<String>,
}

/// How a client takes part in voice. Every room goes through the media server;
/// ICE servers are supplied only when the public TURN service is enabled.
pub fn rtc(settings: &Settings, user: Id, now: Timestamp) -> Rtc {
    let relay: Relay = settings.relay.arrangement;
    let host = settings.rtc.public_host.as_str();
    let ice_servers = if relay.enabled {
        relay_servers(settings, relay, host, user, now)
    } else {
        Vec::new()
    };
    Rtc {
        protocol_version: PROTOCOL_VERSION,
        capabilities: Capabilities {
            audio_codecs: ["opus"],
            video_codecs: ["vp9", "vp8", "h264"],
            audio_processing: ["browser", "rnnoise", "gtcrn", "speex", "gate"],
            camera: true,
            screen_share: true,
            screen_audio: true,
        },
        ice_servers,
        signaling_hub: "/hubs/app",
        media_server: turn::public_endpoint(&settings.rtc, host),
    }
}

fn relay_servers(
    settings: &Settings,
    relay: Relay,
    host: &str,
    user: Id,
    now: Timestamp,
) -> Vec<IceServer> {
    let turn = turn::credentials(&settings.rtc, user, now);
    let port = relay.turn_port;
    let mut turn_urls = vec![format!("turn:{host}:{port}?transport=udp")];
    if relay.tcp_enabled {
        turn_urls.push(format!("turn:{host}:{port}?transport=tcp"));
    }
    let mut ice_servers = vec![
        IceServer {
            urls: vec![format!("stun:{host}:{port}")],
            username: None,
            credential: None,
        },
        IceServer {
            urls: turn_urls,
            username: Some(turn.username.clone()),
            credential: Some(turn.credential.clone()),
        },
    ];
    if relay.turns_enabled {
        ice_servers.push(IceServer {
            urls: vec![format!("turns:{host}:{}?transport=tcp", relay.turns_port)],
            username: Some(turn.username),
            credential: Some(turn.credential),
        });
    }
    ice_servers
}
