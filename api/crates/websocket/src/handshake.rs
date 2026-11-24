//! The server's half of the opening handshake (RFC 6455, section 4.2).

use aws_lc_rs::digest;
use http::header::{
    CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE,
};
use http::request::Parts;
use http::{HeaderMap, HeaderValue, Method, Response, StatusCode, Version};
use hyper::upgrade::OnUpgrade;

use crate::base64;

/// The constant the accept value is derived with.
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// The `Sec-WebSocket-Accept` value for a client's key.
pub fn accept_key(key: &str) -> String {
    let mut input = Vec::with_capacity(key.len() + GUID.len());
    input.extend_from_slice(key.as_bytes());
    input.extend_from_slice(GUID.as_bytes());
    // SHA-1 is what the protocol fixes here. It proves only that the server
    // understood the handshake; nothing is signed with it.
    let hash = digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, &input);
    base64::encode(hash.as_ref())
}

/// Why a request is not a usable WebSocket opening handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeError {
    /// It does not ask for an upgrade at all.
    NotAnUpgrade,
    MethodNotGet,
    VersionNotHttp11,
    /// The client speaks a version of the protocol other than 13.
    UnsupportedVersion,
    /// The key is missing, repeated, or not the base64 of sixteen bytes.
    BadKey,
    /// The connection cannot be taken over (the server was not set up for it).
    NoUpgradeSupport,
}

impl HandshakeError {
    pub fn status(self) -> StatusCode {
        match self {
            Self::UnsupportedVersion => StatusCode::UPGRADE_REQUIRED,
            Self::MethodNotGet => StatusCode::METHOD_NOT_ALLOWED,
            Self::NoUpgradeSupport => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotAnUpgrade | Self::VersionNotHttp11 | Self::BadKey => StatusCode::BAD_REQUEST,
        }
    }

    /// A line of plain text for the body of the refusal.
    pub fn message(self) -> &'static str {
        match self {
            Self::UnsupportedVersion => "Unsupported WebSocket version",
            Self::MethodNotGet => "A WebSocket upgrade is a GET",
            Self::NoUpgradeSupport => "This connection cannot be upgraded",
            Self::NotAnUpgrade => "Not a WebSocket upgrade",
            Self::VersionNotHttp11 => "A WebSocket upgrade needs HTTP/1.1",
            Self::BadKey => "Bad Sec-WebSocket-Key",
        }
    }

    /// The response a server sends instead of switching protocols; its body
    /// is [`message`](Self::message).
    pub fn response(self) -> Response<()> {
        let mut response = Response::new(());
        *response.status_mut() = self.status();
        if self == Self::UnsupportedVersion {
            // Tells the client which version it may retry with.
            response
                .headers_mut()
                .insert(SEC_WEBSOCKET_VERSION, HeaderValue::from_static("13"));
        }
        response
    }
}

fn has_token(headers: &HeaderMap, name: http::header::HeaderName, token: &str) -> bool {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|part| part.trim().eq_ignore_ascii_case(token))
}

/// True when the request asks to become a WebSocket: `Upgrade: websocket`
/// and a `Connection` header that lists `upgrade`.
pub fn is_upgrade_request(headers: &HeaderMap) -> bool {
    has_token(headers, UPGRADE, "websocket") && has_token(headers, CONNECTION, "upgrade")
}

/// Checks the handshake and, when it is good, returns the `101` response to
/// send. The connection is taken over with [`take_upgrade`] and
/// [`upgraded`](crate::upgraded).
///
/// No subprotocol is selected and no extension accepted, whatever the client
/// offers: the client then falls back to plain frames, which is all this
/// crate speaks.
pub fn accept(parts: &Parts) -> Result<Response<()>, HandshakeError> {
    if !is_upgrade_request(&parts.headers) {
        return Err(HandshakeError::NotAnUpgrade);
    }
    if parts.method != Method::GET {
        return Err(HandshakeError::MethodNotGet);
    }
    if parts.version != Version::HTTP_11 {
        return Err(HandshakeError::VersionNotHttp11);
    }
    let version = parts
        .headers
        .get(SEC_WEBSOCKET_VERSION)
        .and_then(|v| v.to_str().ok());
    if version.map(str::trim) != Some("13") {
        return Err(HandshakeError::UnsupportedVersion);
    }
    let mut keys = parts.headers.get_all(SEC_WEBSOCKET_KEY).iter();
    let key = match (keys.next(), keys.next()) {
        (Some(key), None) => key.to_str().map_err(|_| HandshakeError::BadKey)?.trim(),
        _ => return Err(HandshakeError::BadKey),
    };
    if base64::decode(key).is_none_or(|bytes| bytes.len() != 16) {
        return Err(HandshakeError::BadKey);
    }
    let accept = accept_key(key);

    let mut response = Response::new(());
    *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    let headers = response.headers_mut();
    headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
    headers.insert(CONNECTION, HeaderValue::from_static("Upgrade"));
    headers.insert(
        SEC_WEBSOCKET_ACCEPT,
        HeaderValue::from_str(&accept).expect("base64 is a valid header value"),
    );
    Ok(response)
}

/// Takes the handle that yields the raw connection once the response has
/// gone out. It can be taken once.
pub fn take_upgrade(parts: &mut Parts) -> Option<OnUpgrade> {
    parts.extensions.remove::<OnUpgrade>()
}
