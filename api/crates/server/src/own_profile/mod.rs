//! What a person keeps for themselves on their own account: how their profile
//! reads, their preferences, and the sound that plays when they join a voice
//! room.
//!
//! `PATCH /api/me`, `PATCH /api/me/preferences` and `GET /api/me/join-sound`.
//! The account itself is read at `GET /api/me` (see `accounts`); the password,
//! the pictures and the settings the client stores are other groups'.
//!
//! A person's preferences are one row that is made the first time something
//! writes to it ([`ensure_preferences`]); an account that never saved any has
//! the defaults.

mod join_sound;
mod preferences;
mod profile;

use rusqlite::Connection;

use crate::http::Endpoints;
use crate::values::Id;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .patch("/api/me", profile::update)
        .json_body()
        .authorized()
        .patch("/api/me/preferences", preferences::update)
        .json_body()
        .authorized()
        .get("/api/me/join-sound", join_sound::library)
        .authorized()
}

/// Makes the person's row of preferences, with what an account that never
/// saved any has, unless it is there already. The sections the client keeps
/// start out as none.
pub(crate) fn ensure_preferences(db: &Connection, user: Id) -> rusqlite::Result<()> {
    db.execute(
        "INSERT OR IGNORE INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\",
            \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\",
            \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\",
            \"MutedChannelIdsJson\", \"VadSensitivity\", \"JoinSoundId\", \"AppearanceSettingsJson\", \"LanguageSettingsJson\",
            \"KeybindSettingsJson\", \"VoiceSettingsJson\", \"PrivacySettingsJson\")
         VALUES (?1, 1, 1, 1, 0, 0, 1, 1, 1, 1, 1, 1, 0, 0, '[]', 60, NULL, NULL, NULL, NULL, NULL, NULL)",
        [user],
    )?;
    Ok(())
}
