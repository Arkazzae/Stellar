//! What a connection can do in a voice room, and what a connection that starts
//! or ends does to the seats.
//!
//! The browsers in a room tell each other their state through the hub, while
//! their media goes through the media server (see [`super::media`]); this hub
//! decides who may take part.

use serde::Serialize;
use serde_json::{Value, json};
use stellarr_hub::HubError;

use super::document::{self, Members, ParsedPayload};
use super::protocol::{MAX_SIGNAL_BYTES, SIGNAL_KINDS, VERSION};
use super::rooms::{self, broadcast_snapshot};
use crate::events::{Audience, Event};
use crate::permissions::privileges::{CONNECT, VIEW_CHANNEL};
use crate::realtime::method::{Context, Method, Registration, arguments};
use crate::realtime::names::parse_guid;
use crate::realtime::throttle::Policy;
use crate::state::AppState;
use crate::values::Id;

pub fn registration() -> Registration {
    Registration::new()
        .method(Method::new("JoinVoice", join_voice))
        .method(Method::new("LeaveVoice", leave_voice))
        .method(Method::new("DeclineDirectCall", decline_direct_call))
        .method(Method::new("SendVoiceSignal", send_voice_signal))
        .method(Method::new("OpenMediaSession", super::media::open_media_session))
        .method(Method::new("SendMediaSignal", super::media::send_media_signal))
        .method(Method::new("PlaySoundboard", super::sounds::play_soundboard))
        .on_connected(|cx| async move {
            // A caller that was offline, or reconnecting, while a call was
            // started needs to hear about it.
            rooms::restore_direct_calls(cx.state(), cx.user())
                .await
                .map_err(|error| HubError::internal(error.to_string()))
        })
        .on_disconnected(|cx, _error| async move {
            // The seat went with the connection; the room hears of it now.
            if let Some(departure) = cx
                .realtime()
                .presence()
                .take_departure(cx.connection_id())
                && let Err(error) = broadcast_snapshot(cx.state(), departure.channel).await
            {
                tracing::warn!(%error, "a room was not told that somebody left it");
            }
        })
}

/// A hub text that the person reads.
pub(super) fn visible(text: &str) -> HubError {
    HubError::visible(text)
}

pub(super) fn internal(error: impl std::fmt::Display) -> HubError {
    HubError::internal(error.to_string())
}

/// An id a client wrote, which the hub reads as the first server read one;
/// `field` is what its refusal calls it.
pub(super) fn parse_id(text: Option<&str>, field: &str) -> Result<Id, HubError> {
    text.and_then(parse_guid)
        .ok_or_else(|| HubError::visible(format!("Invalid {field}.")))
}

/// Sends one event to each of these connections.
pub(super) fn send_to<T: Serialize>(
    cx: &Context,
    connections: impl IntoIterator<Item = String>,
    name: &str,
    payload: &T,
) {
    send_to_connections(cx.state(), connections, name, payload);
}

