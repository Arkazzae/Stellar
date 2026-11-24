//! The server row, and the shape of it clients see.

use rusqlite::{Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::values::{Id, Timestamp};

/// A server, as stored.
#[derive(Debug, Clone)]
pub struct Server {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub is_public: bool,
    pub icon_url: Option<String>,
    pub created_at: Timestamp,
}

/// The columns [`Server::from_row`] reads, in order, for a query that selects
/// them from `"Servers"`.
pub const COLUMNS: &str = "\"Id\", \"Name\", \"Description\", \"IsPublic\", \"IconUrl\", \"CreatedAt\"";

impl Server {
    /// A row selected with [`COLUMNS`].
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Server> {
        Ok(Server {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            is_public: row.get(3)?,
            icon_url: row.get(4)?,
            created_at: row.get(5)?,
        })
    }

    pub fn find(connection: &Connection, id: Id) -> rusqlite::Result<Option<Server>> {
        connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM \"Servers\" WHERE \"Id\" = ?1"),
                [id],
                Server::from_row,
            )
            .optional()
    }

    pub fn dto(&self) -> ServerDto {
        ServerDto {
            id: self.id,
            name: self.name.clone(),
            description: self.description.clone(),
            icon_url: self.icon_url.clone(),
            created_at: self.created_at,
            is_public: self.is_public,
        }
    }
}

/// A server as clients read it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDto {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub created_at: Timestamp,
    pub is_public: bool,
}
