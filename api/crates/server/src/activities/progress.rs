//! Private saves belong to an account and a published activity across releases and reinstalls.
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::{
    http::{ApiError, ApiResult},
    state::AppState,
    values::Id,
};

pub(super) struct ProgressKey {
    pub user: Id,
    pub source: String,
    pub activity: String,
}

impl ProgressKey {
    fn load(&self, db: &Connection) -> ApiResult<(i64, Value)> {
        let stored: Option<(i64, String)> = db.query_row(
            "SELECT Revision, Data FROM ActivityProgress WHERE UserId=?1 AND Source=?2 AND Activity=?3",
            params![self.user, self.source, self.activity], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        match stored {
            Some((revision, data)) => Ok((
                revision,
                serde_json::from_str(&data).map_err(ApiError::internal)?,
            )),
            None => Ok((0, Value::Null)),
        }
    }

    pub async fn run(self, state: &AppState, action: Value) -> ApiResult<Value> {
        if action["op"] == "loadProgress" {
            return state
                .database()
                .read(move |db| {
                    let (revision, data) = self.load(db)?;
                    Ok(json!({ "revision": revision, "data": data }))
                })
                .await;
        }
        let expected = action["expected"]
            .as_i64()
            .ok_or_else(ApiError::bad_request)?;
        let data = &action["data"];
        let serialized = serde_json::to_string(data).map_err(ApiError::internal)?;
        state.database().write(move |tx| {
            let (revision, previous) = self.load(tx)?;
            if revision != expected {
                // A lost acknowledgement may replay the same conditional replacement.
                if revision == expected + 1 && previous == action["data"] {
                    return Ok(json!({ "revision": revision }));
                }
                return Err(ApiError::conflict());
            }
            let next = revision + 1;
            tx.execute(
                "INSERT INTO ActivityProgress(UserId, Source, Activity, Revision, Data) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(UserId, Source, Activity) DO UPDATE SET Revision=excluded.Revision, Data=excluded.Data",
                params![self.user, self.source, self.activity, next, serialized],
            )?;
            Ok(json!({ "revision": next }))
        }).await
    }
}
