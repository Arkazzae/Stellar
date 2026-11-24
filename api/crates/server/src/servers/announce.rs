//! What joining a server and answering an invitation tell the hub.

use crate::database::DbError;
use crate::events::Event;
use crate::instance::welcome::{self, Welcome};
use crate::state::AppState;

/// Tells the members who are online and see the channel about the notice that
/// greets a newcomer, once it has been committed. The membership is already
/// there, so a notice that cannot be announced is logged and nothing more.
pub async fn welcomed(state: &AppState, welcome: Option<Welcome>) {
    let Some(welcome) = welcome else {
        return;
    };
    let announced = state
        .database()
        .read(move |connection| -> Result<Event, DbError> {
            Ok(welcome::announcement(connection, &welcome)?)
        })
        .await;
    match announced {
        Ok(event) => state.events().publish(event),
        Err(error) => tracing::warn!("a welcome could not be announced: {error}"),
    }
}
