//! The complete request that delivers one notification.
//!
//! The library does no network I/O. It describes the request, and the
//! application sends it with its own HTTP client: no redirects, no cookies, no
//! proxy, a socket opened only to a public address, and a deadline.

use serde::Serialize;

use crate::encrypt::{self, EncryptError};
use crate::outcome::Dispatch;
use crate::subscription::ValidatedSubscription;
use crate::vapid::{VapidError, VapidKeys};

/// The most plaintext one notification may carry. A larger one is not sent;
/// the service worker only needs a title, a line of text and a link.
pub const MAX_PAYLOAD_BYTES: usize = 3_072;
/// How long, in seconds, the push service holds a message for a browser that
/// is offline. Notifications older than this are no longer worth showing.
pub const TIME_TO_LIVE_SECONDS: u32 = 300;

/// The JSON a service worker receives for one notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushPayload {
    pub id: String,
    pub title: String,
    pub body: String,
    pub url: String,
    pub tag: String,
}

impl PushPayload {
    /// The bytes that are encrypted: compact JSON in UTF-8.
    pub fn to_json(&self) -> Vec<u8> {
        // Plain strings in a struct cannot fail to serialize.
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// How urgently a push service should wake the device (RFC 8030 section 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    VeryLow,
    Low,
    Normal,
    High,
}

impl Urgency {
    fn as_str(self) -> &'static str {
        match self {
            Self::VeryLow => "very-low",
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }
}

/// Delivery options. The defaults are what notifications use today: five
/// minutes of time to live, and neither urgency nor topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOptions {
    pub ttl_seconds: u32,
    pub urgency: Option<Urgency>,
    /// Replaces an earlier undelivered message with the same topic. Up to 32
    /// characters of the base64url alphabet (RFC 8030 section 5.4).
    pub topic: Option<String>,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self {
            ttl_seconds: TIME_TO_LIVE_SECONDS,
            urgency: None,
            topic: None,
        }
    }
}

/// A request for the application to send.
#[derive(Clone, PartialEq, Eq)]
pub struct PushRequest {
    /// Always `POST`.
    pub method: &'static str,
    /// The subscription's endpoint, as normalized.
    pub url: String,
    /// Header names and values. There is no `Content-Length`: the HTTP client
    /// that sends the request sets it from the body, and two are an error for
    /// some servers.
    pub headers: Vec<(String, String)>,
    /// The encrypted message.
    pub body: Vec<u8>,
}

impl std::fmt::Debug for PushRequest {
    // The Authorization value is a bearer token for the push service and the
    // body is the message; neither belongs in a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("PushRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &names)
            .finish_non_exhaustive()
    }
}

impl PushRequest {
    /// The first header of that name, compared without regard to case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Why no request could be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildError {
    /// The message is larger than [`MAX_PAYLOAD_BYTES`].
    PayloadTooLarge,
    /// The topic or another option is not allowed.
    InvalidOption,
    /// The receiver's key could not be used: the subscription is dead.
    Encryption(EncryptError),
    /// The token could not be signed.
    Signing(VapidError),
}

impl BuildError {
    /// What the caller that owns retries should do about it.
    ///
    /// A message that is too large is a problem of this notification, not of
    /// the browser, so the subscription stays. Unusable keys mean the
    /// subscription can never work, so it goes.
    pub fn dispatch(self) -> Dispatch {
        match self {
            Self::Encryption(EncryptError::InvalidReceiverKey) => Dispatch::Gone,
            _ => Dispatch::Retry,
        }
    }
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PayloadTooLarge => {
                f.write_str("the notification is larger than the payload limit")
            }
            Self::InvalidOption => f.write_str("a delivery option is not allowed"),
            Self::Encryption(error) => write!(f, "encryption failed: {error}"),
            Self::Signing(error) => write!(f, "signing failed: {error}"),
        }
    }
}

impl std::error::Error for BuildError {}

