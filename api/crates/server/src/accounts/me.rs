//! The account of the person who is signed in, as only they see it.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, json_response};
use crate::instance::users::User;
use crate::state::AppState;
use crate::values::Id;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Account {
    id: Id,
    username: String,
    display_name: String,
    email: Option<String>,
    /// Written without a zone, as the first server wrote it.
    created_at: String,
    avatar_url: Option<String>,
    banner_url: Option<String>,
    wallpaper_url: Option<String>,
    custom_status: String,
    bio: String,
    pronouns: String,
    preferences: Preferences,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preferences {
    desktop_notifications: bool,
    email_notifications: bool,
    push_notifications: bool,
    mentions_only: bool,
    message_sound: bool,
    join_leave_sound: bool,
    allow_direct_messages: bool,
    allow_friend_requests: bool,
    show_presence: bool,
    developer_mode: bool,
    reduced_motion: bool,
    muted_channel_ids: Vec<String>,
    vad_sensitivity: i64,
    join_sound_id: Option<Id>,
    mention_popups: bool,
    call_ringtone: bool,
}

impl Preferences {
    /// What an account that never saved any has.
    fn defaults() -> Preferences {
        Preferences {
            desktop_notifications: true,
            email_notifications: false,
            push_notifications: false,
            mentions_only: true,
            message_sound: true,
            join_leave_sound: true,
            allow_direct_messages: true,
            allow_friend_requests: true,
            show_presence: true,
            developer_mode: false,
            reduced_motion: false,
            muted_channel_ids: Vec::new(),
            vad_sensitivity: 60,
            join_sound_id: None,
            mention_popups: true,
            call_ringtone: true,
        }
    }

    pub(crate) fn load(connection: &Connection, user: Id) -> rusqlite::Result<Preferences> {
        let found = connection
            .query_row(
                "SELECT \"DesktopNotifications\", \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\",
                        \"JoinLeaveSound\", \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\",
                        \"ReducedMotion\", \"MutedChannelIdsJson\", \"VadSensitivity\", \"JoinSoundId\", \"MentionPopups\", \"CallRingtone\"
                 FROM \"UserPreferences\" WHERE \"UserId\" = ?1",
                [user],
                |row| {
                    Ok(Preferences {
                        desktop_notifications: row.get(0)?,
                        email_notifications: row.get(1)?,
                        push_notifications: row.get(2)?,
                        mentions_only: row.get(3)?,
                        message_sound: row.get(4)?,
                        join_leave_sound: row.get(5)?,
                        allow_direct_messages: row.get(6)?,
                        allow_friend_requests: row.get(7)?,
                        show_presence: row.get(8)?,
                        developer_mode: row.get(9)?,
                        reduced_motion: row.get(10)?,
                        // A damaged preference must not make the whole private
                        // account unreadable.
                        muted_channel_ids: serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default(),
                        vad_sensitivity: row.get(12)?,
                        join_sound_id: row.get(13)?,
                        mention_popups: row.get(14)?,
                        call_ringtone: row.get(15)?,
                    })
                },
            )
            .optional()?;
        Ok(found.unwrap_or_else(Preferences::defaults))
    }
}

impl Account {
    /// The account as its owner sees it, from the row and the preferences.
    pub(crate) fn of(user: User, preferences: Preferences) -> Account {
        Account {
            id: user.id,
            username: user.username,
            display_name: user.display_name,
            email: user.email,
            created_at: user.created_at.to_unspecified_wire_text(),
            avatar_url: user.avatar_url,
            banner_url: user.banner_url,
            wallpaper_url: user.wallpaper_url,
            custom_status: user.custom_status.unwrap_or_default(),
            bio: user.bio.unwrap_or_default(),
            pronouns: user.pronouns.unwrap_or_default(),
            preferences,
        }
    }

    /// The row and the preferences of an account, read together.
    pub(crate) fn load(connection: &Connection, user: Id) -> rusqlite::Result<Option<Account>> {
        let Some(row) = User::find(connection, user)? else {
            return Ok(None);
        };
        let preferences = Preferences::load(connection, user)?;
        Ok(Some(Account::of(row, preferences)))
    }
}

pub async fn read(State(state): State<AppState>, caller: Caller) -> ApiResult<Response> {
    let account = state
        .database()
        .read(move |connection| -> Result<_, DbError> { Ok(Account::load(connection, caller.user)?) })
        .await?;
    let account = account.ok_or_else(|| ApiError::internal("the signed-in account is not there"))?;
    Ok(json_response(StatusCode::OK, &account))
}
