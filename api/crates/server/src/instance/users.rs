//! The account row, and the two shapes of it clients see.

use rusqlite::{Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::values::{Id, Timestamp};

/// An account, as stored.
#[derive(Debug, Clone)]
pub struct User {
    pub id: Id,
    pub username: String,
    pub display_name: String,
    pub password_hash: Option<String>,
    pub email: Option<String>,
    pub is_disabled: bool,
    pub session_version: i64,
    pub security_recovery_required: bool,
    pub avatar_color: String,
    pub created_at: Timestamp,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    pub wallpaper_url: Option<String>,
    pub custom_status: Option<String>,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub origin: Option<String>,
}

const COLUMNS: &str = "\"Id\", \"Username\", \"DisplayName\", \"PasswordHash\", \"Email\", \"IsDisabled\", \"SessionVersion\",
    \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\", \"AvatarUrl\", \"BannerUrl\", \"WallpaperUrl\",
    \"CustomStatus\", \"Bio\", \"Pronouns\", \"Origin\"";

impl User {
    /// A row selected with [`User::columns`].
    pub(crate) fn from_row(row: &Row<'_>) -> rusqlite::Result<User> {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            display_name: row.get(2)?,
            password_hash: row.get(3)?,
            email: row.get(4)?,
            is_disabled: row.get(5)?,
            session_version: row.get(6)?,
            security_recovery_required: row.get(7)?,
            avatar_color: row.get(8)?,
            created_at: row.get(9)?,
            avatar_url: row.get(10)?,
            banner_url: row.get(11)?,
            wallpaper_url: row.get(12)?,
            custom_status: row.get(13)?,
            bio: row.get(14)?,
            pronouns: row.get(15)?,
            origin: row.get(16)?,
        })
    }

    /// The columns [`User::from_row`] reads, each taken from the table known as `alias`.
    pub(crate) fn columns(alias: &str) -> String {
        COLUMNS
            .split(", ")
            .map(|column| format!("{alias}.{}", column.trim()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn find(connection: &Connection, id: Id) -> rusqlite::Result<Option<User>> {
        connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM \"Users\" WHERE \"Id\" = ?1"),
                [id],
                User::from_row,
            )
            .optional()
    }

    /// The account a sign-in names, by user name or by e-mail address, which
    /// are both unique in capitals.
    pub fn find_by_login(
        connection: &Connection,
        normalized: &str,
    ) -> rusqlite::Result<Option<User>> {
        connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM \"Users\" WHERE \"NormalizedUsername\" = ?1 OR \"NormalizedEmail\" = ?1"),
                [normalized],
                User::from_row,
            )
            .optional()
    }

    /// What everybody may know of the account.
    pub fn dto(&self, presence: &str) -> UserDto {
        UserDto {
            id: self.id.to_string(),
            username: self.username.clone(),
            display_name: self.display_name.clone(),
            avatar_color: self.avatar_color.clone(),
            presence: presence.to_string(),
            avatar_url: self.avatar_url.clone(),
            banner_url: self.banner_url.clone(),
            custom_status: self.custom_status.clone(),
            bio: self.bio.clone(),
            pronouns: self.pronouns.clone(),
            origin: self.origin.clone(),
        }
    }
}

/// An account as other people see it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    /// A person's id, or `system` for the account that stands in for the system.
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub avatar_color: String,
    pub presence: String,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    pub custom_status: Option<String>,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub origin: Option<String>,
}

impl UserDto {
    /// The account that stands in for the system in a message it posts.
    pub fn system() -> UserDto {
        UserDto {
            id: "system".to_string(),
            username: "system".to_string(),
            display_name: "system".to_string(),
            avatar_color: "#777777".to_string(),
            presence: "online".to_string(),
            avatar_url: None,
            banner_url: None,
            custom_status: None,
            bio: None,
            pronouns: None,
            origin: None,
        }
    }
}