/// Builds the request that delivers `payload` (the bytes of the message,
/// usually [`PushPayload::to_json`]) to one validated subscription.
///
/// `now` is the time in seconds since the Unix epoch, used for the VAPID
/// token's expiry.
pub fn build_request(
    vapid: &VapidKeys,
    subscription: &ValidatedSubscription,
    payload: &[u8],
    options: &SendOptions,
    now: u64,
) -> Result<PushRequest, BuildError> {
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(BuildError::PayloadTooLarge);
    }
    if let Some(topic) = &options.topic
        && !valid_topic(topic)
    {
        return Err(BuildError::InvalidOption);
    }

    let body = encrypt::encrypt(payload, subscription.p256dh(), subscription.auth())
        .map_err(BuildError::Encryption)?;
    let authorization = vapid
        .authorization(subscription.audience(), now)
        .map_err(BuildError::Signing)?;

    let mut headers = vec![
        ("TTL".to_owned(), options.ttl_seconds.to_string()),
        ("Authorization".to_owned(), authorization),
        ("Content-Encoding".to_owned(), "aes128gcm".to_owned()),
        (
            "Content-Type".to_owned(),
            "application/octet-stream".to_owned(),
        ),
    ];
    if let Some(urgency) = options.urgency {
        headers.push(("Urgency".to_owned(), urgency.as_str().to_owned()));
    }
    if let Some(topic) = &options.topic {
        headers.push(("Topic".to_owned(), topic.clone()));
    }

    Ok(PushRequest {
        method: "POST",
        url: subscription.endpoint().to_owned(),
        headers,
        body,
    })
}

