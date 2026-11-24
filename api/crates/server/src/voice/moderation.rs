//! Moderating a live voice session without touching the membership: sending a
//! member out of a voice channel, or from one voice channel to another.
//!
//! Both are the moderator's act and need what the permissions module decides
//! for it in the room (and in both rooms, for a move). A moderator who does not
//! see the room is answered as for a room that does not exist.

use std::collections::HashSet;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::Connection;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use stellarr_messages::{MessageKey, MissingValue};

use super::hub::send_to_connections;
use super::rooms::broadcast_snapshot;
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Path};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization::{self, MemberAuthorization};
use crate::permissions::membership::ActiveServer;
use crate::realtime::Realtime;
use crate::state::AppState;
use crate::values::Id;

/// The room somebody is sent to, as a request names it. A member that is
/// missing reads as no room at all, which the moderator does not see.
pub(super) struct MoveRequest {
    target_channel_id: Option<Id>,
}

impl<'de> Deserialize<'de> for MoveRequest {
    /// Only an object is a request: a list is not one, whatever it holds. A
    /// member that is there must be an id (`null` is none), and members nobody
    /// asked for are ignored.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<MoveRequest, D::Error> {
        struct Reader;

        impl<'de> Visitor<'de> for Reader {
            type Value = MoveRequest;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<MoveRequest, A::Error> {
                let mut target_channel_id = None;
                while let Some(name) = map.next_key::<String>()? {
                    if name == "targetChannelId" {
                        target_channel_id = Some(map.next_value::<Id>()?);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(MoveRequest { target_channel_id })
            }
        }

        deserializer.deserialize_map(Reader)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Kicked {
    channel_id: Id,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Moved {
    from_channel_id: Id,
    to_channel_id: Id,
}

enum Refusal {
    NotFound,
    Forbidden,
}

fn refusal(decision: &MemberAuthorization) -> Option<Refusal> {
    match decision {
        MemberAuthorization::Allowed { .. } => None,
        MemberAuthorization::NotFound => Some(Refusal::NotFound),
        MemberAuthorization::Forbidden => Some(Refusal::Forbidden),
    }
}

fn refuse(refusal: Refusal) -> ApiError {
    match refusal {
        Refusal::NotFound => ApiError::not_found(),
        Refusal::Forbidden => ApiError::forbidden(),
    }
}

fn display_name(db: &Connection, user: Id) -> rusqlite::Result<String> {
    db.query_row(
        "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
        [user],
        |row| row.get(0),
    )
}

fn channel_name(db: &Connection, channel: Id) -> rusqlite::Result<String> {
    db.query_row(
        "SELECT \"Name\" FROM \"Channels\" WHERE \"Id\" = ?1",
        [channel],
        |row| row.get(0),
    )
}

fn realtime(state: &AppState) -> ApiResult<std::sync::Arc<Realtime>> {
    state
        .realtime()
        .cloned()
        .ok_or_else(|| ApiError::internal("there is no hub to hold voice seats"))
}

enum Kick {
    Refused(Refusal),
    Done(Vec<String>),
}

/// Sends a member out of a voice channel, every connection of theirs that sits
/// there. Kicking somebody who has already gone is the end state that was
/// asked for.
pub(super) async fn kick_from_voice(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path((channel, target)): Path<(Id, Id)>,
) -> ApiResult<Response> {
    let realtime = realtime(&state)?;
    let messages = *state.messages();
    let held = realtime.clone();
    let actor = caller.user;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Kick, Failure> {
            let decision = authorization::kick_voice_member(tx, actor, server, channel, target)?;
            if let Some(refused) = refusal(&decision) {
                return Ok(Kick::Refused(refused));
            }

            let presence = held.presence();
            let seated = presence.voice_connections_of(target, channel);
            presence.remove_user_from_voice_channels(target, &HashSet::from([channel]));
            if !seated.is_empty() {
                let name = display_name(tx, target)?;
                let channel_name = channel_name(tx, channel)?;
                audit::record(
                    tx,
                    server,
                    actor,
                    &AuditEvent {
                        action: "voice.disconnect".to_string(),
                        sentence: messages.get(
                            MessageKey::AuditVoiceDisconnected,
                            &[("name", &name), ("channel", &channel_name)],
                        )?,
                        target_type: Some("member".to_string()),
                        target_id: Some(target),
                        target_name: Some(name),
                        changes: vec![AuditChange {
                            field: "channel".to_string(),
                            from: Some(channel_name),
                            ..AuditChange::default()
                        }],
                    },
                )?;
            }
            Ok(Kick::Done(seated))
        })
        .await
        .map_err(ApiError::from)?;

    match outcome {
        Kick::Refused(refused) => Err(refuse(refused)),
        Kick::Done(seated) => {
            if !seated.is_empty() {
                send_to_connections(&state, seated, "VoiceChannelKicked", &Kicked { channel_id: channel });
                if let Err(error) = broadcast_snapshot(&state, channel).await {
                    tracing::warn!(%error, "a room was not told that somebody was sent out of it");
                }
            }
            Ok(StatusCode::NO_CONTENT.into_response())
        }
    }
}

enum Move {
    Refused(Refusal),
    Unchanged,
    /// Nobody of that account sat in the room.
    Conflict,
    Done(Vec<String>),
}

/// Sends a member from one voice channel to another. The connections keep
/// their seats and their mute state; the client is told to rebuild its peers
/// for the new room.
pub(super) async fn move_between_voice_channels(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path((channel, target)): Path<(Id, Id)>,
    Json(request): Json<MoveRequest>,
) -> ApiResult<Response> {
    let realtime = realtime(&state)?;
    let messages = *state.messages();
    let held = realtime.clone();
    let actor = caller.user;
    let destination = request.target_channel_id.unwrap_or(Id::from_bytes([0; 16]));
    let outcome = state
        .database()
        .write(move |tx| -> Result<Move, Failure> {
            let decision =
                authorization::move_voice_member(tx, actor, server, channel, destination, target)?;
            if let Some(refused) = refusal(&decision) {
                return Ok(Move::Refused(refused));
            }
            if channel == destination {
                return Ok(Move::Unchanged);
            }

            let moved = held
                .presence()
                .move_user_between_voice_channels(target, channel, destination);
            if !moved.moved {
                return Ok(Move::Conflict);
            }

            let from = channel_name(tx, channel)?;
            let to = channel_name(tx, destination)?;
            let name = display_name(tx, target)?;
            audit::record(
                tx,
                server,
                actor,
                &AuditEvent {
                    action: "voice.move".to_string(),
                    sentence: messages.get(
                        MessageKey::AuditVoiceMoved,
                        &[("name", &name), ("from", &from), ("to", &to)],
                    )?,
                    target_type: Some("member".to_string()),
                    target_id: Some(target),
                    target_name: Some(name),
                    changes: vec![AuditChange {
                        field: "channel".to_string(),
                        from: Some(from),
                        to: Some(to),
                        ..AuditChange::default()
                    }],
                },
            )?;
            Ok(Move::Done(moved.connection_ids))
        })
        .await
        .map_err(ApiError::from)?;

    match outcome {
        Move::Refused(refused) => Err(refuse(refused)),
        Move::Unchanged => Ok(StatusCode::NO_CONTENT.into_response()),
        Move::Conflict => Err(ApiError::conflict()),
        Move::Done(connections) => {
            send_to_connections(
                &state,
                connections,
                "VoiceChannelMoved",
                &Moved {
                    from_channel_id: channel,
                    to_channel_id: destination,
                },
            );
            for room in [channel, destination] {
                if let Err(error) = broadcast_snapshot(&state, room).await {
                    tracing::warn!(%error, "a room was not told that somebody was moved");
                }
            }
            Ok(StatusCode::NO_CONTENT.into_response())
        }
    }
}

/// What can go wrong inside the change: the database, or a text of the
/// catalogue asked for without a value it needs, which is a mistake in the code
/// that asked.
#[derive(Debug)]
enum Failure {
    Database(DbError),
    Message(MissingValue),
}

impl From<DbError> for Failure {
    fn from(error: DbError) -> Failure {
        Failure::Database(error)
    }
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Failure {
        Failure::Database(error.into())
    }
}

impl From<MissingValue> for Failure {
    fn from(error: MissingValue) -> Failure {
        Failure::Message(error)
    }
}

impl From<Failure> for ApiError {
    fn from(failure: Failure) -> ApiError {
        match failure {
            Failure::Database(error) => error.into(),
            Failure::Message(error) => ApiError::internal(error),
        }
    }
}