/// Sends one event to each of these connections, one after another.
pub(super) fn send_to_connections<T: Serialize>(
    state: &AppState,
    connections: impl IntoIterator<Item = String>,
    name: &str,
    payload: &T,
) {
    for connection in connections {
        state
            .events()
            .publish(Event::new(Audience::Connection(connection), name, payload));
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChannelOnly {
    channel_id: Id,
}

/// Takes a seat in a voice room. The seat is what admits a connection to the
/// room's media: opening a media session asks the media server for one only on
/// behalf of a seat, and the media server takes commands from this API alone.
/// So viewing and connecting are decided here, in the channel, where their
/// overrides count. A connection that already sits in the room keeps its seat
/// without them, because a moderator may have moved it there and the client
/// confirms a move with this call. Anybody else who does not see the room is
/// answered as for a room that does not exist.
async fn join_voice(cx: Context, arguments: Vec<Value>) -> Result<Value, HubError> {
    let [channel] = arguments::exactly(arguments)?;
    let room = parse_id(arguments::text(channel)?.as_deref(), "channelId")?;
    let user = cx.user();
    let presence = cx.realtime().clone();
    let (access, held) = cx
        .state()
        .database()
        .read(move |db| -> Result<_, crate::database::DbError> {
            let access = rooms::access(db, presence.presence(), room, user)?;
            let held = if access.allowed {
                rooms::held_in(db, &access, room, user)?
            } else {
                crate::permissions::authorization::Held(0)
            };
            Ok((access, held))
        })
        .await
        .map_err(internal)?;
    if !access.allowed {
        return Err(visible("Unknown voice room."));
    }

    let join = cx
        .realtime()
        .presence()
        .join_voice(
            cx.connection_id(),
            user,
            room,
            access.user_limit,
            held.has(CONNECT),
        )
        .ok_or_else(|| internal("only a connected account can take a seat"))?;
    if !join.admitted {
        return Err(visible(if held.has(VIEW_CHANNEL) {
            "Not allowed to join this voice room."
        } else {
            "Unknown voice room."
        }));
    }
    if !join.accepted {
        return Err(visible("Voice room is full."));
    }

    for displaced in &join.displaced {
        send_to(
            &cx,
            [displaced.connection_id.clone()],
            "VoiceSessionReplaced",
            &ChannelOnly {
                channel_id: displaced.channel,
            },
        );
    }

    let mut vacated: Vec<Id> = Vec::new();
    for left in join
        .displaced
        .iter()
        .map(|displaced| displaced.channel)
        .chain(join.previous_channel)
    {
        if left != room && !vacated.contains(&left) {
            vacated.push(left);
        }
    }
    for left in vacated {
        broadcast_snapshot(cx.state(), left).await.map_err(internal)?;
    }
    broadcast_snapshot(cx.state(), room).await.map_err(internal)?;
    super::sounds::announce_join_sound(&cx, room, user)
        .await
        .map_err(internal)?;
    Ok(Value::Null)
}

async fn leave_voice(cx: Context, arguments: Vec<Value>) -> Result<Value, HubError> {
    let [] = arguments::exactly(arguments)?;
    if let Some(departure) = cx.realtime().presence().leave_voice(cx.connection_id()) {
        broadcast_snapshot(cx.state(), departure.channel)
            .await
            .map_err(internal)?;
    }
    Ok(Value::Null)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DirectCallDeclined {
    pub(super) channel_id: Id,
    pub(super) by_user_id: Id,
}

/// Ends a direct call for both people, whether or not it was ever answered.
async fn decline_direct_call(cx: Context, arguments: Vec<Value>) -> Result<Value, HubError> {
    let [conversation] = arguments::exactly(arguments)?;
    let id = parse_id(
        arguments::text(conversation)?.as_deref(),
        "conversationId",
    )?;
    let user = cx.user();

    // Either person may always decline, even after a block or a change of
    // preferences that would now refuse a new call: a call that is still
    // ringing has to stay dismissable.
    let is_theirs = cx
        .state()
        .database()
        .read(move |db| -> Result<bool, crate::database::DbError> {
            Ok(rooms::conversation_of(db, id, user)?.is_some())
        })
        .await
        .map_err(internal)?;
    if !is_theirs {
        return Err(visible("Unknown direct call."));
    }

    let seated = cx.realtime().presence().end_voice_room(id);
    send_to(
        &cx,
        seated,
        "DirectCallDeclined",
        &DirectCallDeclined {
            channel_id: id,
            by_user_id: user,
        },
    );
    broadcast_snapshot(cx.state(), id).await.map_err(internal)?;
    Ok(Value::Null)
}

/// What a browser sends the others in its room.
struct VoiceSignal {
    protocol_version: i32,
    channel_id: Option<String>,
    to_user_id: Option<String>,
    kind: Option<String>,
    payload: Option<String>,
}

fn read_voice_signal(members: &Members) -> Result<VoiceSignal, HubError> {
    Ok(VoiceSignal {
        protocol_version: members.number("protocolVersion")?,
        channel_id: members.text("channelId")?,
        to_user_id: members.text("toUserId")?,
        kind: members.text("kind")?,
        payload: members.text("payload")?,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Relayed<'a> {
    protocol_version: i64,
    channel_id: Id,
    from_user_id: Id,
    to_user_id: Option<&'a str>,
    kind: &'a str,
    payload: &'a str,
}

async fn send_voice_signal(cx: Context, arguments: Vec<Value>) -> Result<Value, HubError> {
    // The arguments are read before anything is counted: a call whose argument
    // does not fit never reaches the method.
    let [argument] = arguments::exactly(arguments)?;
    let members = document::object(argument)?;
    let signal = members.as_ref().map(read_voice_signal).transpose()?;

    // A relay reaches every other connection in the room and may carry a
    // hundred and twenty kilobytes, so the flood is capped before any of the
    // lookups below. The burst covers several connections' ICE candidates
    // arriving together, which is normal rather than flood.
    if !cx
        .realtime()
        .throttle()
        .try_acquire(cx.connection_id(), Policy::Signalling)
    {
        return Err(visible("Signalling is too fast."));
    }
    let Some(signal) = signal else {
        return Err(visible("Invalid voice signal."));
    };

    let room = parse_id(signal.channel_id.as_deref(), "ChannelId")?;
    let sender = cx.user();
    // Only the state travels from browser to browser. An offer, answer or
    // candidate would connect two browsers directly, bypassing the media server
    // and giving away their addresses, so it is refused.
    let relayable = i64::from(signal.protocol_version) == VERSION
        && signal
            .kind
            .as_deref()
            .is_some_and(|kind| SIGNAL_KINDS.contains(&kind))
        && signal
            .payload
            .as_deref()
            .is_some_and(|payload| payload.len() <= MAX_SIGNAL_BYTES);
    let presence = cx.realtime().clone();
    let connection = cx.connection_id().to_string();
    let allowed = relayable
        && presence.presence().is_connection_in_voice(&connection, room)
        && cx
            .state()
            .database()
            .read({
                let presence = presence.clone();
                move |db| -> Result<bool, crate::database::DbError> {
                    Ok(rooms::access(db, presence.presence(), room, sender)?.allowed)
                }
            })
            .await
            .map_err(internal)?;
    let (Some(kind), Some(payload), true) = (signal.kind.as_deref(), signal.payload.as_deref(), allowed)
    else {
        return Err(visible("Invalid voice signal."));
    };

    // The room's occupancy follows the sender's own state whether or not
    // anybody is left to receive the signal.
    if kind == "state"
        && let Some(muted) = read_muted(payload)?
    {
        presence.presence().set_voice_muted(&connection, muted);
        broadcast_snapshot(cx.state(), room).await.map_err(internal)?;
    }

    let recipients = recipients(&cx, &signal, room)?;
    if !recipients.is_empty() {
        send_to(
            &cx,
            recipients,
            "VoiceSignal",
            &Relayed {
                protocol_version: VERSION,
                channel_id: room,
                from_user_id: sender,
                to_user_id: signal.to_user_id.as_deref(),
                kind,
                payload,
            },
        );
    }
    Ok(json!(null))
}

/// Who gets a signal: the one peer it names, which must sit in the room, or
/// else every other connection there.
fn recipients(cx: &Context, signal: &VoiceSignal, room: Id) -> Result<Vec<String>, HubError> {
    let presence = cx.realtime().presence();
    let Some(to) = signal.to_user_id.as_deref() else {
        return Ok(presence
            .voice_connections(room)
            .into_iter()
            .filter(|connection| connection != cx.connection_id())
            .collect());
    };
    let peers = parse_guid(to)
        .map(|peer| presence.voice_connections_of(peer, room))
        .unwrap_or_default();
    if peers.is_empty() {
        Err(visible("Voice signal target is not in this channel."))
    } else {
        Ok(peers)
    }
}

/// The mute flag of a state payload; none when it does not say.
fn read_muted(payload: &str) -> Result<Option<bool>, HubError> {
    match document::parse_members(payload) {
        Some(ParsedPayload::Object(members)) => Ok(match members.exact("muted") {
            Some(Value::Bool(muted)) => Some(*muted),
            _ => None,
        }),
        _ => Err(visible("Invalid voice state.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mute_flag_is_the_last_muted_member_when_it_is_a_boolean() {
        assert_eq!(read_muted("{\"muted\":true}").unwrap(), Some(true));
        assert_eq!(
            read_muted("{\"muted\":true,\"muted\":false}").unwrap(),
            Some(false)
        );
        for silent in ["{}", "{\"Muted\":true}", "{\"muted\":null}", "{\"muted\":1}", "{\"muted\":\"yes\"}"] {
            assert_eq!(read_muted(silent).unwrap(), None, "{silent}");
        }
    }

    #[test]
    fn a_payload_that_is_not_an_object_is_no_voice_state() {
        for bad in ["", "[]", "\"x\"", "null", "{", "{\"muted\":true,}", "1"] {
            assert!(read_muted(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_id_is_read_as_the_first_server_read_one_and_refused_by_the_name_of_its_field() {
        let id = "380949bf-331b-48a3-bd4b-be26c59f8487";
        assert!(parse_id(Some(id), "channelId").is_ok());
        assert!(parse_id(Some(&format!("{{{id}}}")), "channelId").is_ok());
        assert!(parse_id(Some(&id.replace('-', "")), "channelId").is_ok());
        assert_eq!(
            parse_id(Some("x"), "conversationId"),
            Err(HubError::visible("Invalid conversationId."))
        );
        assert_eq!(
            parse_id(None, "ChannelId"),
            Err(HubError::visible("Invalid ChannelId."))
        );
    }
}
