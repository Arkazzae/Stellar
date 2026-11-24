//! What every request goes through, in this order, outermost first:
//!
//! 0. read the target as the first server's front did (see [`super::path`]):
//!    refuse a request line that is too long or a path with a null character,
//!    decode the path, remove its dot segments and one slash at its end.
//!    Nothing after this sees another path;
//! 1. count the response, whatever it turns out to be, and give the request
//!    its trace identifier;
//! 2. write one log line when it is answered;
//! 3. turn a handler that panicked into a 500 problem;
//! 4. read the `X-Forwarded-*` headers of the proxies that are believed;
//! 5. in development, answer the web dev server's cross-origin requests;
//! 6. count the request against the rate limit its route asks for;
//! 7. refuse a client that is not one of ours (the client gate);
//! 8. refuse a JSON route a body of another content type;
//! 9. authenticate: the session cookie is read and checked against the
//!    database, and a route that needs a session refuses a request without one;
//!    a route that needs a fresh second step refuses a session that has not
//!    passed one lately;
//! 10. the handler.
//!
//! Which route a request is for is known to every step, since routing comes
//! first, but a step that wants to refuse a request does so before the
//! handler is ever called. The order is the one the first server had and it
//! matters: a refusal by the gate still counts against the limit, and a
//! response that the pipeline itself produces is counted and logged like any
//! other.

use std::collections::{HashMap, HashSet};
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
use crate::auth::session::{self, Authentication};
use crate::auth::{self, Caller};
use crate::database::DbError;
use crate::logging::{self, REQUEST_TARGET};
use crate::state::AppState;
use crate::values::Timestamp;

pub struct Pipeline {
    pub(super) state: AppState,
    rules: HashMap<(Method, String), Rules>,
    limiter: RateLimiter,
    /// Every literal segment of every registered route: the only words of a
    /// path the log may repeat when no route took the request.
    words: HashSet<String>,
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
    let words = super::secret_path::words(rules.keys().map(|(_, pattern)| pattern.as_str()));
    let pipeline = Arc::new(Pipeline {
        state: state.clone(),
        rules,
        limiter,
        words,
    });
    let mut router = router
        .with_state(state)
        // Innermost first: each `layer` wraps what is already there.
        .layer(from_fn_with_state(Arc::clone(&pipeline), route_checks))
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
    // Outermost of all, the target is read as the first server's front read
    // it, so that nothing inside sees another path than the router does.
    Router::new()
        .fallback_service(inner)
        .layer(axum::middleware::from_fn(allowed_methods))
        .layer(axum::middleware::from_fn(super::path::middleware))
}

async fn count(State(pipeline): State<Arc<Pipeline>>, request: Request, next: Next) -> Response {
    let response = trace::scope(trace::new_id(), next.run(request)).await;
    pipeline
        .state
        .metrics()
        .count_response(response.status().as_u16());
    response
}

/// The path as the log may say it. A secret in a route that took the request
/// (an invite's code) is replaced by a mark; when no route took it, every
/// segment the server does not know as a word of its own routes is.
fn shown_path(pipeline: &Pipeline, request: &Request, path: &str) -> String {
    match request.extensions().get::<MatchedPath>() {
        Some(matched) => super::secret_path::masked(matched.as_str(), path),
        None => super::secret_path::masked_unmatched(&pipeline.words, path),
    }
}

async fn request_log(
    State(pipeline): State<Arc<Pipeline>>,
    request: Request,
    next: Next,
) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let shown = shown_path(&pipeline, &request, &path);
    let response = next.run(request).await;
    if !logging::is_probe(&path) {
        info!(
            target: REQUEST_TARGET,
            "{method} {shown} {} in {} ms",
            response.status().as_u16(),
            started.elapsed().as_millis()
        );
    }
    response
}

