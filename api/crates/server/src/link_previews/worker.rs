//! Drains the preview queue: builds the cards of each message and tells the
//! members who see its channel when they are ready.
//!
//! One message at a time, on purpose. Unfurling is outbound traffic to hosts
//! we do not control, and a server that fans out on every posted link can be
//! aimed at somebody as a load generator. The message is on screen the moment
//! it is sent, and its cards arrive underneath a moment later.

use std::time::Duration;

use rusqlite::OptionalExtension;
use stellarr_embeds::Fetcher;
use tokio::time::{Instant, sleep_until};
use tracing::warn;

use super::builder;
use super::queue::PreviewRequest;
use crate::background::{Stop, Worker};
use crate::database::DbError;
use crate::events::Event;
use crate::http::AttemptLimiter;
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
    let unfurls = AttemptLimiter::new(20, Duration::from_secs(60));
    let mut import_ready = Instant::now();
    loop {
        let (request, imported) = tokio::select! {
            biased;
            () = stop.stopped() => return,
            request = state.previews().next() => (request, false),
            request = async {
                sleep_until(import_ready).await;
                state.previews().next_imported().await
            } => (request, true),
        };
        // One bad link must not take the worker down, or every later message
        // would silently lose its previews.
        if let Err(error) = process(&state, &fetcher, request, &unfurls, imported).await {
            warn!("the previews of a message could not be built: {error}");
        }
        if imported {
            import_ready = Instant::now() + Duration::from_secs(1);
        }
    }
}

async fn process(
    state: &AppState,
    fetcher: &Fetcher,
    request: PreviewRequest,
    unfurls: &AttemptLimiter,
    imported: bool,
) -> Result<(), DbError> {
    let Some(embeds) = builder::build(state, fetcher, request.message, unfurls, imported).await?
    else {
        return Ok(());
    };
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
