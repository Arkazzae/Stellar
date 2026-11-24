//! The mail the application sends: a message builder and an SMTP client.
//!
//! [`Message`] is what the application hands over: addresses, a subject, a
//! text and an HTML body and the images the HTML shows through `cid:`.
//! [`send`] puts it together (non-ASCII text in headers and bodies encoded,
//! the multipart structure, short lines) and delivers it to a server as the
//! [`Connection`] says. The caller passes the rustls client configuration:
//! where the trusted roots come from is its decision.
//!
//! A failure is an [`Error`] that says whether trying again can help.

mod address;
mod auth;
mod client;
mod encode;
mod error;
mod message;

pub use address::Mailbox;
pub use client::{Connection, Delivery, Options, Security, send, send_over};
pub use error::{Error, ErrorKind, Stage};
pub use message::{Encoded, InlineImage, Message};
