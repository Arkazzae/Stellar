//! Link previews: the cards under a message that has links.
//!
//! A send or an edit puts the message in the [`PreviewQueue`]; the worker
//! reads each link, copies the preview picture into our own storage, replaces
//! the message's cards and tells whoever sees the channel.

mod queue;

pub use queue::{PreviewQueue, PreviewRequest};
