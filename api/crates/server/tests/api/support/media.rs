//! The control API of the media server as its contract describes it, played
//! by a test. It listens like the real one, keeps sessions and a numbered list
//! of events, answers the long poll, records every request, and lets a test
//! restart it, take it down, make it lose its window of events, answer slowly
//! or answer nonsense.
//!
//! It is the same code whether it stands behind the binary of a test or behind
//! the first server in a container: [`FakeMedia::start_on`] binds any address.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};

/// The secret the fake expects, and the binary of a test is given.
pub const SECRET: &str = "media-test-secret";

/// How long a test waits for the media server's side of something.
const PATIENCE: Duration = Duration::from_secs(10);

/// One request the API sent the fake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    /// What follows the question mark, none when there is no query.
    pub query: String,
    pub body: String,
    pub authorization: Option<String>,
}

impl Recorded {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("the body is JSON")
    }
}

/// What the fake answers instead of what it would.
#[derive(Clone)]
pub struct Canned {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

struct State {
    requests: Vec<Recorded>,
    /// Session id to (room, user, activity camera).
    sessions: HashMap<String, (String, String, bool)>,
    events: Vec<String>,
    signal_answers: HashMap<String, u16>,
    epoch: String,
    first_kept_seq: u64,
    session_count: u64,
    last_served_seq: u64,
    down: bool,
    slowness: Duration,
    polls_held: bool,
    creates_held: bool,
    signal_on_create: Option<(String, String)>,
    canned: Option<Canned>,
    canned_polls: Option<Canned>,
    bearer: Option<String>,
}

struct Inner {
    state: Mutex<State>,
    changed: watch::Sender<u64>,
}

impl Inner {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn wake(&self) {
        self.changed.send_modify(|version| *version += 1);
    }
}

pub struct FakeMedia {
    inner: Arc<Inner>,
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl Drop for FakeMedia {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeMedia {
    /// On the loopback, at a port the system chooses.
    pub async fn start() -> FakeMedia {
        FakeMedia::start_on("127.0.0.1:0".parse().unwrap()).await
    }

    pub async fn start_on(address: SocketAddr) -> FakeMedia {
        let listener = TcpListener::bind(address)
            .await
            .expect("the fake media server binds");
        let address = listener.local_addr().unwrap();
        let (changed, _) = watch::channel(0);
        let inner = Arc::new(Inner {
            state: Mutex::new(State {
                requests: Vec::new(),
                sessions: HashMap::new(),
                events: Vec::new(),
                signal_answers: HashMap::new(),
                epoch: "epoch-1".to_string(),
                first_kept_seq: 1,
                session_count: 0,
                last_served_seq: 0,
                down: false,
                slowness: Duration::ZERO,
                polls_held: false,
                creates_held: false,
                signal_on_create: None,
                canned: None,
                canned_polls: None,
                bearer: Some(SECRET.to_string()),
            }),
            changed,
        });
        let serving = inner.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let inner = serving.clone();
                tokio::spawn(async move { connection(inner, stream).await });
            }
        });
        FakeMedia {
            inner,
            address,
            task,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The address as the API's setting names it.
    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub fn epoch(&self) -> String {
        self.inner.state().epoch.clone()
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.inner.state().requests.clone()
    }

    pub fn sessions(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.inner.state().sessions.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// The room and the account a session was created for.
    pub fn session_of(&self, session: &str) -> Option<(String, String)> {
        self.inner
            .state()
            .sessions
            .get(session)
            .map(|(room, owner, _)| (room.clone(), owner.clone()))
    }

    /// When set, every request is cut off as a media server that is not there
    /// would, after the connection was made.
    pub fn set_down(&self, down: bool) {
        self.inner.state().down = down;
    }

    /// How long every request but the event poll takes before it is answered.
    pub fn set_slowness(&self, slowness: Duration) {
        self.inner.state().slowness = slowness;
    }

    /// The secret the fake expects; none lets any request in.
    pub fn set_bearer(&self, bearer: Option<&str>) {
        self.inner.state().bearer = bearer.map(str::to_string);
    }

    /// Answers every request but the event poll with this, whatever it was.
    pub fn answer_everything_with(&self, canned: Option<Canned>) {
        self.inner.state().canned = canned;
    }

    /// Answers every event poll with this.
    pub fn answer_polls_with(&self, canned: Option<Canned>) {
        self.inner.state().canned_polls = canned;
    }

    /// When set, creating a session issues this signal for the new session at
    /// once and answers only after a poll has handed the signal out, as a
    /// media server that offers its forwarded tracks straight away can.
    pub fn signal_on_create(&self, signal: Option<(&str, &str)>) {
        self.inner.state().signal_on_create =
            signal.map(|(kind, payload)| (kind.to_string(), payload.to_string()));
    }

    /// A new run of the process: a new epoch, no sessions, and numbering from
    /// the start.
    pub fn restart(&self, epoch: &str) {
        {
            let mut state = self.inner.state();
            state.epoch = epoch.to_string();
            state.sessions.clear();
            state.events.clear();
            state.first_kept_seq = 1;
            state.last_served_seq = 0;
        }
        self.inner.wake();
    }

    /// Forgets a session, as a media server that lost it would.
    pub fn drop_session(&self, session: &str) {
        self.inner.state().sessions.remove(session);
    }

    /// Behaves as if more events than the window holds were issued since the
    /// last poll: the next poll gets a reset in the same epoch, until the API
    /// resets the media server.
    pub fn lose_window(&self) {
        {
            let mut state = self.inner.state();
            state.first_kept_seq = state.events.len() as u64 + 2;
        }
        self.inner.wake();
    }

    /// Answers signals of one session with this status instead of the usual
    /// 204.
    pub fn answer_signals(&self, session: &str, status: u16) {
        self.inner
            .state()
            .signal_answers
            .insert(session.to_string(), status);
    }

    /// Keeps every long poll waiting until [`release_polls`](Self::release_polls).
    pub fn hold_polls(&self) {
        self.inner.state().polls_held = true;
        self.inner.wake();
    }

    pub fn release_polls(&self) {
        self.inner.state().polls_held = false;
        self.inner.wake();
    }

    /// Keeps every request to create a session waiting, before it creates
    /// anything, until [`release_creates`](Self::release_creates).
    pub fn hold_creates(&self) {
        self.inner.state().creates_held = true;
        self.inner.wake();
    }

    pub fn release_creates(&self) {
        self.inner.state().creates_held = false;
        self.inner.wake();
    }

    /// Issues a signal event; its number.
    pub fn signal(&self, session: &str, kind: &str, payload: &str) -> u64 {
        self.publish(|seq| {
            format!(
                "{{\"seq\":{seq},\"type\":\"signal\",\"sessionId\":{},\"kind\":{},\"payload\":{payload}}}",
                json!(session),
                json!(kind)
            )
        })
    }

    /// Issues an event that ends a session; its number.
    pub fn closed(&self, session: &str, reason: &str) -> u64 {
        self.publish(|seq| {
            format!(
                "{{\"seq\":{seq},\"type\":\"closed\",\"sessionId\":{},\"reason\":{}}}",
                json!(session),
                json!(reason)
            )
        })
    }

    /// Issues an event as written.
    pub fn raw_event(&self, render: impl Fn(u64) -> String) -> u64 {
        self.publish(render)
    }

    fn publish(&self, render: impl Fn(u64) -> String) -> u64 {
        let seq = {
            let mut state = self.inner.state();
            let seq = state.events.len() as u64 + 1;
            state.events.push(render(seq));
            seq
        };
        self.inner.wake();
        seq
    }

    /// The first request `matching` takes, waiting for it to arrive.
    pub async fn next_request(&self, matching: impl Fn(&Recorded) -> bool) -> Recorded {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(found) = self.requests().into_iter().find(|r| matching(r)) {
                return found;
            }
            assert!(Instant::now() < deadline, "no such request arrived");
            sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn delete_of(&self, session: &str) -> Recorded {
        let path = format!("/v1/sessions/{session}");
        self.next_request(|r| r.method == "DELETE" && r.path == path)
            .await
    }

    pub fn count(&self, method: &str, path: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.method == method && r.path == path)
            .count()
    }

    /// Waits until `count` requests of this kind have arrived.
    pub async fn count_reaches(&self, method: &str, path: &str, count: usize) {
        until(|| self.count(method, path) >= count).await;
    }

    /// The requests that are not polls, in the order they arrived.
    pub fn commands(&self) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|r| r.path != "/v1/events")
            .collect()
    }
}

/// Waits until `condition` holds.
pub async fn until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "the condition never held");
        sleep(Duration::from_millis(10)).await;
    }
}

