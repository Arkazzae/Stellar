//! Resolve an opaque push delivery for its recipient using current access.
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::auth::Caller;
use crate::http::{json_response, ApiError, ApiResult, Path};
use crate::messages::dto::{self, MessageDto};
use crate::permissions::visibility::{self, ChannelKind};
use crate::state::AppState;
use crate::values::Id;

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Target {
    Channel { server_id: Id, message: Box<MessageDto> },
    Dm { conversation_id: Id },
}

pub(super) async fn resolve(
    State(state): State<AppState>,
    caller: Caller,
    Path(delivery): Path<Id>,
) -> ApiResult<Response> {
    let user = caller.user;
    let messages = *state.messages();
    let target = state.database().read(move |db| -> Result<Target, ApiError> {
        let (source, kind): (Id, String) = db.query_row(
            r#"SELECT "SourceMessageId", "EventKind" FROM "NotificationDeliveries"
               WHERE "Id" = ?1 AND "RecipientId" = ?2 AND "Channel" = 'push'"#,
            params![delivery, user], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?.ok_or_else(ApiError::not_found)?;
        match super::outbox::Kind::parse(&kind) {
            Some(super::outbox::Kind::DirectMessage) => {
                let conversation: Id = db.query_row(
                    r#"SELECT "ConversationId" FROM "DirectMessages" WHERE "Id" = ?1"#,
                    [source], |row| row.get(0),
                ).optional()?.ok_or_else(ApiError::not_found)?;
                let peer = crate::direct::peer_in(db, conversation, user)?.ok_or_else(ApiError::not_found)?;
                if crate::social::is_blocked(db, user, peer)? { return Err(ApiError::not_found()); }
                Ok(Target::Dm { conversation_id: conversation })
            }
            Some(super::outbox::Kind::Mention) => {
                let (server, channel, author): (Id, Id, Id) = db.query_row(
                    r#"SELECT c."ServerId", m."ChannelId", m."AuthorId" FROM "Messages" m
                       JOIN "Channels" c ON c."Id" = m."ChannelId" WHERE m."Id" = ?1"#,
                    [source], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ).optional()?.ok_or_else(ApiError::not_found)?;
                if visibility::find(db, user, server, channel, Some(ChannelKind::Text))?.is_none()
                    || crate::social::is_blocked(db, user, author)? {
                    return Err(ApiError::not_found());
                }
                let message = dto::load(db, &messages, &[source], Some(user))?.pop()
                    .ok_or_else(ApiError::not_found)?;
                Ok(Target::Channel { server_id: server, message: Box::new(message) })
            }
            None => Err(ApiError::not_found()),
        }
    }).await?;
    let mut response = json_response(StatusCode::OK, &target);
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
