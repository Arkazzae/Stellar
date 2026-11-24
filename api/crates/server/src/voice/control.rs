//! The API's side of the media server's control API: the HTTP calls, and
//! nothing else.
//!
//! Every failure to get a usable answer is [`Unavailable`], so that the callers
//! have one thing to handle and a slow or broken media server never surfaces
//! as an unexpected error in a hub call. Each call has a time limit of its own
//! that covers reading the answer too, since a media server that stalls half
//! way through a body is as unavailable as one that never answered.
//!
//! The address is the operator's, set in the environment and never by a person
//! using the app, so no address is refused: the media server runs beside the
//! API, on the loopback by default.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use serde_json::value::RawValue;
use stellarr_outbound::header::{AUTHORIZATION, CONTENT_TYPE};
use stellarr_outbound::{AddressPolicy, Body, Client, HeaderValue, Method, Request, StatusCode};

/// A JSON string with its quotes, as it is written.
fn text(value: &str) -> String {
    serde_json::Value::String(value.to_string()).to_string()
}

use crate::settings::{MediaServerSettings, MediaTiming};
use crate::values::Id;

/// What a command's answer may weigh: they are lines of JSON.
const COMMAND_ANSWER_BYTES: u64 = 64 * 1024;

/// What one poll's answer may weigh. A poll carries up to 256 events, and an
/// event may carry a session description of up to 128 KiB (and a directory),
/// which JSON escaping lengthens a little; a poll the API could not read would
/// be asked for again from the same place, for ever.
const EVENT_ANSWER_BYTES: u64 = 64 * 1024 * 1024;

/// The media server could not be reached, did not answer in time, or answered
/// in a way the API cannot use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable(String);

impl Unavailable {
    fn new(text: impl Into<String>) -> Unavailable {
        Unavailable(text.into())
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unavailable {}

/// The answer to `POST /v1/reset`: the run of the media server and the last
/// sequence number it issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reset {
    pub epoch: String,
    pub seq: i64,
}

/// The answer to `POST /v1/sessions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub session_id: String,
    pub epoch: String,
}

/// One answer of the event long poll.
#[derive(Debug)]
pub struct Batch {
    pub epoch: String,
    pub reset: bool,
    pub events: Vec<MediaEvent>,
}

/// One event of the media plane: a `signal` for a browser, which carries a
/// kind and a payload, or a `closed` session, which carries a reason.
#[derive(Debug)]
pub struct MediaEvent {
    pub seq: i64,
    pub kind_of_event: String,
    pub session_id: String,
    pub kind: Option<String>,
    pub payload: Option<Box<RawValue>>,
    pub reason: Option<String>,
}

impl MediaEvent {
    /// The payload as the document text the media server wrote, when it is an
    /// object.
    pub fn payload_object(&self) -> Option<&str> {
        self.payload
            .as_deref()
            .map(RawValue::get)
            .filter(|text| text.starts_with('{'))
    }
}

/// How the media server took a browser's signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalOutcome {
    Accepted,
    /// `404 unknown-session`: the media server no longer has the session.
    UnknownSession,
    /// `409 stale-negotiation`: an answer to an offer the media server has
    /// already given up.
    Stale,
    /// `422`, or `413` for a body over 256 KiB: the signal was refused.
    Refused,
}

