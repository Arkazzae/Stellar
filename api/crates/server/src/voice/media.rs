//! The hub leg of the media server, which every voice room goes through: a
//! browser opens a media session for its seat and exchanges signals with the
//! media server through the hub, which decides who may do so. The media server
//! trusts the hub and nobody else.

use serde_json::{Value, json};
use stellarr_hub::HubError;

use super::control::{SignalOutcome, Unavailable};
use super::document::{self, Members, ParsedPayload};
use super::hub::{parse_id, visible};
use super::plane::{self, Opening};
use super::protocol::{
    MAX_SIGNAL_BYTES, MAX_SUBSCRIPTION_MID_LENGTH, MAX_SUBSCRIPTION_MIDS, MEDIA_SIGNAL_KINDS,
    VERSION,
};
use crate::realtime::method::{Context, arguments};
use crate::realtime::throttle::Policy;

const MEDIA_UNAVAILABLE: &str = "Media server is unavailable.";
const INVALID_MEDIA_SIGNAL: &str = "Invalid media signal.";
const NOT_SEATED: &str = "Not seated in this voice room.";

/// Creates the media session of the caller's seat in a room. A session the
/// connection already had is closed first, silently.
pub(super) async fn open_media_session(
    cx: Context,
    arguments: Vec<Value>,
) -> Result<Value, HubError> {
    let [channel] = arguments::exactly(arguments)?;
    let channel = arguments::text(channel)?;
    // A replacement loop that never proves media is the one thing the client's
    // backoff cannot be trusted to stop on its own.
    if !cx
        .realtime()
        .throttle()
        .try_acquire(cx.connection_id(), Policy::MediaSession)
    {
        return Err(visible("Media sessions open too fast."));
    }

    let room = parse_id(channel.as_deref(), "channelId")?;
    if !cx
        .realtime()
        .presence()
        .is_connection_in_voice(cx.connection_id(), room)
    {
        return Err(visible(NOT_SEATED));
    }

    match plane::open(cx.state(), cx.connection_id(), room, cx.user()).await {
        Opening::Opened(session) => Ok(json!({ "sessionId": session })),
        Opening::NotSeated => Err(visible(NOT_SEATED)),
        Opening::Unavailable => Err(visible(MEDIA_UNAVAILABLE)),
    }
}

/// What a browser sends the media server.
struct MediaSignal {
    protocol_version: i32,
    channel_id: Option<String>,
    session_id: Option<String>,
    kind: Option<String>,
    payload: Option<String>,
}

fn read_media_signal(members: &Members) -> Result<MediaSignal, HubError> {
    Ok(MediaSignal {
        protocol_version: members.number("protocolVersion")?,
        channel_id: members.text("channelId")?,
        session_id: members.text("sessionId")?,
        kind: members.text("kind")?,
        payload: members.text("payload")?,
    })
}

/// Hands one signal of the caller's media session to the media server. The
/// call returns once the media server has taken it, which keeps the signals of
/// one browser in the order it sent them.
pub(super) async fn send_media_signal(
    cx: Context,
    arguments: Vec<Value>,
) -> Result<Value, HubError> {
    let [argument] = arguments::exactly(arguments)?;
    let members = document::object(argument)?;
    let signal = members.as_ref().map(read_media_signal).transpose()?;

    // Every accepted signal becomes a call to the media server's control API,
    // so this connection cannot decide that API's load.
    if !cx
        .realtime()
        .throttle()
        .try_acquire(cx.connection_id(), Policy::MediaSignalling)
    {
        return Err(visible("Media signals are too fast."));
    }
    let Some(signal) = signal else {
        return Err(visible(INVALID_MEDIA_SIGNAL));
    };

    let room = parse_id(signal.channel_id.as_deref(), "ChannelId")?;
    // Holding the seat and the session in one look: the session of a seat in
    // another room, of another connection or of an earlier attempt is not this
    // caller's to use.
    let session = cx
        .realtime()
        .presence()
        .media_session_of(cx.connection_id(), room);
    let (Some(kind), Some(payload), Some(session)) =
        (signal.kind.as_deref(), signal.payload.as_deref(), session)
    else {
        return Err(visible(INVALID_MEDIA_SIGNAL));
    };
    if i64::from(signal.protocol_version) != VERSION
        || !MEDIA_SIGNAL_KINDS.contains(&kind)
        || payload.len() > MAX_SIGNAL_BYTES
        || signal.session_id.as_deref() != Some(session.session_id.as_str())
    {
        return Err(visible(INVALID_MEDIA_SIGNAL));
    }

    let outcome = match forward(&cx, &session.session_id, kind, payload).await {
        Ok(outcome) => outcome,
        Err(Forward::Refused(error)) => return Err(error),
        Err(Forward::Unavailable(error)) => {
            tracing::warn!("A signal for the media server failed: {error}");
            return Err(visible(MEDIA_UNAVAILABLE));
        }
    };
    match outcome {
        SignalOutcome::UnknownSession => {
            // The media server lost the session, most likely in a restart the
            // event pump has not noticed yet. The browser replaces it.
            plane::lose_session(cx.state(), cx.realtime(), &session.session_id);
        }
        SignalOutcome::Refused => return Err(visible(INVALID_MEDIA_SIGNAL)),
        // A stale answer lost a race with a newer offer of the media server,
        // which drops it by design; the browser answers that offer next.
        SignalOutcome::Accepted | SignalOutcome::Stale => {}
    }
    Ok(Value::Null)
}

