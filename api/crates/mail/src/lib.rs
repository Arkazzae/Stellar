//! The mail the server writes.
//!
//! Every kind of message is a function that takes what the message is about
//! and returns a [`Message`] for the `stellarr-smtp` crate: subject, a plain
//! text part, an HTML part in the shared dark layout and the inline images the
//! HTML shows. The message's id is `<key>@stellarr`, where the key is stable
//! for the thing the mail is about (a reset token, a delivery row), so a retry
//! is the same message to the provider.
//!
//! Which way a message leaves is [`MailTransport`]: SMTP, sent with
//! `stellarr_smtp::send`, or the Resend service, for which [`ResendRequest`]
//! describes the call and [`read_resend_answer`] reads the reply. This crate
//! does no network I/O.

mod account;
mod activity;
mod device;
mod error;
mod html;
mod images;
mod layout;
mod resend;
mod transport;

use std::time::SystemTime;

use stellarr_messages::{MessageKey, Messages, MissingValue};
use stellarr_smtp::Mailbox;
pub use stellarr_smtp::Message;

pub use account::{
    SecurityEvent, delivery_test, describe_event, email_change, email_code, event_kinds,
    password_reset, security_alert,
};
pub use activity::{direct_message, mention};
pub use device::describe_device;
pub use error::Error;
pub use resend::{ResendError, ResendRequest, read_resend_answer};
pub use transport::{DEFAULT_SMTP_PORT, MailSettings, MailTransport, Provider};

use layout::MailLayout;

/// The 128 bits of an id as 32 lowercase hexadecimal digits, the way ids
/// stand in a message key.
fn guid(id: u128) -> String {
    format!("{id:032x}")
}

/// The plain-text part's sign-off, after the conventional "-- " separator.
fn signature(
    instance_name: &str,
    public_url: &str,
    messages: &Messages,
) -> Result<String, MissingValue> {
    let sent_by = messages.get(
        MessageKey::MailSentFromInstance,
        &[("instance", &instance_name)],
    )?;
    Ok(if public_url.is_empty() {
        format!("\n-- \n{sent_by}")
    } else {
        format!("\n-- \n{sent_by}\n{public_url}")
    })
}

fn assemble(
    from: &str,
    to: &str,
    subject: String,
    text: String,
    layout: MailLayout,
    key: String,
) -> Result<Message, Error> {
    let (html, images) = layout.render(&subject);
    Ok(Message {
        from: Mailbox::parse(from)?,
        to: vec![Mailbox::parse(to)?],
        subject,
        html,
        text,
        message_id: format!("{key}@stellarr"),
        date: SystemTime::now(),
        images,
    })
}
