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

use crate::database::DbError;
use crate::state::AppState;

/// The whole of sending one message that a request waits on: looks at the
/// settings, lets `build` write the message with what it needs (the sender,
/// the address of the instance, the instance's name), and sends it. Says
/// [`Outcome::Off`] when the operator has switched mail off and
/// [`Outcome::NotConfigured`] when something it needs is missing, and builds
/// nothing then. A message that cannot be written is refused.
///
/// A caller that has switches of its own to check (recovery, invitations) asks
/// [`setup::find_for`] and looks at [`Mailing::switches`] first.
pub async fn deliver(
    state: &AppState,
    build: impl FnOnce(&Mailing) -> Result<stellarr_mail::Message, stellarr_mail::Error>,
) -> Result<Outcome, DbError> {
    let mailing = match setup::find_for(state).await? {
        MailSetup::Off => return Ok(Outcome::Off),
        MailSetup::NotConfigured => return Ok(Outcome::NotConfigured),
        MailSetup::Ready(mailing) => mailing,
    };
    match build(&mailing) {
        Ok(message) => Ok(state.mail().send(&message, &mailing.transport).await),
        Err(error) => Ok(Outcome::Refused {
            reason: error.to_string(),
        }),
    }
}