enum Answer {
    Json(u16, String),
    Raw(Canned),
    Empty(u16),
    /// Close the connection without a word.
    Drop,
}

async fn connection(inner: Arc<Inner>, mut stream: TcpStream) {
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let Some((method, target, headers, body)) = read_request(&mut stream, &mut buffer).await
        else {
            return;
        };
        let (path, query) = match target.split_once('?') {
            Some((path, query)) => (path.to_string(), query.to_string()),
            None => (target.clone(), String::new()),
        };
        let authorization = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.clone());
        let close = headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("connection") && value.eq_ignore_ascii_case("close")
        });
        let recorded = Recorded {
            method,
            path,
            query,
            body,
            authorization,
        };
        // A script that stands outside the process drives the fake through
        // addresses of its own, which are no part of what the API asked.
        if let Some(operation) = recorded.path.strip_prefix("/_fake/") {
            let body = administer(&inner, operation, &recorded.query).await;
            let bytes = render(200, Some("application/json"), body.as_bytes());
            if stream.write_all(&bytes).await.is_err() || close {
                return;
            }
            continue;
        }
        inner.state().requests.push(recorded.clone());
        let answer = respond(&inner, &recorded).await;
        let bytes = match answer {
            Answer::Drop => return,
            Answer::Json(status, body) => render(status, Some("application/json"), body.as_bytes()),
            Answer::Raw(canned) => {
                render(canned.status, canned.content_type.as_deref(), &canned.body)
            }
            Answer::Empty(status) => render(status, None, b""),
        };
        if stream.write_all(&bytes).await.is_err() || close {
            return;
        }
    }
}