fn valid_topic(topic: &str) -> bool {
    !topic.is_empty()
        && topic.len() <= 32
        && topic
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base64url;
    use crate::testing::subscription_keys;
    use crate::vapid::generate_keys;

    fn setup() -> (VapidKeys, ValidatedSubscription) {
        let generated = generate_keys().unwrap();
        let vapid = VapidKeys::from_config(
            "mailto:push@example.test",
            &generated.public_key,
            &generated.private_key,
        )
        .unwrap();
        let keys = subscription_keys();
        let subscription = ValidatedSubscription::validate(
            Some("https://push.example.test/send/subscription"),
            Some(&keys.p256dh_text),
            Some(&keys.auth_text),
        )
        .unwrap();
        (vapid, subscription)
    }

    fn payload() -> PushPayload {
        PushPayload {
            id: "delivery-1".into(),
            title: "Visible title".into(),
            body: "Visible body".into(),
            url: "https://chat.example.test/".into(),
            tag: "delivery-1".into(),
        }
    }

    #[test]
    fn the_request_has_the_method_url_and_headers_a_push_service_needs() {
        let (vapid, subscription) = setup();
        let request = build_request(
            &vapid,
            &subscription,
            &payload().to_json(),
            &SendOptions::default(),
            1_000,
        )
        .unwrap();

        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "https://push.example.test/send/subscription");
        assert_eq!(request.header("TTL"), Some("300"));
        assert_eq!(request.header("Content-Encoding"), Some("aes128gcm"));
        assert_eq!(
            request.header("Content-Type"),
            Some("application/octet-stream")
        );
        assert!(request.header("Content-Length").is_none());
        assert!(request.header("Urgency").is_none());
        assert!(request.header("Topic").is_none());
        assert!(request.body.len() > 32);

        let authorization = request.header("authorization").unwrap();
        assert!(authorization.starts_with("vapid t="));
        assert!(authorization.ends_with(&format!(", k={}", vapid.public_key())));

        let text = String::from_utf8_lossy(&request.body);
        assert!(!text.contains("Visible title"));
        assert!(!text.contains("Visible body"));
    }

    #[test]
    fn the_token_is_addressed_to_the_origin_of_the_endpoint() {
        let (vapid, subscription) = setup();
        let request =
            build_request(&vapid, &subscription, b"{}", &SendOptions::default(), 1_000).unwrap();
        let authorization = request.header("Authorization").unwrap();
        let token = authorization
            .strip_prefix("vapid t=")
            .and_then(|rest| rest.split_once(','))
            .unwrap()
            .0;
        let claims = base64url::decode(token.split('.').nth(1).unwrap(), 400).unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&claims).unwrap();
        assert_eq!(claims["aud"], "https://push.example.test");
        assert_eq!(claims["exp"], 1_000 + 43_200);
        assert_eq!(claims["sub"], "mailto:push@example.test");
    }

    #[test]
    fn payload_json_uses_the_names_the_service_worker_reads() {
        let json = payload().to_json();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        let object = value.as_object().unwrap();
        let mut names: Vec<_> = object.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["body", "id", "tag", "title", "url"]);
        assert_eq!(value["title"], "Visible title");
    }

    #[test]
    fn urgency_and_topic_become_headers() {
        let (vapid, subscription) = setup();
        let options = SendOptions {
            ttl_seconds: 60,
            urgency: Some(Urgency::High),
            topic: Some("dm-0123_ABC-xyz".into()),
        };
        let request = build_request(&vapid, &subscription, b"{}", &options, 1).unwrap();
        assert_eq!(request.header("TTL"), Some("60"));
        assert_eq!(request.header("Urgency"), Some("high"));
        assert_eq!(request.header("Topic"), Some("dm-0123_ABC-xyz"));
    }

    #[test]
    fn a_topic_outside_the_rules_is_refused() {
        let (vapid, subscription) = setup();
        for topic in [
            "",
            &"a".repeat(33),
            "has space",
            "line\nbreak",
            "caf\u{e9}",
            "a/b",
            "a=b",
        ] {
            let options = SendOptions {
                topic: Some(topic.to_owned()),
                ..SendOptions::default()
            };
            assert_eq!(
                build_request(&vapid, &subscription, b"{}", &options, 1),
                Err(BuildError::InvalidOption),
                "{topic:?}"
            );
        }
        let ok = SendOptions {
            topic: Some("a".repeat(32)),
            ..SendOptions::default()
        };
        assert!(build_request(&vapid, &subscription, b"{}", &ok, 1).is_ok());
    }

    #[test]
    fn an_oversized_payload_is_not_built_and_keeps_the_subscription() {
        let (vapid, subscription) = setup();
        let at_limit = vec![b'x'; MAX_PAYLOAD_BYTES];
        assert!(
            build_request(&vapid, &subscription, &at_limit, &SendOptions::default(), 1).is_ok()
        );
        let over = vec![b'x'; MAX_PAYLOAD_BYTES + 1];
        let error =
            build_request(&vapid, &subscription, &over, &SendOptions::default(), 1).unwrap_err();
        assert_eq!(error, BuildError::PayloadTooLarge);
        assert_eq!(error.dispatch(), Dispatch::Retry);

        let big = PushPayload {
            body: "x".repeat(MAX_PAYLOAD_BYTES * 2),
            ..payload()
        };
        assert_eq!(
            build_request(
                &vapid,
                &subscription,
                &big.to_json(),
                &SendOptions::default(),
                1
            ),
            Err(BuildError::PayloadTooLarge)
        );
    }

    #[test]
    fn an_unusable_receiver_key_means_gone() {
        assert_eq!(
            BuildError::Encryption(EncryptError::InvalidReceiverKey).dispatch(),
            Dispatch::Gone
        );
        assert_eq!(
            BuildError::Encryption(EncryptError::Crypto).dispatch(),
            Dispatch::Retry
        );
    }

    #[test]
    fn debug_of_a_request_hides_the_token_the_key_and_the_body() {
        let (vapid, subscription) = setup();
        let request =
            build_request(&vapid, &subscription, b"{}", &SendOptions::default(), 1).unwrap();
        let shown = format!("{request:?}");
        let authorization = request.header("Authorization").unwrap();
        let token = authorization.split(['=', ',']).nth(1).unwrap();
        assert!(token.starts_with("eyJ"));
        assert!(!shown.contains(token));
        assert!(!shown.contains(authorization));
        assert!(!shown.contains(&vapid.private_key()));
        assert!(!shown.contains(&base64url::encode(&request.body)));
        assert!(shown.contains("https://push.example.test/send/subscription"));
        assert!(shown.contains("Authorization"));
        assert!(shown.contains("TTL"));
    }
}
