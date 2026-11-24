//! The two calls after the callback, as descriptions the application sends
//! with its own client, and the parsing of what comes back.

use std::fmt;

use serde_json::{Map, Value};

use crate::MAX_DOCUMENT;
use crate::discovery::Discovery;
use crate::flow::Flow;
use crate::form;
use crate::options::Provider;

/// A POST of a form to the token endpoint.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenRequest {
    pub url: String,
    /// `application/x-www-form-urlencoded`, already encoded.
    pub body: String,
}

impl TokenRequest {
    pub const CONTENT_TYPE: &'static str = "application/x-www-form-urlencoded";
}

impl fmt::Debug for TokenRequest {
    // The body holds the client secret and the code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenRequest")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

/// Redeems `code`. The client proves itself by sending its secret in the
/// body, and the PKCE verifier proves the redeemer started the sign-in.
pub fn token_request(
    discovery: &Discovery,
    provider: &Provider,
    flow: &Flow,
    code: &str,
) -> TokenRequest {
    TokenRequest {
        url: discovery.token_endpoint.clone(),
        body: form::encode(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &flow.redirect_uri),
            ("client_id", provider.client_id()),
            ("client_secret", provider.client_secret()),
            ("code_verifier", &flow.code_verifier),
        ]),
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TokenResponse {
    pub id_token: String,
    pub access_token: String,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenResponse").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    TooLarge,
    NotJson,
    /// The provider refused the request and said why.
    Provider {
        error: String,
        description: Option<String>,
    },
    /// A failing status without an error the provider named.
    Status(u16),
    /// No `id_token`, or one that is not text.
    MissingIdToken,
    /// No `access_token`, or one that is not text. An answer without one is refused.
    MissingAccessToken,
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("the token response is too large"),
            Self::NotJson => f.write_str("the token response is not a JSON object"),
            Self::Provider { error, .. } => write!(f, "the provider refused the code: {error}"),
            Self::Status(status) => write!(f, "the token endpoint answered {status}"),
            Self::MissingIdToken => f.write_str("the token response holds no id_token"),
            Self::MissingAccessToken => f.write_str("the token response holds no access_token"),
        }
    }
}

impl std::error::Error for TokenError {}

fn object(body: &[u8]) -> Result<Map<String, Value>, TokenError> {
    if body.len() > MAX_DOCUMENT {
        return Err(TokenError::TooLarge);
    }
    match serde_json::from_slice(body) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(TokenError::NotJson),
    }
}

/// Reads the answer of the token endpoint. A provider's own error is named
/// whatever the status; a failing status without one is only a status.
pub fn parse_token_response(status: u16, body: &[u8]) -> Result<TokenResponse, TokenError> {
    let parsed = object(body);
    if let Ok(map) = &parsed
        && let Some(error) = map.get("error").and_then(Value::as_str)
    {
        return Err(TokenError::Provider {
            error: error.chars().take(256).collect(),
            description: map
                .get("error_description")
                .and_then(Value::as_str)
                .map(|d| d.chars().take(1024).collect()),
        });
    }
    if !(200..300).contains(&status) {
        return Err(TokenError::Status(status));
    }
    let map = parsed?;
    let id_token = match map.get("id_token") {
        Some(Value::String(token)) if !token.is_empty() => token.clone(),
        _ => return Err(TokenError::MissingIdToken),
    };
    let access_token = match map.get("access_token") {
        Some(Value::String(token)) if !token.is_empty() => token.clone(),
        _ => return Err(TokenError::MissingAccessToken),
    };
    Ok(TokenResponse {
        id_token,
        access_token,
    })
}

/// A GET of the userinfo endpoint with the access token as a bearer.
#[derive(Clone, PartialEq, Eq)]
pub struct UserinfoRequest {
    pub url: String,
    /// The value of the `Authorization` header.
    pub authorization: String,
}

impl fmt::Debug for UserinfoRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserinfoRequest")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserinfoError {
    /// The provider has a userinfo endpoint and sent no access token to ask it with.
    NoAccessToken,
    Status(u16),
    TooLarge,
    /// The answer is a signed token, which this crate does not read.
    SignedAnswer,
    /// The content type is missing or is neither JSON nor a signed token.
    WrongContentType,
    NotJson,
    /// No `sub`, or one that is not the ID token's subject.
    SubjectMismatch,
}

impl fmt::Display for UserinfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the userinfo answer is refused: {self:?}")
    }
}

impl std::error::Error for UserinfoError {}

/// The request for the profile, when the provider has an endpoint for it.
/// `Ok(None)` means there is nothing to ask.
pub fn userinfo_request(
    discovery: &Discovery,
    access_token: Option<&str>,
) -> Result<Option<UserinfoRequest>, UserinfoError> {
    let Some(url) = &discovery.userinfo_endpoint else {
        return Ok(None);
    };
    let token = access_token
        .filter(|t| !t.is_empty())
        .ok_or(UserinfoError::NoAccessToken)?;
    Ok(Some(UserinfoRequest {
        url: url.clone(),
        authorization: format!("Bearer {token}"),
    }))
}

/// Reads the userinfo answer. Whoever it describes must be the person the ID
/// token names, compared exactly; an answer about someone else is refused
/// outright. Only `application/json` is read.
pub fn parse_userinfo(
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
    subject: &str,
) -> Result<Map<String, Value>, UserinfoError> {
    if !(200..300).contains(&status) {
        return Err(UserinfoError::Status(status));
    }
    if body.len() > MAX_DOCUMENT {
        return Err(UserinfoError::TooLarge);
    }
    let media = content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if media.eq_ignore_ascii_case("application/jwt") {
        return Err(UserinfoError::SignedAnswer);
    }
    if !media.eq_ignore_ascii_case("application/json") {
        return Err(UserinfoError::WrongContentType);
    }
    let Ok(Value::Object(map)) = serde_json::from_slice(body) else {
        return Err(UserinfoError::NotJson);
    };
    if map.get("sub").and_then(Value::as_str) != Some(subject) {
        return Err(UserinfoError::SubjectMismatch);
    }
    Ok(map)
}
