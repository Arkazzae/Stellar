//! Sending mail: the settings that say whether and how, one delivery service
//! for every kind of message, and the worker that sends the queue of security
//! alerts.
//!
//! The messages themselves are built by `stellarr_mail`. A kind of mail asks
//! [`setup::find`] for the sender, the address of the instance and the
//! transport, builds its message with those, and gives it to
//! [`MailDelivery::send`] (`state.mail()`), which says what became of it as an
//! [`Outcome`]. A request that waits on mail (a reset link, a code) answers
//! from the outcome; a queue (security alerts here, activity notifications in
//! their own outbox) records it on its row and tries again later.
//!
//! Nothing in this module logs a password, a key, a link or the text of a
//! message.

mod delivery;
pub mod security_alerts;
pub mod setup;

#[cfg(test)]
mod tests;

pub use delivery::{MailDelivery, Outcome, Wiring};
pub use setup::{MailSetup, Mailing, Switches};
