//! How the API reads and writes JSON, decided once.
//!
//! - Property names are camelCase: put `#[serde(rename_all = "camelCase")]` on
//!   every struct that crosses the wire.
//! - A missing value is `null`, never left out: write `Option<T>` fields as
//!   they are, and do not use `skip_serializing_if`.
//! - Ids are lower case and moments end in `Z`: use `values::Id` and
//!   `values::Timestamp`, which write themselves that way.
//! - An enum is written as the clients expect it for that enum, usually a
//!   lower case name: say so with `#[serde(rename_all = "lowercase")]`.
//! - The content type is `application/json; charset=utf-8`.
//! - Members come in the order the struct declares them. Do not answer with
//!   `serde_json::json!` or a `Value`, whose objects sort their keys.
//!
//! A request body that is not JSON of the right shape is a bad request with an
//! empty body, as is one that has no body at all.

use axum::body::Body;
use axum::extract::{FromRequest, Request};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::body::{BodyCeiling, MAX_REQUEST_BODY_BYTES, read_all};
use super::error::ApiError;

/// A value that is sent as JSON with status 200, or read from the body of the
/// request.
pub struct Json<T>(pub T);

/// The value as JSON with the given status.
pub fn json_response<T: Serialize>(status: StatusCode, value: &T) -> Response {
    let body = match serde_json::to_vec(value) {
        Ok(body) => body,
        // A value that cannot be written as JSON is a mistake in the code
        // that built it, which the client is told as any other failure.
        Err(error) => {
            return ApiError::internal(format!("a response did not serialize: {error}"))
                .into_response();
        }
    };
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        json_response(StatusCode::OK, &self.0)
    }
}

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for Json<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, _: &S) -> Result<Json<T>, ApiError> {
        let (parts, body) = request.into_parts();
        let ceiling = parts
            .extensions
            .get::<BodyCeiling>()
            .map_or(MAX_REQUEST_BODY_BYTES, |ceiling| ceiling.0);
        let bytes = read_all(&parts.headers, body, ceiling).await?;
        serde_json::from_slice(&bytes)
            .map(Json)
            .map_err(|_| ApiError::bad_request())
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::values::{Id, Timestamp};

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Account {
        id: Id,
        display_name: String,
        avatar_url: Option<String>,
        created_at: Timestamp,
    }

    #[test]
    fn a_struct_is_written_the_way_the_clients_read_it() {
        let account = Account {
            id: Id::parse("8F62A4D1-BDCB-4D86-BD1C-6E351900E629").unwrap(),
            display_name: "Maya".to_string(),
            avatar_url: None,
            created_at: Timestamp::parse("2026-11-18 21:57:04.81431").unwrap(),
        };

        assert_eq!(
            serde_json::to_string(&account).unwrap(),
            "{\"id\":\"8f62a4d1-bdcb-4d86-bd1c-6e351900e629\",\"displayName\":\"Maya\",\
             \"avatarUrl\":null,\"createdAt\":\"2026-11-18T21:57:04.81431Z\"}"
        );
    }

    #[tokio::test]
    async fn the_response_has_the_content_type_and_the_status() {
        let response = json_response(
            StatusCode::CREATED,
            &serde_json::json!({ "status": "live" }),
        );

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/json; charset=utf-8"
        );
    }

    async fn read(body: &str) -> Result<Json<Account>, ApiError> {
        let request = Request::builder()
            .body(Body::from(body.to_string()))
            .unwrap();
        Json::<Account>::from_request(request, &()).await
    }

    #[tokio::test]
    async fn a_body_that_is_not_the_right_json_is_a_bad_request() {
        for body in ["", "{bad", "[]", "{\"id\":\"x\"}"] {
            let refusal = read(body).await.err().expect(body);

            assert!(
                matches!(refusal, ApiError::Status(status) if status.as_u16() == 400),
                "{body:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_body_of_the_right_shape_is_read() {
        let Json(account) = read(
            "{\"id\":\"8f62a4d1-bdcb-4d86-bd1c-6e351900e629\",\"displayName\":\"Maya\",\
             \"avatarUrl\":null,\"createdAt\":\"2026-11-18T21:57:04Z\",\"unknown\":1}",
        )
        .await
        .unwrap();

        assert_eq!(account.display_name, "Maya");
    }
}
