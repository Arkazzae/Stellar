//! Link previews: the cards under a message that has links.
//!
//! A send or an edit puts the message in the [`PreviewQueue`]; the worker
//! reads each link, copies the preview picture into our own storage, replaces
//! the message's cards and tells whoever sees the channel.

mod builder;
mod queue;
mod thumbnails;
mod worker;

pub use queue::{PreviewQueue, PreviewRequest};
pub(crate) use thumbnails::NOBODY;
pub use thumbnails::{clear_embeds, discard_thumbnails};
pub use worker::{workers, workers_with};
