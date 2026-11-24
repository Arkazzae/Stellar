//! The HTTP upgrade for the application WebSocket.

use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use axum::routing::get;
use http::header::{CACHE_CONTROL, CONTENT_TYPE, SEC_WEBSOCKET_PROTOCOL};
use http::request::Parts;
use http::{HeaderMap, HeaderValue, StatusCode};
use stellarr_websocket::{self as websocket, HandshakeError};

use crate::app::Application;
use crate::config::Config;
use crate::connection;
use crate::hub::Hub;
use crate::ids::random_id;
use crate::native;

struct Server<A: Application, F> {
    app: Arc<A>,
    hub: Hub,
    config: Arc<Config>,
    authenticate: F,
}

/// Serves the application WebSocket at `path`.
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
    });
    Router::new().route(
        path,
        get(move |request: Request| connect(server.clone(), request)),
    )
}

async fn connect<A, F, Fut>(server: Arc<Server<A, F>>, request: Request) -> Response
where
    A: Application,
    F: Fn(Arc<Parts>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<A::Identity, Response>> + Send + 'static,
{
    let (mut parts, _) = request.into_parts();
    let on_upgrade = websocket::take_upgrade(&mut parts);
    let mut handshake = match websocket::accept(&parts) {
        Ok(response) => response,
        Err(error) => return refuse(error.response(), error.message()),
    };
    if !offers_stellarr_protocol(&parts.headers) {
        let mut response = Response::new(());
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return refuse(response, "Expected WebSocket subprotocol stellarr.v1");
    }
    let Some(on_upgrade) = on_upgrade else {
        let error = HandshakeError::NoUpgradeSupport;
        return refuse(error.response(), error.message());
    };
    let identity = match (server.authenticate)(Arc::new(parts)).await {
        Ok(identity) => identity,
        Err(refusal) => return refusal,
    };
    handshake.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static(native::SUBPROTOCOL),
    );

    tokio::spawn(async move {
        let ws = websocket::Config {
            max_message_size: server.config.max_message_size,
            ..websocket::Config::default()
        };
        match websocket::upgraded(on_upgrade, ws).await {
            Ok((sender, receiver)) => {
                connection::serve(
                    server.app.clone(),
                    server.hub.clone(),
                    server.config.clone(),
                    Arc::new(identity),
                    random_id().into(),
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

/// Whether the client offers the Stellarr WebSocket subprotocol.
fn offers_stellarr_protocol(headers: &HeaderMap) -> bool {
    headers
        .get_all(SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|protocol| protocol.trim() == native::SUBPROTOCOL)
}

fn refuse(response: http::Response<()>, message: &'static str) -> Response {
    let mut response = response.map(|()| Body::from(message));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store"),
    );
    response
}
