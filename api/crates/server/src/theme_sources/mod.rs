//! Read public theme files and the two theme discovery APIs across browser origins.

use std::sync::LazyLock;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use serde::Deserialize;
use stellarr_outbound::{AddressPolicy, Client, Request, Url};
use tokio::sync::Semaphore;

use crate::auth::Caller;
use crate::http::{ApiError, ApiResult, Endpoints, Query};

static HTTP: LazyLock<Client> = LazyLock::new(Client::new);
static IN_FLIGHT: Semaphore = Semaphore::const_new(8);

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/me/theme-source", fetch)
        .authorized()
}

#[derive(Deserialize)]
struct Source {
    url: String,
    kind: Kind,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Css,
    Catalog,
    Picture,
}

impl Kind {
    fn limit(self) -> u64 {
        match self {
            Self::Css => 4 * 1024 * 1024,
            Self::Catalog => 7 * 1024 * 1024,
            Self::Picture => 25 * 1024 * 1024,
        }
    }

    fn accepts(self, url: &Url) -> bool {
        if !url.is_https() || url.port() != 443 {
            return false;
        }
        if !matches!(self, Self::Catalog) {
            return true;
        }
        let path = url.path_and_query().split('?').next().unwrap_or("");
        match url.host() {
            "api.betterdiscord.app" => path
                .strip_prefix("/v3/store/")
                .is_some_and(|entry| !entry.is_empty() && !entry.contains('/')),
            "api.github.com" => {
                path == "/search/repositories"
                    || matches!(
                        path.split('/').collect::<Vec<_>>().as_slice(),
                        ["", "repos", _, _, "git", "trees", _]
                    )
            }
            _ => false,
        }
    }

    fn content_type(self, upstream: &str) -> Option<&'static str> {
        match self {
            Self::Catalog if upstream == "application/json" => Some("application/json"),
            Self::Css => match upstream {
                "" | "text/css" | "text/plain" | "application/octet-stream" => {
                    Some("text/plain; charset=utf-8")
                }
                _ => None,
            },
            Self::Picture => match upstream {
                "image/jpeg" => Some("image/jpeg"),
                "image/png" => Some("image/png"),
                "image/webp" => Some("image/webp"),
                "image/gif" => Some("image/gif"),
                "image/avif" => Some("image/avif"),
                "application/octet-stream" | "" => Some("application/octet-stream"),
                _ => None,
            },
            _ => None,
        }
    }
}

async fn fetch(_: Caller, Query(source): Query<Source>) -> ApiResult {
    if source.url.len() > 4096 {
        return Err(ApiError::bad_request());
    }
    let request = Request::get(&source.url).map_err(|_| ApiError::bad_request())?;
    if !source.kind.accepts(request.url()) {
        return Err(ApiError::bad_request());
    }
    let _permit = IN_FLIGHT
        .try_acquire()
        .map_err(|_| ApiError::status(StatusCode::TOO_MANY_REQUESTS))?;
    let answer = HTTP
        .send_collected(
            request
                .header(header::USER_AGENT, HeaderValue::from_static("Stellarr"))
                .policy(AddressPolicy::PublicOnly)
                .timeout(Duration::from_secs(10))
                .max_response_bytes(source.kind.limit())
                .max_redirects(4),
        )
        .await
        .map_err(|_| ApiError::status(StatusCode::BAD_GATEWAY))?;

    match answer.status {
        StatusCode::NOT_FOUND => return Err(ApiError::not_found()),
        StatusCode::TOO_MANY_REQUESTS => {
            return Err(ApiError::status(StatusCode::TOO_MANY_REQUESTS));
        }
        StatusCode::FORBIDDEN
            if answer
                .headers
                .get("x-ratelimit-remaining")
                .is_some_and(|value| value == "0") =>
        {
            return Err(ApiError::status(StatusCode::TOO_MANY_REQUESTS));
        }
        _ if !answer.status.is_success() || !source.kind.accepts(&answer.url) => {
            return Err(ApiError::status(StatusCode::BAD_GATEWAY));
        }
        _ => {}
    }

    let upstream_type = answer
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let content_type = source
        .kind
        .content_type(&upstream_type)
        .ok_or_else(|| ApiError::status(StatusCode::UNSUPPORTED_MEDIA_TYPE))?;
    let final_url = format!(
        "https://{}{}",
        answer.url.host_header(),
        answer.url.path_and_query()
    );
    let mut response = Response::new(Body::from(answer.body));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    headers.insert(
        "x-theme-source",
        HeaderValue::from_str(&final_url).map_err(|_| ApiError::bad_request())?,
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_catalog_requests_to_discovery_and_serves_inert_files() {
        let url = |text| Url::parse(text).unwrap();
        assert!(Kind::Catalog.accepts(&url("https://api.betterdiscord.app/v3/store/ClearVision")));
        assert!(Kind::Catalog.accepts(&url(
            "https://api.github.com/repos/owner/theme/git/trees/HEAD?recursive=1"
        )));
        assert!(!Kind::Catalog.accepts(&url("https://api.github.com/user")));
        assert!(!Kind::Css.accepts(&url("http://example.com/file.css")));
        assert_eq!(
            Kind::Css.content_type("text/css"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(Kind::Css.content_type("text/html"), None);
        assert_eq!(Kind::Picture.content_type("image/svg+xml"), None);
    }
}