/// The client of the media server's control API.
pub struct Control {
    client: Client,
    base: String,
    secret: String,
    timing: MediaTiming,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireReset {
    epoch: Option<String>,
    seq: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireCreated {
    session_id: Option<String>,
    epoch: Option<String>,
}

#[derive(Deserialize)]
struct WireBatch {
    epoch: Option<String>,
    #[serde(default)]
    reset: bool,
    events: Option<Vec<Option<WireEvent>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireEvent {
    #[serde(default)]
    seq: i64,
    #[serde(rename = "type")]
    kind_of_event: Option<String>,
    session_id: Option<String>,
    kind: Option<String>,
    payload: Option<Box<RawValue>>,
    reason: Option<String>,
}

impl Control {
    pub fn new(settings: &MediaServerSettings) -> Control {
        Control {
            client: Client::new(),
            base: settings.control_url.clone(),
            secret: settings.control_secret.clone(),
            timing: settings.timing,
        }
    }

    pub fn timing(&self) -> &MediaTiming {
        &self.timing
    }

    /// Closes every session without events, as the API does when it starts or
    /// loses track of the events.
    pub async fn reset(&self) -> Result<Reset, Unavailable> {
        let answer = self
            .call(Method::POST, "v1/reset", None, self.timing.command)
            .await?;
        expect(&answer, StatusCode::OK)?;
        let wire: WireReset = read(&answer, "v1/reset")?;
        match wire {
            WireReset {
                epoch: Some(epoch),
                seq: Some(seq),
            } if !epoch.is_empty() && seq >= 0 => Ok(Reset { epoch, seq }),
            _ => Err(malformed("v1/reset")),
        }
    }

    pub async fn create_session(&self, room: Id, user: Id) -> Result<Created, Unavailable> {
        let body = format!("{{\"roomId\":\"{room}\",\"userId\":\"{user}\"}}");
        let answer = self
            .call(Method::POST, "v1/sessions", Some(body), self.timing.command)
            .await?;
        expect(&answer, StatusCode::CREATED)?;
        let wire: WireCreated = read(&answer, "v1/sessions")?;
        match wire {
            WireCreated {
                session_id: Some(session_id),
                epoch: Some(epoch),
            } if !session_id.is_empty() && !epoch.is_empty() => Ok(Created { session_id, epoch }),
            _ => Err(malformed("v1/sessions")),
        }
    }

    /// Closes one session. A session the media server does not know is as
    /// closed as it gets.
    pub async fn close_session(&self, session: &str) -> Result<(), Unavailable> {
        let path = session_path(session);
        let answer = self
            .call(Method::DELETE, &path, None, self.timing.command)
            .await?;
        if answer.status.is_success() || answer.status == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(unexpected(&answer))
        }
    }

    /// Hands the media server the browser's description of what it publishes.
    pub async fn send_offer(&self, session: &str, sdp: &str) -> Result<SignalOutcome, Unavailable> {
        let body = format!("{{\"sdp\":{}}}", text(sdp));
        self.signal(session, "offer", body).await
    }

    /// Answers the media server's offer number `negotiation`.
    pub async fn send_answer(
        &self,
        session: &str,
        sdp: &str,
        negotiation: i64,
    ) -> Result<SignalOutcome, Unavailable> {
        let body = format!(
            "{{\"sdp\":{},\"negotiationId\":{negotiation}}}",
            text(sdp)
        );
        self.signal(session, "answer", body).await
    }

    /// Hands over one ICE candidate of the browser, given as the text of its
    /// JSON document, or `None` for the end of its candidates.
    pub async fn send_candidate(
        &self,
        session: &str,
        candidate: Option<&str>,
    ) -> Result<SignalOutcome, Unavailable> {
        // The document is handed on as the browser wrote it; the caller has
        // checked that it is JSON.
        let candidate = candidate.map_or("null", str::trim);
        self.signal(session, "candidate", format!("{{\"candidate\":{candidate}}}"))
            .await
    }

    /// Replaces the session's complete allowlist of forwarded media ids.
    pub async fn send_subscriptions(
        &self,
        session: &str,
        mids: &[String],
    ) -> Result<SignalOutcome, Unavailable> {
        let list: Vec<String> = mids.iter().map(|mid| text(mid)).collect();
        self.signal(session, "subscriptions", format!("{{\"mids\":[{}]}}", list.join(",")))
            .await
    }

    /// Waits for the events after `after` of the run `epoch`. The media server
    /// answers at once when it has some, or when the run or the window of
    /// events it keeps no longer matches.
    pub async fn events(&self, epoch: &str, after: i64) -> Result<Batch, Unavailable> {
        let path = format!(
            "v1/events?epoch={}&after={after}&waitMs={}",
            escape(epoch),
            self.timing.event_wait_ms
        );
        let limit = self.timing.command + Duration::from_millis(self.timing.event_wait_ms);
        let answer = self
            .call_limited(Method::GET, &path, None, limit, EVENT_ANSWER_BYTES)
            .await?;
        expect(&answer, StatusCode::OK)?;
        let wire: WireBatch = read(&answer, "v1/events")?;
        let (Some(epoch), Some(events)) = (wire.epoch.filter(|epoch| !epoch.is_empty()), wire.events)
        else {
            return Err(malformed("v1/events"));
        };
        let events = events
            .into_iter()
            .map(|event| {
                let event = event?;
                Some(MediaEvent {
                    seq: event.seq,
                    kind_of_event: event.kind_of_event?,
                    session_id: event.session_id?,
                    kind: event.kind,
                    payload: event.payload,
                    reason: event.reason,
                })
            })
            .collect::<Option<Vec<MediaEvent>>>()
            .ok_or_else(|| malformed("v1/events"))?;
        Ok(Batch {
            epoch,
            reset: wire.reset,
            events,
        })
    }

    async fn signal(
        &self,
        session: &str,
        kind: &str,
        body: String,
    ) -> Result<SignalOutcome, Unavailable> {
        let path = format!("{}/{kind}", session_path(session));
        let answer = self
            .call(Method::POST, &path, Some(body), self.timing.command)
            .await?;
        Ok(match answer.status {
            status if status.is_success() => SignalOutcome::Accepted,
            StatusCode::NOT_FOUND => SignalOutcome::UnknownSession,
            StatusCode::CONFLICT => SignalOutcome::Stale,
            // A body the media server will not read is as much the browser's
            // doing as a description it cannot use: the hub limits the
            // payload, not the body that JSON escaping makes of it.
            StatusCode::UNPROCESSABLE_ENTITY | StatusCode::PAYLOAD_TOO_LARGE => {
                SignalOutcome::Refused
            }
            _ => return Err(unexpected(&answer)),
        })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
        limit: Duration,
    ) -> Result<Answer, Unavailable> {
        self.call_limited(method, path, body, limit, COMMAND_ANSWER_BYTES)
            .await
    }

    /// Sends one request under its own time limit and ceiling on the answer.
    async fn call_limited(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
        limit: Duration,
        ceiling: u64,
    ) -> Result<Answer, Unavailable> {
        // What is logged is the route without its query: the path of a
        // session is no secret, but nothing from a URL belongs in a log.
        let route = path.split('?').next().unwrap_or(path).to_string();
        let mut request = Request::new(method, &format!("{}{path}", self.base))
            .map_err(|_| Unavailable::new(format!("the media server's address cannot be used for {route}")))?
            .policy(AddressPolicy::Anywhere)
            .connect_timeout(self.timing.connect)
            .timeout(limit)
            .max_response_bytes(ceiling)
            .max_redirects(0)
            .decompress(false);
        // Without a secret the control API listens on the loopback only and
        // checks nothing, so an empty bearer would only be noise.
        if !self.secret.is_empty() {
            let mut value = HeaderValue::from_str(&format!("Bearer {}", self.secret))
                .map_err(|_| Unavailable::new("the media server's secret cannot be sent"))?;
            value.set_sensitive(true);
            request = request.header(AUTHORIZATION, value);
        }
        if let Some(body) = body {
            request = request
                .header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/json; charset=utf-8"),
                )
                .body(Body::bytes(body.into_bytes()));
        }
        match self.client.send_collected(request).await {
            Ok(collected) => Ok(Answer {
                status: collected.status,
                json: is_json(&collected.headers),
                body: collected.body,
                route,
            }),
            Err(error) if error.is_timeout() => Err(Unavailable::new(format!(
                "the media server did not answer {route} in time"
            ))),
            Err(error) => Err(Unavailable::new(format!(
                "the media server could not be reached for {route}: {error}"
            ))),
        }
    }
}

struct Answer {
    status: StatusCode,
    json: bool,
    body: bytes::Bytes,
    route: String,
}

/// Whether an answer says it is JSON, as `application/json` or a type that
/// ends in `+json`.
fn is_json(headers: &stellarr_outbound::HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
        .is_some_and(|kind| kind == "application/json" || kind.ends_with("+json"))
}

fn expect(answer: &Answer, status: StatusCode) -> Result<(), Unavailable> {
    if answer.status == status {
        Ok(())
    } else {
        Err(unexpected(answer))
    }
}

fn read<'a, T: Deserialize<'a>>(answer: &'a Answer, path: &str) -> Result<T, Unavailable> {
    if !answer.json {
        return Err(Unavailable::new(format!(
            "the media server answered {path} with something that is not JSON"
        )));
    }
    serde_json::from_slice(&answer.body).map_err(|_| {
        Unavailable::new(format!(
            "the media server answered {path} with unreadable JSON"
        ))
    })
}

fn unexpected(answer: &Answer) -> Unavailable {
    Unavailable::new(format!(
        "the media server answered {} with {}",
        answer.route,
        answer.status.as_u16()
    ))
}

fn malformed(path: &str) -> Unavailable {
    Unavailable::new(format!(
        "the media server answered {path} without the fields the contract names"
    ))
}

fn session_path(session: &str) -> String {
    format!("v1/sessions/{}", escape(session))
}

/// A path segment or query value as `Uri.EscapeDataString` writes it: every
/// byte but the unreserved characters of RFC 3986 as `%XX`.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_escaped_but_for_its_unreserved_characters() {
        assert_eq!(escape("epoch-1_a.b~c"), "epoch-1_a.b~c");
        assert_eq!(escape("a b/c?d&e=ż"), "a%20b%2Fc%3Fd%26e%3D%C5%BC");
    }

    #[test]
    fn a_batch_is_read_with_the_payload_text_as_the_media_server_wrote_it() {
        let wire: WireBatch = serde_json::from_str(
            "{\"epoch\":\"e\",\"reset\":false,\"events\":[{\"seq\":1,\"type\":\"signal\",\"sessionId\":\"s\",\"kind\":\"offer\",\"payload\":{ \"sdp\" : \"v=0\", \"n\": 1.0 }}]}",
        )
        .unwrap();

        let event = wire.events.unwrap().remove(0).unwrap();

        assert_eq!(event.payload.unwrap().get(), "{ \"sdp\" : \"v=0\", \"n\": 1.0 }");
    }
}
