//! The voice parts of the bootstrap: who sits in which room, and how a client
//! takes part.
//!
//! Voice rooms are another group's. Until they are on this branch nobody sits
//! anywhere, which is what the first server answers for an instance where no
//! room is in use, and the relay credentials and the media endpoint are written
//! here the way the voice group writes them. When voice lands, the three
//! calls below are the ones to switch, and the rest of this file goes:
//!
//! - [`seats`] to `PresenceTracker::voice_users(channel)`;
//! - [`credentials`] to `voice::turn::credentials(&settings.rtc, user, now)`;
//! - [`media_endpoint`] to `voice::turn::public_endpoint(&settings.rtc, host)`.

use aws_lc_rs::hmac;
use serde::Serialize;

use crate::encoding::base64;
use crate::settings::Settings;
use crate::settings::relay::Relay;
use crate::values::{Id, Timestamp};

/// Somebody sitting in a voice room.
pub struct Seat {
    pub user: Id,
    pub muted: bool,
}

/// Who sits in a room. Nobody does, until voice is here.
pub fn seats(_channel: Id) -> Vec<Seat> {
    Vec::new()
}

/// What a browser is given to use the relay: valid for an hour.
struct Credentials {
    username: String,
    credential: String,
}

/// The secret that is used when none is set, which only development and tests
/// can be without: a start elsewhere is refused without one.
const DEVELOPMENT_SECRET: &str = "stellarr-turn-development-secret";

fn credentials(settings: &Settings, user: Id, now: Timestamp) -> Credentials {
    let secret = settings
        .rtc
        .turn_shared_secret
        .as_deref()
        .filter(|secret| !secret.trim().is_empty())
        .unwrap_or(DEVELOPMENT_SECRET);
    let ends = now.plus(std::time::Duration::from_secs(60 * 60));
    let username = format!(
        "{}:{}",
        ends.unix_seconds(),
        user.to_string().replace('-', "")
    );
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret.as_bytes());
    let mac = hmac::sign(&key, username.as_bytes());
    Credentials {
        credential: base64(mac.as_ref()),
        username,
    }
}

/// The media server's public endpoint, which a browser does not connect to:
/// it uses the candidates of the session description.
fn media_endpoint(settings: &Settings, host: &str) -> String {
    format!("udp://{host}:{}", settings.rtc.media.media_port)
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
/// the relay stays in the ICE servers for browsers on networks that block UDP.
/// A relay that is set up wrongly is an error here, at the time of asking.
pub fn rtc(settings: &Settings, user: Id, now: Timestamp) -> Result<Rtc, String> {
    let relay: Relay = settings.relay.arrangement.clone()?;
    let host = settings.rtc.public_host.as_str();
    let turn = credentials(settings, user, now);
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
    Ok(Rtc {
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
        media_server: media_endpoint(settings, host),
    })
}
