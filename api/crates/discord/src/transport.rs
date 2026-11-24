//! Sends one GET request at a time to Discord's API and turns the answer into
//! a value or a [`DiscordError`], waiting out rate limits in between.
//!
//! One transport serves one token (or none, for templates), because Discord
//! counts limits per bot. Requests are meant to be sequential: an import reads
//! one page after another, and Discord's limits are per bot anyway, so a
//! parallel reader would only queue up behind the same buckets.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use http::{HeaderMap, HeaderValue};
use serde::Deserialize;
use stellarr_outbound::{Collected, Error as OutboundError, Request};

use crate::config::ClientConfig;
use crate::failure::{DiscordError, FailureKind, Result};
use crate::rate_limits::{self, RateLimits, Route};
use crate::token::BotToken;
use crate::wire::{Obj, WireError, nests_deeper_than};

/// The form Discord requires, without which it may refuse the request.
pub(crate) const USER_AGENT_TEXT: &str = "DiscordBot (https://github.com/Arkazzae/Stellarr, 2.0)";

/// More than any error body Discord sends; a longer one is cut, not read whole.
const MAX_ERROR_BYTES: usize = 64 * 1024;

/// How deep Discord's JSON may nest. Its answers are a few levels deep.
const MAX_JSON_DEPTH: usize = 64;

/// The longest text of Discord's own that goes into an error message.
const MAX_QUOTED: usize = 500;

pub(crate) struct Transport {
    config: Arc<ClientConfig>,
    limits: RateLimits,
    token: Option<BotToken>,
}

/// Shows that there is one and nothing about it.
impl fmt::Debug for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transport")
            .field("authenticated", &self.token.is_some())
            .finish_non_exhaustive()
    }
}

impl Transport {
    pub fn new(config: Arc<ClientConfig>, token: Option<BotToken>) -> Transport {
        Transport {
            config,
            limits: RateLimits::default(),
            token,
        }
    }