/// A handler that panics is a 500 problem like one that returned a failure,
/// not a dropped connection. It runs as a task of its own so that the panic
/// stops there.
async fn recover(State(pipeline): State<Arc<Pipeline>>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let shown = shown_path(&pipeline, &request, &path);
    let id = trace::current();
    let task = tokio::spawn(trace::scope(id, logging::serving(&path, next.run(request))));
    match task.await {
        Ok(response) => response,
        Err(failure) => {
            tracing::error!(trace_id = %trace::current(), "a handler for {shown} failed: {failure}");
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

/// Reads the session cookie, then holds the request to what its route asks:
/// the right content type, a method it has, and a signed-in person where the
/// route needs one. Authentication runs for every request, whatever its
/// route, since a cookie that is no longer good is taken away on whatever
/// request carries it.
async fn route_checks(
    State(pipeline): State<Arc<Pipeline>>,
    mut request: Request,
    next: Next,
) -> Response {
    let environment = &pipeline.state.settings().environment;
    let https = request
        .extensions()
        .get::<ClientInfo>()
        .is_some_and(ClientInfo::is_https);
    let mut cookies = Vec::new();
    let mut renewal = None;
    let cookie = auth::cookies::read(request.headers(), auth::session::cookie_name(environment));
    let ticket = cookie
        .and_then(|value| session::Ticket::open(pipeline.state.sealer(), &value, Timestamp::now()));
    if let Some(ticket) = ticket {
        let state = pipeline.state.clone();
        let outcome = pipeline
            .state
            .database()
            .read(move |connection| -> Result<_, DbError> {
                auth::session::authenticate(
                    connection,
                    state.sealer(),
                    &state.settings().environment,
                    https,
                    ticket,
                    Timestamp::now(),
                )
            })
            .await;
        match outcome {
            Err(error) => return ApiError::from(error).into_response(),
            Ok(Authentication::Anonymous) => {}
            Ok(Authentication::Rejected) => {
                cookies.push(auth::session::sign_out_cookie(environment, https))
            }
            Ok(Authentication::SignedIn {
                caller,
                renewed_cookie,
                touch,
            }) => {
                if let Some(touch) = touch
                    && let Err(error) = pipeline
                        .state
                        .database()
                        .write(move |tx| touch.write(tx))
                        .await
                {
                    return ApiError::from(error).into_response();
                }
                renewal = renewed_cookie;
                request.extensions_mut().insert(caller);
            }
        }
    }

    let rules = pipeline.rules_of(&request);
    if let Some(rules) = rules.filter(|rules| rules.json_body) {
        let ceiling = rules
            .body_limit
            .unwrap_or(super::routes::DEFAULT_JSON_BODY_BYTES);
        request
            .extensions_mut()
            .insert(super::body::BodyCeiling(ceiling));
    }
    let refusal = if rules.is_some_and(|rules| refuses_content_type(rules, &request)) {
        Some(ApiError::status(StatusCode::UNSUPPORTED_MEDIA_TYPE).into_response())
    } else if request.method() == Method::HEAD
        && let Some(refusal) = pipeline.head_refusal(&request)
    {
        Some(refusal)
    } else if rules.is_some_and(|rules| rules.authorize)
        && request.extensions().get::<Caller>().is_none()
    {
        // A route that needs a session answers a request without one with
        // 401 and no body.
        Some(ApiError::unauthorized().into_response())
    } else {
        None
    };
    let refusal = match refusal {
        None if rules.is_some_and(|rules| rules.fresh_second_step) => {
            refusal_for_stale_second_step(&pipeline, request.extensions().get::<Caller>().copied())
                .await
        }
        other => other,
    };
    let mut response = match refusal {
        Some(refusal) => refusal,
        None => next.run(request).await,
    };
    // A cookie that is due for renewal is left as it is when the handler has
    // signed in or out itself.
    let own_name = format!("{}=", auth::session::cookie_name(environment));
    let handler_set_it = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .any(|value| value.to_str().is_ok_and(|line| line.starts_with(&own_name)));
    if !handler_set_it {
        cookies.extend(renewal);
    }
    if !cookies.is_empty() {
        put_cookies_first(&mut response, &cookies);
    }
    response
}

/// The refusal of a session that has not passed its second step lately, or
/// nothing for one that has (and for an account that has none to pass).
async fn refusal_for_stale_second_step(
    pipeline: &Pipeline,
    caller: Option<Caller>,
) -> Option<Response> {
    let caller = caller?;
    let fresh = pipeline
        .state
        .database()
        .read(move |connection| {
            auth::fresh::has_fresh_second_step(
                connection,
                caller.user,
                Some(caller.session),
                Timestamp::now(),
            )
        })
        .await;
    match fresh {
        Ok(true) => None,
        Ok(false) => Some(match auth::fresh::refusal(pipeline.state.messages()) {
            Ok(problem) => ApiError::from(problem).into_response(),
            Err(missing) => ApiError::from(missing).into_response(),
        }),
        Err(error) => Some(ApiError::from(error).into_response()),
    }
}

/// Puts the cookies that authentication set ahead of the ones the handler
/// set, so that a handler that signs in after a rejected cookie wins, and
/// keeps the answer out of every cache, as any answer that sets a session
/// cookie is.
fn put_cookies_first(response: &mut Response, cookies: &[String]) {
    let handler: Vec<HeaderValue> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .cloned()
        .collect();
    response.headers_mut().remove(header::SET_COOKIE);
    let first = cookies
        .iter()
        .filter_map(|line| HeaderValue::from_str(line).ok());
    for value in first.chain(handler) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    super::session_cookie::keep_out_of_caches(response);
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
