//! The seam to the HTTP server: a router that serves the hub's negotiation
//! and its WebSocket at one path, for the application's router to mount.
//!
//! Who may connect is not decided here. The application gives a function
//! that looks at the request, cookies and headers and query included, and
//! either names who is connecting or refuses with the response to send.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use axum::routing::any;
use http::header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE};
use http::request::Parts;
use http::{HeaderValue, Method, StatusCode, Uri};
use stellarr_websocket::{self as websocket, HandshakeError};

use crate::app::Application;
use crate::config::Config;
use crate::connection;
use crate::hub::Hub;
use crate::ids::random_id;
use crate::message;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Negotiated; its transport has not come yet.
    Waiting,
    /// A transport is connected.
    Active,
}

struct PendingEntry {
    connection_id: Arc<str>,
    created: Instant,
    phase: Phase,
}

/// The connections that negotiation has made and a transport has not yet
/// claimed, and the ones it has, by the key the transport connects with.
#[derive(Default)]
struct Pending {
    entries: HashMap<String, PendingEntry>,
    /// Keys in the order they were made, to find the ones that expired.
    order: VecDeque<(String, Instant)>,
}

enum Claim {
    NotFound,
    AlreadyActive,
    Claimed(Arc<str>),
}

impl Pending {
    fn purge(&mut self, config: &Config, now: Instant) {
        while let Some((key, created)) = self.order.front() {
            if now.duration_since(*created) < config.pending_connection_ttl {
                break;
            }
            if self
                .entries
                .get(key)
                .is_some_and(|entry| entry.phase == Phase::Waiting && entry.created == *created)
            {
                self.entries.remove(key);
            }
            self.order.pop_front();
        }
    }

    fn insert(&mut self, config: &Config, key: String, connection_id: Arc<str>) -> bool {
        let now = Instant::now();
        self.purge(config, now);
        if self.entries.len() >= config.max_pending_connections {
            return false;
        }
        self.order.push_back((key.clone(), now));
        self.entries.insert(
            key,
            PendingEntry {
                connection_id,
                created: now,
                phase: Phase::Waiting,
            },
        );
        true
    }

    fn peek(&mut self, config: &Config, key: &str) -> Option<Phase> {
        self.purge(config, Instant::now());
        self.entries.get(key).map(|entry| entry.phase)
    }

    fn claim(&mut self, config: &Config, key: &str) -> Claim {
        self.purge(config, Instant::now());
        match self.entries.get_mut(key) {
            None => Claim::NotFound,
            Some(entry) if entry.phase == Phase::Active => Claim::AlreadyActive,
            Some(entry) => {
                entry.phase = Phase::Active;
                Claim::Claimed(entry.connection_id.clone())
            }
        }
    }

    fn release(&mut self, key: &str) {
        self.entries.remove(key);
    }
}

struct Server<A: Application, F> {
    app: Arc<A>,
    hub: Hub,
    config: Arc<Config>,
    authenticate: F,
    pending: Mutex<Pending>,
}

/// Forgets a connection's key when its task ends, however it ends.
struct Release<A: Application, F> {
    server: Arc<Server<A, F>>,
    key: Option<String>,
}

impl<A: Application, F> Drop for Release<A, F> {
    fn drop(&mut self) {
        if let Some(key) = &self.key {
            self.server
                .pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .release(key);
        }
    }
}

/// A router with the hub at `path` (the WebSocket) and `path/negotiate`.
pub fn router<A, F, Fut>(
    path: &str,
    app: Arc<A>,
    hub: Hub,
    config: Config,
    authenticate: F,
) -> Router
where
    A: Application,
    F: Fn(Arc<Parts>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<A::Identity, Response>> + Send + 'static,
{
    let server = Arc::new(Server {
        app,
        hub,
        config: Arc::new(config),
        authenticate,
        pending: Mutex::new(Pending::default()),
    });
    let on_negotiate = server.clone();
    let on_connect = server;
    Router::new()
        .route(
            &format!("{}/negotiate", path.trim_end_matches('/')),
            any(move |request: Request| negotiate(on_negotiate.clone(), request)),
        )
        .route(
            path,
            any(move |request: Request| connect(on_connect.clone(), request)),
        )
}

fn text(status: StatusCode, body: &'static str) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store"),
    );
    response
}

