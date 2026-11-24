//! In development the web client is served by its own dev server, on another
//! origin than the API. This lets exactly those two origins call the API with
//! their cookies. It is not part of the pipeline in any other environment.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;

const ORIGINS: [&str; 2] = ["http://localhost:3000", "http://127.0.0.1:3000"];

pub(super) async fn middleware(request: Request, next: Next) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .filter(|origin| {
            ORIGINS
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(origin))
        })
        .map(str::to_string);
    let is_preflight = request.method() == Method::OPTIONS
        && request.headers().contains_key(header::ORIGIN)
        && request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    if is_preflight {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NO_CONTENT;
        if let Some(origin) = origin {
            let requested = |name| request.headers().get(name).cloned();
            let headers = response.headers_mut();
            allow(headers, &origin);
            if let Some(method) = requested(header::ACCESS_CONTROL_REQUEST_METHOD) {
                headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, method);
            }
            if let Some(asked) = requested(header::ACCESS_CONTROL_REQUEST_HEADERS) {
                headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, asked);
            }
        }
        return response;
    }
    let mut response = next.run(request).await;
    if let Some(origin) = origin {
        allow(response.headers_mut(), &origin);
    }
    response
}

fn allow(headers: &mut axum::http::HeaderMap, origin: &str) {
    if let Ok(origin) = HeaderValue::from_str(origin) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
}
