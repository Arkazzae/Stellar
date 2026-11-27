//! Rewriting a message and removing it.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use stellarr_messages::MessageKey;

use super::dto::{self, MessageDeletedDto};
use super::{content_limit, field_problem, trimmed, units};
use crate::auth::Caller;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Json, Path, Problem, json_response};
use crate::link_previews::{self, PreviewRequest};
use crate::permissions::authorization::{self, Scope};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_MESSAGES;
use crate::permissions::visibility;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

#[derive(Debug, Deserialize)]
pub struct EditMessageRequest {
    content: Option<String>,
}

/// The channel of a message of the server, and who wrote it, unless it is a
/// notice of the server, which nobody may rewrite or remove.
fn find(
    db: &rusqlite::Connection,
    server: Id,
    message: Id,
) -> rusqlite::Result<Option<(Id, Id, String)>> {
    let found: Option<(Id, Id, String, bool)> = db
        .query_row(
            "SELECT m.\"ChannelId\", m.\"AuthorId\", m.\"Content\", m.\"IsSystem\" FROM \"Messages\" m
             WHERE m.\"Id\" = ?1 AND EXISTS (SELECT 1 FROM \"Channels\" c WHERE c.\"Id\" = m.\"ChannelId\" AND c.\"ServerId\" = ?2)",
            params![message, server],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    Ok(found.and_then(|(channel, author, content, system)| {
        (!system).then_some((channel, author, content))
    }))
}

/// Rewrites a message. Only its author may: removing what somebody said is one
/// act, putting new words under their name is another. Anyone else, moderators
/// included, and any server notice, is answered with 404, so that probing
/// cannot tell which ids exist. So is an author who no longer views the
/// channel the message is in.
pub async fn edit(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(message): Path<Id>,
    Json(request): Json<EditMessageRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let content = trimmed(request.content.as_deref().unwrap_or("")).to_string();
    let length = units(&content);
    let max = content_limit(&content, length);
    if content.is_empty() || length > max {
        return Err(field_problem(
            "content",
            &messages,
            MessageKey::MessageEditContentInvalid,
            &[("max", &max)],
        ));
    }

    let user = caller.user;
    let (channel, changed, shared, own) = state
        .database()
        .write(move |tx| -> Result<_, ApiError> {
            let Some((channel, author, stored)) = find(tx, server, message)? else {
                return Err(ApiError::not_found());
            };
            if author != user || !visibility::views(tx, user, channel)? {
                return Err(ApiError::not_found());
            }
            // A poll is not edited after it is sent, its words included:
            // people voted on what it said.
            if crate::polls::exists(tx, message)? {
                return Err(match messages.get(MessageKey::PollNotEditable, &[]) {
                    Ok(title) => Problem::new(StatusCode::CONFLICT)
                        .title(title)
                        .with("code", "poll-not-editable")
                        .into(),
                    Err(error) => error.into(),
                });
            }
            // Saving the same text twice is not an edit, and "edited" is a
            // claim readers rely on.
            let changed = stored != content;
            if changed {
                tx.execute(
                    "UPDATE \"Messages\" SET \"Content\" = ?1, \"EditedAt\" = ?2 WHERE \"Id\" = ?3",
                    params![content, Timestamp::now(), message],
                )?;
            }
            // The event is shared by everyone who sees the channel, so it
            // cannot say which reactions are the viewer's own; clients keep
            // that from history.
            let loaded = |viewer| -> Result<_, ApiError> {
                dto::load(tx, &messages, &[message], viewer)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| ApiError::internal("a stored message was not found again"))
            };
            Ok((channel, changed, loaded(None)?, loaded(Some(user))?))
        })
        .await?;

    let audience = state
        .database()
        .read(move |db| -> Result<Audience, ApiError> {
            Ok(visibility::audience_of(db, server, channel)?)
        })
        .await?;
    state
        .events()
        .publish(Event::new(audience, "MessageUpdated", &shared));

    // The new text may have gained links, or lost the ones it had cards for.
    if changed {
        state
            .previews()
            .enqueue(PreviewRequest { channel, message });
    }
    Ok(json_response(StatusCode::OK, &own))
}

/// Removes a message. Its author may, and so may whoever can manage messages
/// in that channel; everyone else is answered with 404, and so is anybody who
/// does not view the channel, its author included.
pub async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(message): Path<Id>,
) -> ApiResult<Response> {
    let user = caller.user;
    let (channel, thumbnails) = state
        .database()
        .write(move |tx| -> Result<_, ApiError> {
            let Some((channel, author, _)) = find(tx, server, message)? else {
                return Err(ApiError::not_found());
            };
            // Its author needs to see the channel; anybody else to manage
            // messages there, which nobody holds where they do not see.
            let allowed = if author == user {
                visibility::views(tx, user, channel)?
            } else {
                authorization::allows(tx, user, &Scope::channel(server, channel), MANAGE_MESSAGES)?
            };
            if !allowed {
                return Err(ApiError::not_found());
            }
            // The attachment links go with the message; the stored files stay,
            // because another message may use them. The preview images belong
            // to the message's embeds alone, so they are removed with it.
            let thumbnails = link_previews::clear_embeds(tx, message)?;
            tx.execute("DELETE FROM \"Messages\" WHERE \"Id\" = ?1", [message])?;
            Ok((channel, thumbnails))
        })
        .await?;
    link_previews::discard_thumbnails(&state, thumbnails).await;

    let audience = state
        .database()
        .read(move |db| -> Result<Audience, ApiError> {
            Ok(visibility::audience_of(db, server, channel)?)
        })
        .await?;
    state.events().publish(Event::new(
        audience,
        "MessageDeleted",
        &MessageDeletedDto {
            message_id: message,
            channel_id: channel,
        },
    ));
    Ok(StatusCode::NO_CONTENT.into_response())
}