enum Forward {
    Refused(HubError),
    Unavailable(Unavailable),
}

impl From<Unavailable> for Forward {
    fn from(error: Unavailable) -> Forward {
        Forward::Unavailable(error)
    }
}

/// Turns the browser's payload into a call of the control API. A payload that
/// is not what its kind promises is refused here and never reaches the media
/// server.
async fn forward(
    cx: &Context,
    session: &str,
    kind: &str,
    payload: &str,
) -> Result<SignalOutcome, Forward> {
    let invalid = || Forward::Refused(visible(INVALID_MEDIA_SIGNAL));
    let control = cx.state().voice().control();
    let Some(parsed) = document::parse_members(payload) else {
        return Err(invalid());
    };
    match (kind, &parsed) {
        ("offer", ParsedPayload::Object(root)) => {
            let sdp = description(root, "offer").ok_or_else(invalid)?;
            Ok(control.send_offer(session, &sdp).await?)
        }
        ("answer", ParsedPayload::Object(root)) => {
            let sdp = description(root, "answer").ok_or_else(invalid)?;
            let negotiation = match root.exact("negotiationId") {
                Some(Value::Number(number)) => number.as_i64().ok_or_else(invalid)?,
                _ => return Err(invalid()),
            };
            Ok(control.send_answer(session, &sdp, negotiation).await?)
        }
        ("candidate", ParsedPayload::Null) => Ok(control.send_candidate(session, None).await?),
        ("candidate", ParsedPayload::Object(_)) => {
            Ok(control.send_candidate(session, Some(payload)).await?)
        }
        ("subscriptions", ParsedPayload::Object(root)) => {
            let mids = subscriptions(root).ok_or_else(invalid)?;
            Ok(control.send_subscriptions(session, &mids).await?)
        }
        _ => Err(invalid()),
    }
}

/// An `{ "type", "sdp" }` object whose type is `kind`; its description.
fn description(root: &Members, kind: &str) -> Option<String> {
    match (root.exact("type"), root.exact("sdp")) {
        (Some(Value::String(declared)), Some(Value::String(sdp))) if declared == kind => {
            Some(sdp.clone())
        }
        _ => None,
    }
}

/// A complete allowlist of printable ASCII ids, including an empty one.
fn subscriptions(root: &Members) -> Option<Vec<String>> {
    if root.count("mids") != 1 {
        return None;
    }
    let Some(Value::Array(entries)) = root.exact("mids") else {
        return None;
    };
    if entries.len() > MAX_SUBSCRIPTION_MIDS {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    let mut mids = Vec::with_capacity(entries.len());
    for entry in entries {
        let Value::String(mid) = entry else {
            return None;
        };
        // The characters are ASCII, so a length in bytes is a length in the
        // units the limit counts.
        if mid.is_empty()
            || mid.len() > MAX_SUBSCRIPTION_MID_LENGTH
            || mid.chars().any(|c| c <= ' ' || c >= '\u{7f}')
            || !seen.insert(mid.as_str())
        {
            return None;
        }
        mids.push(mid.clone());
    }
    Some(mids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(text: &str) -> Members {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn a_description_names_its_type_and_carries_text() {
        let offer = members("{\"type\":\"offer\",\"sdp\":\"v=0\"}");
        assert_eq!(description(&offer, "offer"), Some("v=0".to_string()));
        assert_eq!(description(&offer, "answer"), None);
        assert_eq!(description(&members("{\"type\":\"offer\"}"), "offer"), None);
        assert_eq!(
            description(&members("{\"type\":\"offer\",\"sdp\":5}"), "offer"),
            None
        );
        assert_eq!(
            description(&members("{\"type\":\"OFFER\",\"sdp\":\"x\"}"), "offer"),
            None
        );
    }

    #[test]
    fn a_subscription_list_is_complete_ordered_and_printable() {
        let list = |text: &str| subscriptions(&members(text));

        assert_eq!(list("{\"mids\":[]}"), Some(vec![]));
        assert_eq!(
            list("{\"mids\":[\"b\",\"a\",\"~\",\"!\"]}"),
            Some(vec!["b".into(), "a".into(), "~".into(), "!".into()])
        );
        for bad in [
            "{}",
            "{\"mids\":null}",
            "{\"mids\":\"a\"}",
            "{\"mids\":[1]}",
            "{\"mids\":[\"\"]}",
            "{\"mids\":[\"a b\"]}",
            "{\"mids\":[\"\\u007f\"]}",
            "{\"mids\":[\"é\"]}",
            "{\"mids\":[\"a\",\"a\"]}",
            "{\"mids\":[\"abcdefghijklmnopq\"]}",
            "{\"mids\":[],\"mids\":[]}",
            "{\"Mids\":[]}",
        ] {
            assert_eq!(list(bad), None, "{bad}");
        }
        assert!(list("{\"mids\":[\"abcdefghijklmnop\"]}").is_some());
    }

    #[test]
    fn a_list_holds_4096_ids_and_no_more() {
        let many = |count: usize| {
            let ids: Vec<String> = (0..count).map(|n| format!("\"m{n}\"")).collect();
            format!("{{\"mids\":[{}]}}", ids.join(","))
        };
        assert_eq!(subscriptions(&members(&many(4096))).map(|l| l.len()), Some(4096));
        assert_eq!(subscriptions(&members(&many(4097))), None);
    }
}
