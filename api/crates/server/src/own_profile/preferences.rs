//! `PATCH /api/me/preferences`: the switches and choices of the own account.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use super::ensure_preferences;
use crate::accounts::me::Preferences;
use crate::auth::Caller;
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Json, Problem, json_response};
use crate::permissions::membership::parse_guid;
use crate::realtime::audience::presence_reach;
use crate::servers::Object;
use crate::soundboard::numbers::optional_whole;
use crate::state::AppState;
use crate::values::Id;

/// A property left out, or null, changes nothing; so does every other one.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePreferences {
    desktop_notifications: Option<bool>,
    email_notifications: Option<bool>,
    push_notifications: Option<bool>,
    mentions_only: Option<bool>,
    message_sound: Option<bool>,
    join_leave_sound: Option<bool>,
    allow_direct_messages: Option<bool>,
    allow_friend_requests: Option<bool>,
    show_presence: Option<bool>,
    developer_mode: Option<bool>,
    reduced_motion: Option<bool>,
    muted_channel_ids: Option<Vec<Id>>,
    #[serde(default, deserialize_with = "optional_whole")]
    vad_sensitivity: Option<i32>,
    /// A soundboard sound to announce this account's arrivals with. Empty
    /// clears the choice; absent leaves it alone.
    join_sound_id: Option<String>,
    mention_popups: Option<bool>,
    call_ringtone: Option<bool>,
}

fn refusal(state: &AppState, field: &str, key: MessageKey) -> Result<ApiError, ApiError> {
    let text = state.messages().get(key, &[])?;
    Ok(Problem::validation(vec![(field.to_string(), vec![text])]).into())
}

enum Saved {
    NotAvailable,
    Done(Box<Preferences>),
}

pub async fn update(
    State(state): State<AppState>,
    caller: Caller,
    Json(Object(request)): Json<Object<UpdatePreferences>>,
) -> ApiResult<Response> {
    if request.vad_sensitivity.is_some_and(|value| !(0..=100).contains(&value)) {
        return Err(refusal(&state, "vadSensitivity", MessageKey::VadSensitivityRange)?);
    }
    let user = caller.user;
    // What is asked of the sound: blank clears it, anything else must be an id
    // of a sound on a server the account is in.
    let requested = request.join_sound_id.as_deref().map(str::trim);
    let sound = requested.filter(|text| !text.is_empty()).map(parse_guid);
    let change_sound = request.join_sound_id.is_some();
    let show_presence = request.show_presence;
    let muted = request
        .muted_channel_ids
        .as_ref()
        .map(|ids| {
            let mut seen = std::collections::HashSet::new();
            let distinct: Vec<Id> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
            serde_json::to_string(&distinct)
        })
        .transpose()
        .map_err(ApiError::internal)?;
    let saved = state
        .database()
        .write(move |tx| -> Result<Saved, DbError> {
            // An arrival may only be announced with a sound this account can
            // already play, which is a question for the database and not for
            // the client that sent the id.
            let chosen = match sound {
                None => None,
                Some(None) => return Ok(Saved::NotAvailable),
                Some(Some(id)) => {
                    let playable: bool = tx.query_row(
                        "SELECT EXISTS (SELECT 1 FROM \"SoundboardSounds\" s WHERE s.\"Id\" = ?1 AND EXISTS (
                             SELECT 1 FROM \"ServerMembers\" m WHERE m.\"ServerId\" = s.\"ServerId\" AND m.\"UserId\" = ?2))",
                        params![id, user],
                        |row| row.get(0),
                    )?;
                    if !playable {
                        return Ok(Saved::NotAvailable);
                    }
                    Some(id)
                }
            };
            ensure_preferences(tx, user)?;
            tx.execute(
                "UPDATE \"UserPreferences\" SET
                    \"DesktopNotifications\" = COALESCE(?1, \"DesktopNotifications\"),
                    \"MentionPopups\" = COALESCE(?2, \"MentionPopups\"),
                    \"CallRingtone\" = COALESCE(?3, \"CallRingtone\"),
                    \"EmailNotifications\" = COALESCE(?4, \"EmailNotifications\"),
                    \"PushNotifications\" = COALESCE(?5, \"PushNotifications\"),
                    \"MentionsOnly\" = COALESCE(?6, \"MentionsOnly\"),
                    \"MessageSound\" = COALESCE(?7, \"MessageSound\"),
                    \"JoinLeaveSound\" = COALESCE(?8, \"JoinLeaveSound\"),
                    \"AllowDirectMessages\" = COALESCE(?9, \"AllowDirectMessages\"),
                    \"AllowFriendRequests\" = COALESCE(?10, \"AllowFriendRequests\"),
                    \"ShowPresence\" = COALESCE(?11, \"ShowPresence\"),
                    \"DeveloperMode\" = COALESCE(?12, \"DeveloperMode\"),
                    \"ReducedMotion\" = COALESCE(?13, \"ReducedMotion\"),
                    \"VadSensitivity\" = COALESCE(?14, \"VadSensitivity\"),
                    \"MutedChannelIdsJson\" = COALESCE(?15, \"MutedChannelIdsJson\"),
                    \"JoinSoundId\" = CASE WHEN ?16 THEN ?17 ELSE \"JoinSoundId\" END
                 WHERE \"UserId\" = ?18",
                params![
                    request.desktop_notifications,
                    request.mention_popups,
                    request.call_ringtone,
                    request.email_notifications,
                    request.push_notifications,
                    request.mentions_only,
                    request.message_sound,
                    request.join_leave_sound,
                    request.allow_direct_messages,
                    request.allow_friend_requests,
                    request.show_presence,
                    request.developer_mode,
                    request.reduced_motion,
                    request.vad_sensitivity,
                    muted,
                    change_sound,
                    chosen,
                    user,
                ],
            )?;
            Ok(Saved::Done(Box::new(Preferences::load(tx, user)?)))
        })
        .await?;
    let preferences = match saved {
        Saved::NotAvailable => {
            return Err(refusal(&state, "joinSoundId", MessageKey::JoinSoundNotAvailable)?);
        }
        Saved::Done(preferences) => preferences,
    };
    if let Some(visible) = show_presence
        && state.presence().set_visibility(user, visible)
    {
        announce_presence(&state, user).await?;
    }
    Ok(json_response(StatusCode::OK, &*preferences))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PresenceChanged {
    user_id: Id,
    presence: String,
}

/// Tells the servers the person is on and the people they talk to what status
/// they now show, and the person's own connections what they themselves see.
async fn announce_presence(state: &AppState, user: Id) -> Result<(), ApiError> {
    let reach = state
        .database()
        .read(move |db| -> Result<_, DbError> { Ok(presence_reach(db, user)?) })
        .await?;
    let shown = PresenceChanged {
        user_id: user,
        presence: state.presence().public_presence_of(user, None),
    };
    if !reach.servers.is_empty() || !reach.peers.is_empty() {
        let audience = Audience::Reach { servers: reach.servers, users: reach.peers };
        state.events().publish(Event::new(audience, "PresenceChanged", &shown));
    }
    let own = PresenceChanged { user_id: user, presence: state.presence_of(user) };
    state
        .events()
        .publish(Event::new(Audience::Users(vec![user]), "PresenceChanged", &own));
    Ok(())
}
