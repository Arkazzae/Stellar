//! What every request goes through, in this order, outermost first:
//!
//! 1. count the response, whatever it turns out to be, and give the request
//!    its trace identifier;
//! 2. write one log line when it is answered;
//! 3. turn a handler that panicked into a 500 problem;
//! 4. read the `X-Forwarded-*` headers of the proxies that are believed;
//! 5. in development, answer the web dev server's cross-origin requests;
//! 6. count the request against the rate limit its route asks for;
//! 7. refuse a client that is not one of ours (the client gate);
//! 8. refuse a JSON route a body of another content type;
//! 9. authenticate: where a session cookie is read, once sessions exist;
//! 10. the handler.
//!
//! Which route a request is for is known to every step, since routing comes
//! first, but a step that wants to refuse a request does so before the
//! handler is ever called. The order is the one the first server had and it
//! matters: a refusal by the gate still counts against the limit, and a
//! response that the pipeline itself produces is counted and logged like any
//! other.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::Body;
use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use tracing::info;

use super::client_gate;
use super::error::{ApiError, internal_error_response};
use super::forwarded::{self, ClientInfo};
use super::rate_limit::RateLimiter;
use super::routes::Rules;
use super::trace;
use crate::logging::{self, REQUEST_TARGET};
use crate::state::AppState;

pub struct Pipeline {
    pub(super) state: AppState,
    rules: HashMap<(Method, String), Rules>,
    limiter: RateLimiter,
}

impl Pipeline {
    /// The router answers `HEAD` where a route has `GET`. A route answers only
    /// the methods its group named, so this is a 405 that lists them.
    fn head_refusal(&self, request: &Request) -> Option<Response> {
        let path = request.extensions().get::<MatchedPath>()?.as_str();
        if self.rules.contains_key(&(Method::HEAD, path.to_string())) {
            return None;
        }
        let mut methods: Vec<&str> = self
            .rules
            .keys()
            .filter(|(_, route)| route == path)
            .map(|(method, _)| method.as_str())
            .collect();
        methods.sort_unstable();
        let mut response = ApiError::status(StatusCode::METHOD_NOT_ALLOWED).into_response();
        response.headers_mut().insert(
            header::ALLOW,
            HeaderValue::from_str(&methods.join(", ")).ok()?,
        );
        Some(response)
    }

    fn rules_of(&self, request: &Request) -> Option<&Rules> {
        let path = request
            .extensions()
            .get::<MatchedPath>()?
            .as_str()
            .to_string();
        self.rules.get(&(request.method().clone(), path))
    }
}

/// Wraps the routes of every group in the pipeline.
pub fn build(
    state: AppState,
    router: Router<AppState>,
    rules: HashMap<(Method, String), Rules>,
) -> Router {
    let limiter = RateLimiter::new(&state.settings().rate_limits);
    let pipeline = Arc::new(Pipeline {
        state: state.clone(),
        rules,
        limiter,
    });
    let mut router = router
        .with_state(state)
        // Innermost first: each `layer` wraps what is already there.
        .layer(from_fn_with_state(Arc::clone(&pipeline), content_type))
        .layer(from_fn_with_state(
            Arc::clone(&pipeline),
            client_gate::middleware,
        ))
        .layer(from_fn_with_state(Arc::clone(&pipeline), rate_limit));
    if pipeline.state.settings().environment.is_development() {
        router = router.layer(from_fn_with_state(
            Arc::clone(&pipeline),
            super::cors::middleware,
        ));
    }
    let inner = router
        .layer(from_fn_with_state(
            Arc::clone(&pipeline),
            forwarded::middleware,
        ))
        .layer(from_fn_with_state(Arc::clone(&pipeline), recover))
        .layer(from_fn_with_state(Arc::clone(&pipeline), request_log))
        .layer(from_fn_with_state(pipeline, count));
    // The router adds the `Allow` header of a 405 after its layers have run,
    // so the one step that rewrites it has to stand outside the router.
    Router::new()
        .fallback_service(inner)
        .layer(axum::middleware::from_fn(allowed_methods))
}