fn query_value<'a>(uri: &'a Uri, name: &str) -> Option<&'a str> {
    uri.query()?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then_some(value)
    })
}

async fn negotiate<A, F, Fut>(server: Arc<Server<A, F>>, request: Request) -> Response
where
    A: Application,
    F: Fn(Arc<Parts>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<A::Identity, Response>> + Send + 'static,
{
    let (parts, _) = request.into_parts();
    let parts = Arc::new(parts);
    if let Err(refusal) = (server.authenticate)(parts.clone()).await {
        return refusal;
    }
    if parts.method != Method::POST {
        let mut response = text(StatusCode::METHOD_NOT_ALLOWED, "");
        response
            .headers_mut()
            .insert(ALLOW, HeaderValue::from_static("POST"));
        return response;
    }
    let requested = query_value(&parts.uri, "negotiateVersion");
    let connection_id = random_id();
    let connection_token = random_id();
    let body = message::negotiate_response(requested, &connection_id, &connection_token);
    // A version 0 client connects with the id itself.
    let key = if body.contains("\"connectionToken\"") {
        connection_token
    } else {
        connection_id.clone()
    };
    if !body.starts_with("{\"error\"") {
        let stored = server
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(&server.config, key, connection_id.into());
        if !stored {
            return text(
                StatusCode::SERVICE_UNAVAILABLE,
                "Too many connections are waiting to start",
            );
        }
    }
    let mut response = Response::new(Body::from(body));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

async fn connect<A, F, Fut>(server: Arc<Server<A, F>>, request: Request) -> Response
where
    A: Application,
    F: Fn(Arc<Parts>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<A::Identity, Response>> + Send + 'static,
{
    let (mut parts, _) = request.into_parts();
    let on_upgrade = websocket::take_upgrade(&mut parts);
    let parts = Arc::new(parts);
    let identity = match (server.authenticate)(parts.clone()).await {
        Ok(identity) => identity,
        Err(refusal) => return refusal,
    };

    let key = query_value(&parts.uri, "id")
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let upgrade = websocket::is_upgrade_request(&parts.headers);
    if !upgrade {
        return match key {
            None => text(StatusCode::BAD_REQUEST, "Connection ID required"),
            Some(key) => {
                let known = server
                    .pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .peek(&server.config, &key);
                match known {
                    None => text(StatusCode::NOT_FOUND, "No Connection with that ID"),
                    Some(_) => text(
                        StatusCode::BAD_REQUEST,
                        "The connection can only be used over WebSockets",
                    ),
                }
            }
        };
    }

    // The handshake is checked before the key is used up, so a request that
    // is refused for its headers leaves the connection to be tried again.
    let handshake = match websocket::accept(&parts) {
        Ok(response) => response,
        Err(error) => return refuse(error),
    };
    let Some(on_upgrade) = on_upgrade else {
        return refuse(HandshakeError::NoUpgradeSupport);
    };

    let connection_id: Arc<str> = match &key {
        None => random_id().into(),
        Some(key) => match server
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .claim(&server.config, key)
        {
            Claim::NotFound => return text(StatusCode::NOT_FOUND, "No Connection with that ID"),
            Claim::AlreadyActive => return text(StatusCode::CONFLICT, ""),
            Claim::Claimed(id) => id,
        },
    };

    let release = Release {
        server: server.clone(),
        key,
    };
    let config = server.config.clone();
    tokio::spawn(async move {
        let _release = release;
        let ws = websocket::Config {
            max_message_size: usize::MAX,
            ..websocket::Config::default()
        };
        match websocket::upgraded(on_upgrade, ws).await {
            Ok((sender, receiver)) => {
                connection::serve(
                    _release.server.app.clone(),
                    _release.server.hub.clone(),
                    config,
                    Arc::new(identity),
                    connection_id,
                    sender,
                    receiver,
                )
                .await;
            }
            Err(error) => {
                tracing::debug!(%error, "a connection was dropped before it was upgraded")
            }
        }
    });
    handshake.map(|()| Body::empty())
}

fn refuse(error: HandshakeError) -> Response {
    let mut response = error.response().map(|()| Body::from(error.message()));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    response
}