fn render(status: u16, content_type: Option<&str>, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    if let Some(content_type) = content_type {
        head.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    if status != 204 {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    if status != 204 {
        bytes.extend_from_slice(body);
    }
    bytes
}

/// One request: its method, target, headers and body. A body is read by its
/// length or in chunks.
async fn read_request(
    stream: &mut TcpStream,
    buffer: &mut Vec<u8>,
) -> Option<(String, String, Vec<(String, String)>, String)> {
    let end = loop {
        if let Some(at) = find(buffer, b"\r\n\r\n") {
            break at;
        }
        let mut chunk = [0u8; 8192];
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    buffer.drain(..end + 4);
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    let value_of = |wanted: &str| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.clone())
    };
    let mut body = Vec::new();
    if value_of("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let at = fill_until(stream, buffer, b"\r\n").await?;
            let size_line = String::from_utf8_lossy(&buffer[..at]).into_owned();
            buffer.drain(..at + 2);
            let size = usize::from_str_radix(size_line.split(';').next()?.trim(), 16).ok()?;
            while buffer.len() < size + 2 {
                let mut chunk = [0u8; 8192];
                let read = stream.read(&mut chunk).await.ok()?;
                if read == 0 {
                    return None;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            body.extend_from_slice(&buffer[..size]);
            buffer.drain(..size + 2);
            if size == 0 {
                break;
            }
        }
    } else if let Some(length) = value_of("content-length").and_then(|v| v.parse::<usize>().ok()) {
        while buffer.len() < length {
            let mut chunk = [0u8; 8192];
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }
        body.extend_from_slice(&buffer[..length]);
        buffer.drain(..length);
    }
    Some((
        method,
        target,
        headers,
        String::from_utf8_lossy(&body).into_owned(),
    ))
}

async fn fill_until(stream: &mut TcpStream, buffer: &mut Vec<u8>, pattern: &[u8]) -> Option<usize> {
    loop {
        if let Some(at) = find(buffer, pattern) {
            return Some(at);
        }
        let mut chunk = [0u8; 8192];
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn error(status: u16, code: &str) -> Answer {
    Answer::Json(status, json!({"error": code, "detail": code}).to_string())
}

async fn respond(inner: &Arc<Inner>, request: &Recorded) -> Answer {
    let (down, slowness, bearer, canned, canned_polls) = {
        let state = inner.state();
        (
            state.down,
            state.slowness,
            state.bearer.clone(),
            state.canned.clone(),
            state.canned_polls.clone(),
        )
    };
    if down {
        return Answer::Drop;
    }
    if let Some(secret) = bearer
        && request.authorization.as_deref() != Some(&format!("Bearer {secret}"))
    {
        return error(401, "unauthorized");
    }
    let segments: Vec<&str> = request.path.trim_matches('/').split('/').collect();
    let polling = segments.as_slice() == ["v1", "events"];
    if !slowness.is_zero() && !polling {
        sleep(slowness).await;
    }
    if polling {
        if let Some(canned) = canned_polls {
            return Answer::Raw(canned);
        }
    } else if let Some(canned) = canned {
        return Answer::Raw(canned);
    }
    match (request.method.as_str(), segments.as_slice()) {
        ("POST", ["v1", "reset"]) => reset(inner),
        ("POST", ["v1", "sessions"]) => create(inner, &request.body, false).await,
        ("POST", ["v1", "activity-sessions"]) => create(inner, &request.body, true).await,
        ("DELETE", ["v1", "sessions", id]) => {
            inner.state().sessions.remove(*id);
            Answer::Empty(204)
        }
        (
            "POST",
            [
                "v1",
                "sessions",
                id,
                "offer" | "answer" | "candidate" | "subscriptions",
            ],
        ) => signal(inner, id),
        ("GET", ["v1", "events"]) => poll(inner, &request.query).await,
        _ => error(404, "not-found"),
    }
}

fn reset(inner: &Inner) -> Answer {
    let mut state = inner.state();
    state.sessions.clear();
    state.first_kept_seq = state.first_kept_seq.min(state.events.len() as u64 + 1);
    Answer::Json(
        200,
        json!({"epoch": state.epoch, "seq": state.events.len()}).to_string(),
    )
}

async fn create(inner: &Arc<Inner>, body: &str, activity: bool) -> Answer {
    let Ok(request) = serde_json::from_str::<Value>(body) else {
        return error(422, "invalid-request");
    };
    let room = request["roomId"].as_str().unwrap_or_default().to_string();
    let user = request["userId"].as_str().unwrap_or_default().to_string();
    let mut changes = inner.changed.subscribe();
    while inner.state().creates_held {
        if changes.changed().await.is_err() {
            return Answer::Drop;
        }
    }
    let (session, epoch, wait_for) = {
        let mut state = inner.state();
        state.sessions.retain(|_, (place, owner, camera)| {
            *owner != user || *camera != activity || (activity && *place != room)
        });
        state.session_count += 1;
        let session = format!("session-{}", state.session_count);
        let epoch = state.epoch.clone();
        state
            .sessions
            .insert(session.clone(), (room, user, activity));
        (session, epoch, state.signal_on_create.clone())
    };
    if let Some((kind, payload)) = wait_for {
        let seq = {
            let mut state = inner.state();
            let seq = state.events.len() as u64 + 1;
            state.events.push(format!(
                "{{\"seq\":{seq},\"type\":\"signal\",\"sessionId\":{},\"kind\":{},\"payload\":{payload}}}",
                json!(session),
                json!(kind)
            ));
            seq
        };
        inner.wake();
        let deadline = Instant::now() + PATIENCE;
        while inner.state().last_served_seq < seq && Instant::now() < deadline {
            sleep(Duration::from_millis(10)).await;
        }
        // Long enough for a pump that did not wait to have handed the event
        // out already, to nobody.
        sleep(Duration::from_millis(150)).await;
    }
    Answer::Json(
        201,
        json!({"sessionId": session, "epoch": epoch}).to_string(),
    )
}

fn signal(inner: &Inner, session: &str) -> Answer {
    let state = inner.state();
    if let Some(status) = state.signal_answers.get(session) {
        return match status {
            204 => Answer::Empty(204),
            409 => error(409, "stale-negotiation"),
            422 => error(422, "invalid-sdp"),
            413 => error(413, "payload-too-large"),
            other => error(*other, "failure"),
        };
    }
    if state.sessions.contains_key(session) {
        Answer::Empty(204)
    } else {
        error(404, "unknown-session")
    }
}

async fn poll(inner: &Arc<Inner>, query: &str) -> Answer {
    let parameter = |name: &str| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_string())
    };
    let epoch = parameter("epoch").unwrap_or_default();
    let after: u64 = parameter("after").and_then(|v| v.parse().ok()).unwrap_or(0);
    let wait: u64 = parameter("waitMs")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut changes = inner.changed.subscribe();
    while inner.state().polls_held {
        if changes.changed().await.is_err() {
            return Answer::Drop;
        }
    }
    let deadline = Instant::now() + Duration::from_millis(wait.min(30_000));
    loop {
        changes.borrow_and_update();
        {
            let mut state = inner.state();
            if epoch != state.epoch || after + 1 < state.first_kept_seq {
                return Answer::Json(
                    200,
                    json!({"epoch": state.epoch, "reset": true, "events": []}).to_string(),
                );
            }
            let pending: Vec<String> = state
                .events
                .iter()
                .skip(after as usize)
                .take(256)
                .cloned()
                .collect();
            if !pending.is_empty() || Instant::now() >= deadline {
                state.last_served_seq = state.last_served_seq.max(after + pending.len() as u64);
                return Answer::Json(
                    200,
                    format!(
                        "{{\"epoch\":\"{}\",\"reset\":false,\"events\":[{}]}}",
                        state.epoch,
                        pending.join(",")
                    ),
                );
            }
        }
        let _ = timeout(
            deadline.saturating_duration_since(Instant::now()),
            changes.changed(),
        )
        .await;
    }
}

/// The value of a query parameter, decoded.
fn query_value(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| decode(value))
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let pair = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                if let Ok(byte) = u8::from_str_radix(pair, 16) {
                    out.push(byte);
                    index += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What a script outside the process may ask of the fake, over `/_fake/...`:
/// the same moves a test makes, and the requests it has taken.
async fn administer(inner: &Arc<Inner>, operation: &str, query: &str) -> String {
    let text = |name: &str| query_value(query, name).unwrap_or_default();
    let number = |name: &str| {
        query_value(query, name)
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let canned = || Canned {
        status: number("status") as u16,
        content_type: query_value(query, "type").filter(|kind| !kind.is_empty()),
        body: text("body").into_bytes(),
    };
    match operation {
        "requests" => {
            let list: Vec<Value> = inner
                .state()
                .requests
                .iter()
                .map(|r| {
                    json!({"method": r.method, "path": r.path, "query": r.query, "body": r.body, "authorization": r.authorization})
                })
                .collect();
            return Value::Array(list).to_string();
        }
        "sessions" => {
            let mut ids: Vec<String> = inner.state().sessions.keys().cloned().collect();
            ids.sort();
            return json!(ids).to_string();
        }
        "clear" => inner.state().requests.clear(),
        "down" => inner.state().down = number("on") == 1,
        "slow" => inner.state().slowness = Duration::from_millis(number("ms")),
        "bearer" => inner.state().bearer = query_value(query, "secret").filter(|s| !s.is_empty()),
        "drop" => {
            inner.state().sessions.remove(&text("session"));
        }
        "restart" => {
            {
                let mut state = inner.state();
                state.epoch = text("epoch");
                state.sessions.clear();
                state.events.clear();
                state.first_kept_seq = 1;
                state.last_served_seq = 0;
            }
            inner.wake();
        }
        "lose-window" => {
            {
                let mut state = inner.state();
                state.first_kept_seq = state.events.len() as u64 + 2;
            }
            inner.wake();
        }
        "signal" | "closed" => {
            let seq = {
                let mut state = inner.state();
                let seq = state.events.len() as u64 + 1;
                let line = if operation == "signal" {
                    format!(
                        "{{\"seq\":{seq},\"type\":\"signal\",\"sessionId\":{},\"kind\":{},\"payload\":{}}}",
                        json!(text("session")),
                        json!(text("kind")),
                        text("payload")
                    )
                } else {
                    format!(
                        "{{\"seq\":{seq},\"type\":\"closed\",\"sessionId\":{},\"reason\":{}}}",
                        json!(text("session")),
                        json!(text("reason"))
                    )
                };
                state.events.push(line);
                seq
            };
            inner.wake();
            return json!({"seq": seq}).to_string();
        }
        "answer-signals" => {
            inner
                .state()
                .signal_answers
                .insert(text("session"), number("status") as u16);
        }
        "hold-polls" => {
            inner.state().polls_held = true;
            inner.wake();
        }
        "release-polls" => {
            inner.state().polls_held = false;
            inner.wake();
        }
        "hold-creates" => {
            inner.state().creates_held = true;
            inner.wake();
        }
        "release-creates" => {
            inner.state().creates_held = false;
            inner.wake();
        }
        "canned" => inner.state().canned = Some(canned()),
        "canned-polls" => inner.state().canned_polls = Some(canned()),
        "canned-clear" => {
            let mut state = inner.state();
            state.canned = None;
            state.canned_polls = None;
        }
        _ => return json!({"error": "unknown operation"}).to_string(),
    }
    json!({"ok": true}).to_string()
}