    pub async fn get<T>(&self, route: Route) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let policy = self.config.policy;
        let clock = &self.config.clock;
        let mut rate_limited = 0u32;
        let mut server_errors = 0u32;
        loop {
            let wait = self.limits.delay_before(&route, clock.now());
            if wait > Duration::ZERO {
                if wait > policy.max_wait {
                    return Err(self.rate_limited(&route, wait));
                }
                tracing::info!(
                    seconds = wait.as_secs_f64(),
                    route = %route,
                    "waiting for Discord's rate limit"
                );
                clock.sleep(wait).await;
            }

            let request = self.request(&route)?;
            let answer = match self.config.client.send_collected(request).await {
                Ok(answer) => answer,
                Err(error) => return Err(self.transport_failure(&route, &error)),
            };

            let now = clock.now();
            self.limits.observe(&route, &answer.headers, now);
            let status = answer.status.as_u16();

            if status == 429 {
                let error = read_error(&answer.body);
                let retry_after = rate_limits::retry_after(
                    error.as_ref().and_then(|e| e.retry_after.0),
                    &answer.headers,
                    now,
                )
                .unwrap_or(policy.unstated_retry_after);
                let global = error.as_ref().and_then(|e| e.global.0) == Some(true)
                    || rate_limits::is_global(&answer.headers);
                self.limits.limited(&route, retry_after, global, now);
                rate_limited += 1;
                if rate_limited > policy.max_rate_limit_retries || retry_after > policy.max_wait {
                    return Err(self.rate_limited(&route, retry_after));
                }
                tracing::info!(
                    seconds = retry_after.as_secs_f64(),
                    route = %route,
                    global,
                    "Discord rate-limited a route"
                );
                continue;
            }

            if status >= 500 {
                server_errors += 1;
                if server_errors > policy.max_server_error_retries {
                    return Err(self.failure(
                        FailureKind::Unreachable,
                        &route,
                        &format!("Discord failed with HTTP {status}"),
                        Some(status),
                    ));
                }
                clock.sleep(policy.server_error_pause * server_errors).await;
                continue;
            }

            if answer.status.is_success() {
                return self.read(&route, &answer);
            }

            return Err(self.refused(&route, status, read_error(&answer.body)));
        }
    }

    fn request(&self, route: &Route) -> Result<Request> {
        let url = format!("{}{}", self.config.base, route.path);
        let mut request = Request::get(&url)
            .map_err(|_| {
                self.failure(
                    FailureKind::UnexpectedResponse,
                    route,
                    "the address of the request is not valid",
                    None,
                )
            })?
            .header(USER_AGENT, HeaderValue::from_static(USER_AGENT_TEXT))
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .timeout(self.config.timeout)
            .connect_timeout(self.config.timeout)
            .max_response_bytes(self.config.max_response_bytes)
            .max_redirects(0)
            .policy(self.config.address_policy);
        if let Some(token) = &self.token {
            // Marked sensitive, so that neither a debug print of the header
            // nor a redirect (there are none) can carry it anywhere.
            let mut value =
                HeaderValue::from_str(&format!("Bot {}", token.value())).map_err(|_| {
                    self.failure(
                        FailureKind::InvalidToken,
                        route,
                        "the token cannot be sent in a header",
                        None,
                    )
                })?;
            value.set_sensitive(true);
            request = request.header(AUTHORIZATION, value);
        }
        Ok(request)
    }

    fn read<T>(&self, route: &Route, answer: &Collected) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let status = Some(answer.status.as_u16());
        if !is_json(&answer.headers) {
            return Err(self.failure(
                FailureKind::UnexpectedResponse,
                route,
                "Discord answered with something other than JSON",
                status,
            ));
        }
        let another_shape = || {
            self.failure(
                FailureKind::UnexpectedResponse,
                route,
                "Discord answered with JSON of another shape",
                status,
            )
        };
        if nests_deeper_than(&answer.body, MAX_JSON_DEPTH) {
            return Err(another_shape());
        }
        match serde_json::from_slice::<Obj<T>>(&answer.body) {
            Ok(Obj(Some(value))) => Ok(value),
            Ok(Obj(None)) => Err(self.failure(
                FailureKind::UnexpectedResponse,
                route,
                "Discord answered with an empty body",
                status,
            )),
            Err(_) => Err(another_shape()),
        }
    }

    fn refused(&self, route: &Route, status: u16, error: Option<WireError>) -> DiscordError {
        let kind = match status {
            401 => FailureKind::InvalidToken,
            403 => FailureKind::MissingAccess,
            404 => FailureKind::UnknownResource,
            _ => FailureKind::UnexpectedResponse,
        };
        let code = error.as_ref().and_then(|e| e.code.0);
        let said = error
            .as_ref()
            .and_then(|e| e.message.get())
            .filter(|message| !message.is_empty())
            // Redacted before it is cut, or a cut through the token would
            // leave the front of it.
            .map(|message| format!(": {}", cut(&self.redact(message.to_owned()), MAX_QUOTED)))
            .unwrap_or_default();
        let number = code
            .map(|number| format!(" (code {number})"))
            .unwrap_or_default();
        DiscordError::new(
            kind,
            self.redact(format!(
                "Discord refused {route}: HTTP {status}{number}{said}"
            )),
            self.redact(route.to_string()),
            Some(status),
            code,
            None,
        )
    }

    fn rate_limited(&self, route: &Route, retry_after: Duration) -> DiscordError {
        DiscordError::new(
            FailureKind::RateLimited,
            self.redact(format!(
                "Discord asked to wait {} s before {route}, longer than the import waits",
                seconds_text(retry_after)
            )),
            self.redact(route.to_string()),
            Some(429),
            None,
            Some(retry_after),
        )
    }

    fn failure(
        &self,
        kind: FailureKind,
        route: &Route,
        what: &str,
        status: Option<u16>,
    ) -> DiscordError {
        DiscordError::new(
            kind,
            self.redact(format!("{route} failed: {what}")),
            self.redact(route.to_string()),
            status,
            None,
            None,
        )
    }

    fn transport_failure(&self, route: &Route, error: &OutboundError) -> DiscordError {
        // The names of what went wrong, never the client's own text: it can
        // carry a host name, and an error text is no place for more.
        let what = if error.is_timeout() {
            "Discord did not answer in time".to_owned()
        } else if error.is_too_large() {
            "Discord's answer is larger than the import reads".to_owned()
        } else {
            let how = match error {
                OutboundError::Resolve { .. } => "name resolution error",
                OutboundError::Connect { .. } => "connection error",
                OutboundError::Tls { .. } | OutboundError::Roots(_) => "secure connection error",
                OutboundError::AddressRefused { .. } => "address not allowed",
                OutboundError::Protocol(_) | OutboundError::Body(_) => {
                    "the answer from Discord broke off"
                }
                _ => "request error",
            };
            format!("could not reach Discord ({how})")
        };
        self.failure(FailureKind::Unreachable, route, &what, None)
    }

    /// Discord's own text goes into messages; it should never hold the token,
    /// and this makes sure. The token's own parts go too, for the one that
    /// would be written on its own, such as the bot's id in front.
    fn redact(&self, text: String) -> String {
        let Some(token) = &self.token else {
            return text;
        };
        let mut text = text.replace(token.value(), "[token]");
        for part in token.value().split('.').filter(|part| part.len() >= 8) {
            text = text.replace(part, "[token]");
        }
        text
    }
}

/// Reads Discord's error body, if it is one. A 429 from Discord's edge rather
/// than its API may be HTML; that reads as no body.
fn read_error(body: &[u8]) -> Option<WireError> {
    let body = &body[..body.len().min(MAX_ERROR_BYTES)];
    if body.is_empty() || nests_deeper_than(body, MAX_JSON_DEPTH) {
        return None;
    }
    serde_json::from_slice::<Obj<WireError>>(body)
        .ok()
        .and_then(|obj| obj.0)
}

fn is_json(headers: &HeaderMap) -> bool {
    let Some(value) = headers.get(CONTENT_TYPE) else {
        return true;
    };
    let Ok(text) = value.to_str() else {
        return false;
    };
    let media = text
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    media == "application/json" || media.ends_with("+json")
}

/// At most `limit` characters, never cutting one in two.
fn cut(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_owned(),
    }
}

/// Seconds with up to three decimals and no trailing zeros.
fn seconds_text(duration: Duration) -> String {
    let text = format!("{:.3}", duration.as_secs_f64());
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}
