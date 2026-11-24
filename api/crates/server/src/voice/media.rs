//! The hub leg of the media server, which every voice room goes through: a
//! browser opens a media session for its seat and exchanges signals with the
//! media server through the hub, which decides who may do so. The media server
//! trusts the hub and nobody else.

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Value, json};
use stellarr_hub::HubError;

use super::control::{SignalOutcome, Unavailable};
use super::document;
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

/// What a browser sends the media server. A version that is missing reads as
/// 0, which no server speaks.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaSignal {
    #[serde(default)]
    protocol_version: i32,
    channel_id: Option<String>,
    session_id: Option<String>,
    kind: Option<String>,
    payload: Option<String>,
}

/// Hands one signal of the caller's media session to the media server. The
/// call returns once the media server has taken it, which keeps the signals of
/// one browser in the order it sent them.
pub(super) async fn send_media_signal(
    cx: Context,
    arguments: Vec<Value>,
) -> Result<Value, HubError> {
    let [argument] = arguments::exactly(arguments)?;
    let signal: Option<MediaSignal> = document::argument(argument)?;

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
    match kind {
        "offer" => {
            let sdp = document::read::<Description>(payload)
                .and_then(|found| found.sdp_of("offer"))
                .ok_or_else(invalid)?;
            Ok(control.send_offer(session, &sdp).await?)
        }
        "answer" => {
            let found = document::read::<Description>(payload).ok_or_else(invalid)?;
            let negotiation = found.negotiation_id.ok_or_else(invalid)?;
            let sdp = found.sdp_of("answer").ok_or_else(invalid)?;
            Ok(control.send_answer(session, &sdp, negotiation).await?)
        }
        "candidate" => match document::read::<Option<Candidate>>(payload) {
            Some(None) => Ok(control.send_candidate(session, None).await?),
            Some(Some(_)) => Ok(control.send_candidate(session, Some(payload)).await?),
            None => Err(invalid()),
        },
        "subscriptions" => {
            let mids = document::read::<Subscriptions>(payload)
                .and_then(Subscriptions::checked)
                .ok_or_else(invalid)?;
            Ok(control.send_subscriptions(session, &mids).await?)
        }
        _ => Err(invalid()),
    }
}

/// An `{ "type", "sdp" }` object, with the negotiation an answer belongs to.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Description {
    #[serde(rename = "type")]
    kind: Option<String>,
    sdp: Option<String>,
    negotiation_id: Option<i64>,
}

impl Description {
    /// The description, when its type is `kind`.
    fn sdp_of(self, kind: &str) -> Option<String> {
        match (self.kind, self.sdp) {
            (Some(declared), Some(sdp)) if declared == kind => Some(sdp),
            _ => None,
        }
    }
}

/// An ICE candidate is handed on as it was written; reading it here only says
/// that it is an object whose members are not repeated.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct Candidate {
    candidate: Option<IgnoredAny>,
    sdp_mid: Option<IgnoredAny>,
    sdp_m_line_index: Option<IgnoredAny>,
    username_fragment: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct Subscriptions {
    mids: Vec<String>,
}

impl Subscriptions {
    /// A complete allowlist of printable ASCII ids, including an empty one.
    fn checked(self) -> Option<Vec<String>> {
        let entries = self.mids;
        if entries.len() > MAX_SUBSCRIPTION_MIDS {
            return None;
        }
        let mut seen = std::collections::HashSet::new();
        for mid in &entries {
            // The characters are ASCII, so a length in bytes is a length in
            // the units the limit counts.
            if mid.is_empty()
                || mid.len() > MAX_SUBSCRIPTION_MID_LENGTH
                || mid.chars().any(|c| c <= ' ' || c >= '\u{7f}')
                || !seen.insert(mid.as_str())
            {
                return None;
            }
        }
        Some(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn description(text: &str, kind: &str) -> Option<String> {
        document::read::<Description>(text).and_then(|found| found.sdp_of(kind))
    }

    #[test]
    fn a_description_names_its_type_and_carries_text() {
        let offer = "{\"type\":\"offer\",\"sdp\":\"v=0\"}";
        assert_eq!(description(offer, "offer"), Some("v=0".to_string()));
        assert_eq!(description(offer, "answer"), None);
        assert_eq!(description("{\"type\":\"offer\"}", "offer"), None);
        assert_eq!(description("{\"type\":\"offer\",\"sdp\":5}", "offer"), None);
        assert_eq!(description("{\"type\":\"OFFER\",\"sdp\":\"x\"}", "offer"), None);
        assert_eq!(
            description("{\"type\":\"offer\",\"sdp\":\"a\",\"sdp\":\"b\"}", "offer"),
            None
        );
    }

    #[test]
    fn a_subscription_list_is_complete_ordered_and_printable() {
        let list = |text: &str| document::read::<Subscriptions>(text).and_then(Subscriptions::checked);

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
        let list = |text: &str| document::read::<Subscriptions>(text).and_then(Subscriptions::checked);
        assert_eq!(list(&many(4096)).map(|l| l.len()), Some(4096));
        assert_eq!(list(&many(4097)), None);
    }

    #[test]
    fn a_candidate_is_an_object_or_null_without_a_repeated_member() {
        for good in ["null", "{}", "{\"candidate\":\"x\",\"sdpMid\":\"0\"}"] {
            assert!(document::read::<Option<Candidate>>(good).is_some(), "{good}");
        }
        for bad in ["[]", "5", "\"x\"", "{\"candidate\":\"x\",\"candidate\":\"y\"}", ""] {
            assert!(document::read::<Option<Candidate>>(bad).is_none(), "{bad}");
        }
    }
}