async fn count(State(pipeline): State<Arc<Pipeline>>, request: Request, next: Next) -> Response {
    let response = trace::scope(trace::new_id(), next.run(request)).await;
    pipeline
        .state
        .metrics()
        .count_response(response.status().as_u16());
    response
}

async fn request_log(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    if !logging::is_probe(&path) {
        info!(
            target: REQUEST_TARGET,
            "{method} {path} {} in {} ms",
            response.status().as_u16(),
            started.elapsed().as_millis()
        );
    }
    response
}

/// A handler that panics is a 500 problem like one that returned a failure,
/// not a dropped connection. It runs as a task of its own so that the panic
/// stops there.
async fn recover(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let id = trace::current();
    let task = tokio::spawn(trace::scope(id, logging::serving(&path, next.run(request))));
    match task.await {
        Ok(response) => response,
        Err(failure) => {
            tracing::error!(trace_id = %trace::current(), "a handler for {path} failed: {failure}");
            internal_error_response()
        }
    }
}

async fn rate_limit(
    State(pipeline): State<Arc<Pipeline>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(rules) = pipeline.rules_of(&request) else {
        return next.run(request).await;
    };
    let Some(policy) = rules.rate_limit else {
        return next.run(request).await;
    };
    // A request that is about to be refused for its content type reaches an
    // answer that has no limit of its own.
    if refuses_content_type(rules, &request) {
        return next.run(request).await;
    }
    let address = request.extensions().get::<ClientInfo>().map_or_else(
        || "unknown".to_string(),
        |client| client.address.to_string(),
    );
    match pipeline.limiter.acquire(policy, &address).await {
        None => {
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
            response
        }
        // The lease is held until the handler has answered.
        Some(lease) => {
            let response = next.run(request).await;
            drop(lease);
            response
        }
    }
}

async fn content_type(
    State(pipeline): State<Arc<Pipeline>>,
    request: Request,
    next: Next,
) -> Response {
    if pipeline
        .rules_of(&request)
        .is_some_and(|rules| refuses_content_type(rules, &request))
    {
        return ApiError::status(StatusCode::UNSUPPORTED_MEDIA_TYPE).into_response();
    }
    if request.method() == Method::HEAD
        && let Some(refusal) = pipeline.head_refusal(&request)
    {
        return refusal;
    }
    // Authentication goes here: once there are sessions, the cookie is read
    // and the person is put into the request's extensions for the handler.
    next.run(request).await
}

/// The router lists `HEAD` among the methods of a route that has `GET`. The
/// API serves a method only where its group names it, so the answer does not
/// list it.
async fn allowed_methods(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    if response.status() == StatusCode::METHOD_NOT_ALLOWED {
        only_the_methods_of_the_route(&mut response);
    }
    response
}

fn only_the_methods_of_the_route(response: &mut Response) {
    let Some(allowed) = response
        .headers()
        .get(header::ALLOW)
        .and_then(|value| value.to_str().ok())
    else {
        return;
    };
    let mut kept: Vec<&str> = allowed
        .split(',')
        .map(str::trim)
        .filter(|method| *method != "HEAD")
        .collect();
    kept.sort_unstable();
    if let Ok(value) = HeaderValue::from_str(&kept.join(", ")) {
        response.headers_mut().insert(header::ALLOW, value);
    }
}

/// A JSON route that is sent a body of another type, or a body that names no
/// type at all. A request with neither a type nor a body gets as far as the
/// handler, which finds there is no body.
fn refuses_content_type(rules: &Rules, request: &Request) -> bool {
    if !rules.json_body {
        return false;
    }
    let headers = request.headers();
    let named = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|media_type| !media_type.is_empty());
    let Some(media_type) = named else {
        let has_body = headers.contains_key(header::TRANSFER_ENCODING)
            || headers
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|length| length.trim() != "0");
        return has_body;
    };
    !(media_type == "application/json"
        || (media_type.starts_with("application/") && media_type.ends_with("+json")))
}
