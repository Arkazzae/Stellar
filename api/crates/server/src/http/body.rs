//! Reading the body of a request, within the ceiling.
//!
//! Every body is read through here, so that none is read without a limit. The
//! ceiling is the outer one for a request of any kind, form overhead
//! included; each kind of upload still has a limit of its own, checked once
//! the form is parsed.

use axum::body::Body;
use axum::http::{HeaderMap, header};
use bytes::Bytes;
use http_body_util::{BodyExt, Limited};

use super::error::ApiError;

/// The ceiling of the whole server, 128 MiB. A route that reads JSON has a
/// lower one of its own (see `Endpoints::json_body`).
pub const MAX_REQUEST_BODY_BYTES: usize = 128 * 1024 * 1024;

/// The whole body. A request that declares more than the ceiling is refused
/// with 413 before a byte is read; one that turns out to have more, or whose
/// body breaks off, is a bad request.
pub async fn read_all(headers: &HeaderMap, body: Body, ceiling: usize) -> Result<Bytes, ApiError> {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    if declared.is_some_and(|length| length > ceiling as u64) {
        return Err(ApiError::TooLarge);
    }
    match Limited::new(body, ceiling).collect().await {
        Ok(collected) => Ok(collected.to_bytes()),
        Err(_) => Err(ApiError::bad_request()),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn declared(length: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(length).unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn a_body_within_the_ceiling_is_read_whole() {
        let bytes = read_all(&declared("5"), Body::from("hello"), 5)
            .await
            .unwrap();

        assert_eq!(&bytes[..], b"hello");
    }

    #[tokio::test]
    async fn a_declared_length_over_the_ceiling_is_refused_unread() {
        let refusal = read_all(&declared("6"), Body::from("hello!"), 5)
            .await
            .unwrap_err();

        assert!(matches!(refusal, ApiError::TooLarge));
    }

    #[tokio::test]
    async fn a_body_that_outgrows_the_ceiling_is_a_bad_request() {
        let refusal = read_all(&HeaderMap::new(), Body::from("hello!"), 5)
            .await
            .unwrap_err();

        assert!(matches!(refusal, ApiError::Status(status) if status.as_u16() == 400));
    }

    #[tokio::test]
    async fn no_body_is_an_empty_one() {
        let bytes = read_all(&HeaderMap::new(), Body::empty(), 5).await.unwrap();

        assert!(bytes.is_empty());
    }
}

/// The most the body of this request may be, which the pipeline sets from the
/// rules of its route.
#[derive(Debug, Clone, Copy)]
pub struct BodyCeiling(pub usize);
