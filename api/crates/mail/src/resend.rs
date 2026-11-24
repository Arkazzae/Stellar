//! Sending through the Resend HTTP service, without the HTTP: the request
//! as data, and the reading of the answer.

use std::fmt;

use serde::Serialize;
use stellarr_smtp::{Mailbox, Message};

/// The most of a refusal's text that goes into the error.
const MAX_DETAIL: usize = 1000;

#[derive(Serialize)]
struct Body<'a> {
    from: String,
    to: [String; 1],
    subject: &'a str,
    html: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<Attachment<'a>>,
}

#[derive(Serialize)]
struct Attachment<'a> {
    content: String,
    filename: &'a str,
    content_type: &'a str,
    content_id: &'a str,
}

/// The call that hands a message to Resend: a POST of `json` to
/// `https://api.resend.com/emails` with these two headers, and a
/// `Content-Type: application/json`.
#[derive(Clone, PartialEq, Eq)]
pub struct ResendRequest {
    /// The value of the `Authorization` header: `Bearer <key>`.
    pub authorization: String,
    /// The value of the `Idempotency-Key` header: a retry of the same
    /// message must not be delivered twice.
    pub idempotency_key: String,
    /// The body.
    pub json: String,
}

// Printing a request must never print the key.
impl fmt::Debug for ResendRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResendRequest")
            .field("idempotency_key", &self.idempotency_key)
            .finish_non_exhaustive()
    }
}

impl ResendRequest {
    pub const URL: &'static str = "https://api.resend.com/emails";

    /// Fails without a key to send with, which is a setting that is
    /// missing, not something to ask Resend about.
    pub fn new(message: &Message, api_key: &str) -> Result<ResendRequest, ResendError> {
        if api_key.trim().is_empty() {
            return Err(ResendError::NoApiKey);
        }
        // Resend embeds an attachment that has a content id, and the HTML
        // shows it through cid: and that id.
        let attachments = message
            .images
            .iter()
            .map(|image| Attachment {
                content: base64(&image.content),
                filename: &image.file_name,
                content_type: &image.content_type,
                content_id: &image.content_id,
            })
            .collect();
        let body = Body {
            from: written(&message.from),
            to: [message.to.first().map(written).unwrap_or_default()],
            subject: &message.subject,
            html: &message.html,
            text: &message.text,
            attachments,
        };
        let json =
            serde_json::to_string(&body).map_err(|e| ResendError::Unwritable(e.to_string()))?;
        // The message id is `<key>@host`; the key is what makes it one message.
        let key = message
            .message_id
            .rsplit_once('@')
            .map_or(message.message_id.as_str(), |(key, _)| key);
        Ok(ResendRequest {
            authorization: format!("Bearer {api_key}"),
            idempotency_key: key.to_owned(),
            json,
        })
    }
}

/// Why Resend did not take a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResendError {
    /// Resend answered with a status that is not a success.
    Rejected { status: u16, detail: String },
    /// Resend answered with a success and no message id.
    NoMessageId,
    /// There is no API key to send with.
    NoApiKey,
    /// The request body could not be written as JSON.
    Unwritable(String),
}

impl fmt::Display for ResendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResendError::Rejected { status, detail } => {
                write!(f, "Resend rejected the message ({status}): {detail}")
            }
            ResendError::NoMessageId => f.write_str("Resend did not return a message id."),
            ResendError::NoApiKey => f.write_str("The Resend API key is not configured."),
            ResendError::Unwritable(why) => {
                write!(f, "The Resend request could not be written: {why}")
            }
        }
    }
}

impl std::error::Error for ResendError {}

/// The id Resend gave the message, from its answer.
pub fn read_resend_answer(status: u16, body: &[u8]) -> Result<String, ResendError> {
    if !(200..300).contains(&status) {
        let text = String::from_utf8_lossy(body);
        let detail: String = text.chars().take(MAX_DETAIL).collect();
        return Err(ResendError::Rejected { status, detail });
    }
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| ResendError::NoMessageId)?;
    match value.get("id") {
        Some(serde_json::Value::String(id)) => Ok(id.clone()),
        _ => Err(ResendError::NoMessageId),
    }
}

/// `Name <user@host>` for a mailbox with a name, the bare address without
/// one; a name with punctuation that means something in an address is quoted.
fn written(mailbox: &Mailbox) -> String {
    match mailbox.name() {
        None => mailbox.address().to_owned(),
        Some(name) => {
            let plain = name.chars().all(|c| {
                c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '\'' || c == '&'
            });
            let name = if plain {
                name.to_owned()
            } else {
                format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
            };
            format!("{name} <{}>", mailbox.address())
        }
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(char::from(ALPHABET[(n >> 18) as usize & 63]));
        out.push(char::from(ALPHABET[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[n as usize & 63])
        } else {
            '='
        });
    }
    out
}
