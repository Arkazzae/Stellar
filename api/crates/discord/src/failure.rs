//! What went wrong talking to Discord, in terms an import can act on.

use std::fmt;
use std::time::Duration;

/// The kinds of failure an import tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FailureKind {
    /// The token is wrong, revoked or not a bot's (HTTP 401). Stop using it:
    /// Discord shuts out an address after too many refused requests.
    InvalidToken,
    /// The bot may not see this server or channel, or lacks a permission the
    /// read needs (HTTP 403). [`DiscordError::discord_code`] says which:
    /// 50001 for no access, 50013 for a missing permission.
    MissingAccess,
    /// No such server, channel, message, template or emoji (HTTP 404).
    UnknownResource,
    /// The app lacks a privileged intent the read needs, which its owner
    /// turns on in the Developer Portal, on the Bot page.
    MissingIntent,
    /// Discord asked to wait longer than the reader may wait, or kept
    /// refusing after the reader waited; [`DiscordError::retry_after`] says
    /// how long Discord asked for.
    RateLimited,
    /// Discord could not be reached, did not answer in time, or failed itself
    /// (HTTP 5xx).
    Unreachable,
    /// Discord answered with a status or a body the reader does not
    /// understand.
    UnexpectedResponse,
    /// An id handed to the reader is not a snowflake. Nothing was sent.
    InvalidArgument,
}

/// A request to Discord that did not give what it was asked for.
///
/// The message names the route with its ids, the status and Discord's own
/// message, and never the token: requests carry it in a header that no
/// message or log line here prints, and Discord's text is checked for it.
#[derive(Clone, PartialEq)]
pub struct DiscordError {
    kind: FailureKind,
    message: String,
    route: String,
    status: Option<u16>,
    discord_code: Option<i32>,
    retry_after: Option<Duration>,
}

impl DiscordError {
    pub(crate) fn new(
        kind: FailureKind,
        message: String,
        route: String,
        status: Option<u16>,
        discord_code: Option<i32>,
        retry_after: Option<Duration>,
    ) -> DiscordError {
        DiscordError {
            kind,
            message,
            route,
            status,
            discord_code,
            retry_after,
        }
    }

    pub(crate) fn invalid_argument(what: &str) -> DiscordError {
        DiscordError {
            kind: FailureKind::InvalidArgument,
            message: format!("{what} is not a Discord id: a snowflake is up to 20 digits"),
            route: String::new(),
            status: None,
            discord_code: None,
            retry_after: None,
        }
    }

    pub fn kind(&self) -> FailureKind {
        self.kind
    }

    /// The method and path that failed, such as `GET guilds/1/members`.
    pub fn route(&self) -> &str {
        &self.route
    }

    pub fn status(&self) -> Option<u16> {
        self.status
    }

    /// Discord's JSON error code, such as 50001 or 10003, when it gave one.
    pub fn discord_code(&self) -> Option<i32> {
        self.discord_code
    }

    /// For [`FailureKind::RateLimited`], how long Discord asked to wait.
    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// The same failure with another kind and message, for a refusal that the
    /// caller can explain better than the transport can.
    pub(crate) fn explained(&self, kind: FailureKind, message: String) -> DiscordError {
        DiscordError {
            kind,
            message,
            ..self.clone()
        }
    }
}

impl fmt::Display for DiscordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// The same as the message: nothing in an error is secret, and nothing secret
/// is in it, but a derived form would invite the day somebody adds a field.
impl fmt::Debug for DiscordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscordError")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .field("route", &self.route)
            .field("status", &self.status)
            .field("discord_code", &self.discord_code)
            .field("retry_after", &self.retry_after)
            .finish()
    }
}

impl std::error::Error for DiscordError {}

/// The result type of the readers.
pub type Result<T> = std::result::Result<T, DiscordError>;
