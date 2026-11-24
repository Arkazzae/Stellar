//! The server side of WebSocket (RFC 6455) on a connection hyper has
//! upgraded: the opening handshake, frames, fragmentation, ping and pong, and
//! the closing handshake.
//!
//! Only what a server needs is here. There are no extensions, so there is no
//! compression, and the server never masks. A client must mask every frame,
//! as the standard requires.
//!
//! A connection is read through a [`Receiver`] and written through a
//! [`Sender`], which can be cloned and used from other tasks. Reading hands
//! out the pieces of a message as they arrive instead of buffering the whole
//! message, so the caller decides how much of it to keep.

mod base64;
pub mod frame;
mod handshake;
mod socket;
mod utf8;

pub use handshake::{HandshakeError, accept, accept_key, is_upgrade_request};
pub use socket::{CloseFrame, Config, Error, Event, MessageKind, Receiver, Sender, split, status};

use hyper::upgrade::{OnUpgrade, Upgraded};
use hyper_util::rt::TokioIo;

/// The raw connection of an upgraded HTTP request.
pub type Upgrade = TokioIo<Upgraded>;

/// Waits for the `101` response to go out and takes over the connection.
pub async fn upgraded(
    on_upgrade: OnUpgrade,
    config: Config,
) -> std::io::Result<(Sender<Upgrade>, Receiver<Upgrade>)> {
    let io = on_upgrade.await.map_err(std::io::Error::other)?;
    Ok(split(TokioIo::new(io), config))
}
