//! One installation covers the whole instance. Count the viewer's private
//! messages and unread mention entries across the channels they still see.
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::Connection;
use serde_json::json;

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiResult, json_response, no_store};
use crate::permissions::visibility;
use crate::social::{conversations, is_blocked};
use crate::state::AppState;
use crate::values::Id;

fn count(db: &Connection, user: Id) -> rusqlite::Result<i64> {
    let mut total = 0;
    for conversation in conversations::of_person(db, user)? {
        if !is_blocked(db, user, conversation.peer(user))? {
            total += conversations::unread(db, user, &conversation)?;
        }
    }
    let mut mentions = db.prepare_cached(
        r#"SELECT "TargetChannelId", COUNT(*) FROM "UserNotifications" n
           WHERE "UserId" = ?1 AND "ReadAt" IS NULL AND "Source" = 'system'
             AND "SourcePackageId" = 'stellarr.mention' AND "TargetChannelId" IS NOT NULL
             AND NOT EXISTS (SELECT 1 FROM "ChannelReads" r WHERE r."UserId" = n."UserId"
                 AND r."ChannelId" = n."TargetChannelId" AND r."LastReadAt" >= n."CreatedAt")
           GROUP BY "TargetChannelId""#,
    )?;
    for row in mentions.query_map([user], |row| {
        Ok((row.get::<_, Id>(0)?, row.get::<_, i64>(1)?))
    })? {
        let (channel, unread) = row?;
        if visibility::views(db, user, channel)? {
            total += unread;
        }
    }
    Ok(total)
}

pub async fn read(State(state): State<AppState>, caller: Caller) -> ApiResult<Response> {
    let unread = state
        .database()
        .read(move |db| -> Result<_, DbError> { Ok(count(db, caller.user)?) })
        .await?;
    Ok(no_store(json_response(
        StatusCode::OK,
        &json!({ "unread": unread }),
    )))
}
