//! The mail a mention or a direct message causes. It is deliberately generic:
//! no message body, sender name, server name or channel name leaves the
//! instance, and the only link is the public entry point. The app has no
//! stable channel or conversation permalink, so a deep link would be a guess.
//! For the same reason the header shows the wordmark without the instance's
//! name, which is the name of its first server.

use stellarr_messages::{MessageKey, Messages};
use stellarr_smtp::InlineImage;

use crate::error::Error;
use crate::layout::MailLayout;
use crate::{guid, images};
use stellarr_smtp::Message;

pub fn mention(
    from: &str,
    to: &str,
    public_url: &str,
    delivery_id: u128,
    messages: &Messages,
) -> Result<Message, Error> {
    build(
        from,
        to,
        public_url,
        delivery_id,
        [
            messages.get(MessageKey::NotificationMentionSubject, &[])?,
            messages.get(MessageKey::NotificationMentionPushTitle, &[])?,
            messages.get(MessageKey::NotificationMentionBody, &[])?,
        ],
        images::mention(),
        messages,
    )
}

pub fn direct_message(
    from: &str,
    to: &str,
    public_url: &str,
    delivery_id: u128,
    messages: &Messages,
) -> Result<Message, Error> {
    build(
        from,
        to,
        public_url,
        delivery_id,
        [
            messages.get(MessageKey::NotificationDirectMessageSubject, &[])?,
            messages.get(MessageKey::NotificationDirectMessagePushTitle, &[])?,
            messages.get(MessageKey::NotificationDirectMessageBody, &[])?,
        ],
        images::direct_message(),
        messages,
    )
}

fn build(
    from: &str,
    to: &str,
    public_url: &str,
    delivery_id: u128,
    [subject, heading, body]: [String; 3],
    sticker: InlineImage,
    messages: &Messages,
) -> Result<Message, Error> {
    let url = format!("{}/", public_url.trim_end_matches('/'));
    let footer = messages.get(MessageKey::NotificationEmailFooter, &[])?;
    let text = [body.as_str(), "", &url, "", "-- ", &footer].join("\n");

    let layout = MailLayout::new(messages.language().code())
        .preheader(&body)
        .illustration(sticker)
        .heading(&heading)
        .paragraph(&body)
        .button(
            &url,
            &messages.get(MessageKey::NotificationOpenButton, &[])?,
        )
        .footer(&footer);
    // Stable for the row, so a second attempt is the same message to the
    // provider even though SMTP itself promises no exactly-once.
    crate::assemble(
        from,
        to,
        subject,
        text,
        layout,
        format!("activity-notification/{}", guid(delivery_id)),
    )
}
