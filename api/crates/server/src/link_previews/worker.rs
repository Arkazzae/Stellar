//! Drains the preview queue: builds the cards of each message and tells the
//! members who see its channel when they are ready.
//!
//! One message at a time, on purpose. Unfurling is outbound traffic to hosts
//! we do not control, and a server that fans out on every posted link can be
//! aimed at somebody as a load generator. The message is on screen the moment
//! it is sent, and its cards arrive underneath a moment later.

use rusqlite::OptionalExtension;
use stellarr_embeds::Fetcher;
use tracing::warn;

use super::builder;
use super::queue::PreviewRequest;
use crate::background::{Stop, Worker};
use crate::database::DbError;
use crate::events::Event;
use crate::messages::dto::MessageEmbedsDto;
use crate::permissions::visibility;
use crate::state::AppState;
use crate::values::Id;

/// The worker, fetching as the settings say.
pub fn workers() -> Vec<Worker> {
    vec![Worker::new("link-previews", |state, stop| async move {
        let fetcher = Fetcher::new(state.settings().embeds.clone());
        run(state, stop, fetcher).await;
    })]
}

/// The worker with a fetcher of the caller's own: a test points it at servers
/// of its own.
pub fn workers_with(fetcher: Fetcher) -> Vec<Worker> {
    vec![Worker::new("link-previews", move |state, stop| {
        let fetcher = fetcher.clone();
        async move { run(state, stop, fetcher).await }
    })]
}

async fn run(state: AppState, stop: Stop, fetcher: Fetcher) {
    loop {
        let request = tokio::select! {
            () = stop.stopped() => return,
            request = state.previews().next() => request,
        };
        // One bad link must not take the worker down, or every later message
        // would silently lose its previews.
        if let Err(error) = process(&state, &fetcher, request).await {
            warn!("the previews of a message could not be built: {error}");
        }
    }
}

async fn process(
    state: &AppState,
    fetcher: &Fetcher,
    request: PreviewRequest,
) -> Result<(), DbError> {
    let embeds = builder::build(state, fetcher, request.message).await?;
    let channel = request.channel;
    let audience = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            let server: Option<Id> = db
                .query_row(
                    "SELECT \"ServerId\" FROM \"Channels\" WHERE \"Id\" = ?1",
                    [channel],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(match server {
                Some(server) => Some(visibility::audience_of(db, server, channel)?),
                None => None,
            })
        })
        .await?;
    let Some(audience) = audience else {
        return Ok(());
    };
    // Sent even when nothing was built: after an edit, an empty set is how
    // clients learn that the old cards are gone.
    state.events().publish(Event::new(
        audience,
        "MessageEmbedsUpdated",
        &MessageEmbedsDto {
            message_id: request.message,
            channel_id: request.channel,
            embeds,
        },
    ));
    Ok(())
}
