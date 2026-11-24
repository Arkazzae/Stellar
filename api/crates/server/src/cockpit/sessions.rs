//! The signed-in devices of the instance server's members.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use stellarr_messages::MessageKey;

use super::{no_content};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Path, json_response};
use crate::permissions::membership::InstanceOwner;
use crate::state::AppState;
use crate::values::{Guid, Id, Timestamp};

/// The most sessions the page lists.
const MOST_LISTED: i64 = 500;

/// What a desktop client says about itself in its user agent, as
/// `StellarrDesktop/version CEF/version Package/kind`. Every field is none for
/// any other client.
#[derive(Debug, Default, PartialEq, Eq)]
struct Client {
    name: Option<&'static str>,
    version: Option<String>,
    runtime: Option<String>,
    install_type: Option<String>,
}

fn describe(user_agent: &str) -> Client {
    let tokens: Vec<&str> = user_agent.split(' ').filter(|token| !token.is_empty()).collect();
    let value_of = |prefix: &str| {
        tokens
            .iter()
            .find_map(|token| token.strip_prefix(prefix))
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
    };
    match value_of("StellarrDesktop/") {
        None => Client::default(),
        Some(version) => Client {
            name: Some("Stellarr Desktop"),
            version: Some(version),
            runtime: value_of("CEF/").map(|cef| format!("CEF {cef}")),
            install_type: value_of("Package/"),
        },
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionDto {
    id: Id,
    user_id: Id,
    user_display_name: String,
    device: String,
    client_name: Option<&'static str>,
    client_version: Option<String>,
    client_runtime: Option<String>,
    install_type: Option<String>,
    ip_address: String,
    created_at: Timestamp,
    last_seen_at: Timestamp,
    expires_at: Timestamp,
    current: bool,
}

pub(super) async fn list(
    State(state): State<AppState>,
    caller: Caller,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Response> {
    let answer = state
        .database()
        .read(move |db| -> Result<Vec<SessionDto>, DbError> {
            let mut statement = db.prepare(
                "SELECT s.\"Id\", s.\"UserId\", u.\"DisplayName\", s.\"Device\", s.\"IpAddress\", s.\"CreatedAt\", s.\"LastSeenAt\", s.\"ExpiresAt\"
                 FROM \"UserSessions\" s JOIN \"Users\" u ON u.\"Id\" = s.\"UserId\"
                 WHERE s.\"RevokedAt\" IS NULL AND s.\"ExpiresAt\" > ?2
                   AND EXISTS (SELECT 1 FROM \"ServerMembers\" m WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" = s.\"UserId\")
                 ORDER BY s.\"LastSeenAt\" DESC LIMIT ?3",
            )?;
            let rows = statement
                .query_map(params![server, Timestamp::now(), MOST_LISTED], |row| {
                    let id: Id = row.get(0)?;
                    let device: String = row.get(3)?;
                    let client = describe(&device);
                    Ok(SessionDto {
                        id,
                        user_id: row.get(1)?,
                        user_display_name: row.get(2)?,
                        client_name: client.name,
                        client_version: client.version,
                        client_runtime: client.runtime,
                        install_type: client.install_type,
                        device,
                        ip_address: row.get(4)?,
                        created_at: row.get(5)?,
                        last_seen_at: row.get(6)?,
                        expires_at: row.get(7)?,
                        current: id == caller.session,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}

pub(super) async fn revoke(
    State(state): State<AppState>,
    Path(Guid(session)): Path<Guid>,
    caller: Caller,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Response> {
    if caller.session == session {
        return Err(ApiError::conflict());
    }
    let messages = *state.messages();
    state
        .database()
        .write(move |tx| -> Result<(), ApiError> {
            let found: Option<(Id, String)> = tx
                .query_row(
                    "SELECT s.\"UserId\", u.\"DisplayName\" FROM \"UserSessions\" s JOIN \"Users\" u ON u.\"Id\" = s.\"UserId\"
                     WHERE s.\"Id\" = ?1 AND s.\"RevokedAt\" IS NULL
                       AND EXISTS (SELECT 1 FROM \"ServerMembers\" m WHERE m.\"ServerId\" = ?2 AND m.\"UserId\" = s.\"UserId\")",
                    params![session, server],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((_, name)) = found else {
                return Err(ApiError::not_found());
            };
            tx.execute(
                "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1",
                params![session, Timestamp::now()],
            )?;
            crate::panel::record(
                tx,
                server,
                caller.user,
                messages.get(MessageKey::AuditSessionRevoked, &[("name", &name)])?,
            )?;
            Ok(())
        })
        .await?;
    Ok(no_content())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_client_says_what_it_is_in_its_user_agent() {
        let client = describe("Mozilla/5.0 StellarrDesktop/0.4.2 CEF/131.0 Package/appimage Safari");
        assert_eq!(client.name, Some("Stellarr Desktop"));
        assert_eq!(client.version.as_deref(), Some("0.4.2"));
        assert_eq!(client.runtime.as_deref(), Some("CEF 131.0"));
        assert_eq!(client.install_type.as_deref(), Some("appimage"));
        assert_eq!(describe("StellarrDesktop/1"), Client { name: Some("Stellarr Desktop"), version: Some("1".into()), ..Client::default() });
        assert_eq!(describe("Mozilla/5.0 Firefox"), Client::default());
        assert_eq!(describe("StellarrDesktop/"), Client::default());
        assert_eq!(describe(""), Client::default());
    }
}
