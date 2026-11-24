//! How the API says that a request failed: a JSON problem, as RFC 9457
//! describes, sent as `application/problem+json`.
//!
//! ```json
//! {"type":"https://tools.ietf.org/html/rfc9110#section-15.5.4","title":"...","status":403,
//!  "code":"unsupported-client","downloadPath":"/","traceId":"00-...-00"}
//! ```
//!
//! The members come in this order and no other: `type`, `title`, `status`,
//! `detail`, then the extensions in the order they were added, then `errors`
//! for a validation problem, and last `traceId`. A client that only reads a
//! problem does not care, but a person comparing two responses does.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::Value;

use super::trace;

/// The `type` and the title that go with a status when the code gives none.
fn defaults(status: StatusCode) -> Option<(&'static str, &'static str)> {
    let known = match status.as_u16() {
        400 => ("15.5.1", "Bad Request"),
        401 => ("15.5.2", "Unauthorized"),
        403 => ("15.5.4", "Forbidden"),
        404 => ("15.5.5", "Not Found"),
        405 => ("15.5.6", "Method Not Allowed"),
        406 => ("15.5.7", "Not Acceptable"),
        408 => ("15.5.9", "Request Timeout"),
        409 => ("15.5.10", "Conflict"),
        413 => ("15.5.14", "Content Too Large"),
        415 => ("15.5.16", "Unsupported Media Type"),
        500 => ("15.6.1", "An error occurred while processing your request."),
        501 => ("15.6.2", "Not Implemented"),
        502 => ("15.6.3", "Bad Gateway"),
        503 => ("15.6.4", "Service Unavailable"),
        504 => ("15.6.5", "Gateway Timeout"),
        _ => return None,
    };
    Some(known)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    status: StatusCode,
    title: Option<String>,
    detail: Option<String>,
    extensions: Vec<(String, Value)>,
    errors: Option<Vec<(String, Vec<String>)>>,
}

impl Problem {
    /// A problem with the given status and the standard title for it.
    pub fn new(status: StatusCode) -> Problem {
        Problem {
            status,
            title: None,
            detail: None,
            extensions: Vec::new(),
            errors: None,
        }
    }

    /// A refusal of the request's fields, `errors` naming each field and what
    /// is wrong with it, in the language of the instance.
    pub fn validation(errors: Vec<(String, Vec<String>)>) -> Problem {
        Problem {
            title: Some("One or more validation errors occurred.".to_string()),
            errors: Some(errors),
            ..Problem::new(StatusCode::BAD_REQUEST)
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Problem {
        self.title = Some(title.into());
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Problem {
        self.detail = Some(detail.into());
        self
    }

    /// Adds a member after the standard ones, such as `code`.
    pub fn with(mut self, name: &str, value: impl Into<Value>) -> Problem {
        self.extensions.push((name.to_string(), value.into()));
        self
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }
}

struct Errors<'a>(&'a [(String, Vec<String>)]);

impl Serialize for Errors<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (field, messages) in self.0 {
            map.serialize_entry(field, messages)?;
        }
        map.end()
    }
}

impl Serialize for Problem {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let standard = defaults(self.status);
        let mut map = serializer.serialize_map(None)?;
        if let Some((section, _)) = standard {
            map.serialize_entry(
                "type",
                &format!("https://tools.ietf.org/html/rfc9110#section-{section}"),
            )?;
        }
        if let Some(title) = self.title.as_deref().or(standard.map(|(_, title)| title)) {
            map.serialize_entry("title", title)?;
        }
        map.serialize_entry("status", &self.status.as_u16())?;
        if let Some(detail) = &self.detail {
            map.serialize_entry("detail", detail)?;
        }
        for (name, value) in &self.extensions {
            map.serialize_entry(name, value)?;
        }
        if let Some(errors) = &self.errors {
            map.serialize_entry("errors", &Errors(errors))?;
        }
        map.serialize_entry("traceId", &trace::current())?;
        map.end()
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        // A problem always serializes: its members are strings and numbers.
        let body = serde_json::to_vec(&self).unwrap_or_default();
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = self.status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(problem: Problem) -> String {
        serde_json::to_string(&problem).unwrap()
    }

    #[test]
    fn the_members_come_in_the_order_clients_have_always_seen() {
        let problem = Problem::new(StatusCode::FORBIDDEN)
            .title("This app is no longer supported.")
            .with("code", "unsupported-client")
            .with("downloadPath", "/");

        let json = serde_json::to_string(&problem).unwrap();

        assert!(
            json.starts_with(
                "{\"type\":\"https://tools.ietf.org/html/rfc9110#section-15.5.4\",\
                 \"title\":\"This app is no longer supported.\",\"status\":403,\
                 \"code\":\"unsupported-client\",\"downloadPath\":\"/\",\"traceId\":\"00-"
            ),
            "{json}"
        );
    }

    #[test]
    fn a_validation_problem_lists_its_fields_before_the_trace() {
        let problem = Problem::validation(vec![
            ("username".to_string(), vec!["Too short.".to_string()]),
            (
                "email".to_string(),
                vec!["Not an address.".to_string(), "Taken.".to_string()],
            ),
        ]);

        let json = serde_json::to_string(&problem).unwrap();

        assert!(
            json.starts_with(
                "{\"type\":\"https://tools.ietf.org/html/rfc9110#section-15.5.1\",\
                 \"title\":\"One or more validation errors occurred.\",\"status\":400,\
                 \"errors\":{\"username\":[\"Too short.\"],\"email\":[\"Not an address.\",\"Taken.\"]},\
                 \"traceId\":\"00-"
            ),
            "{json}"
        );
    }

    #[test]
    fn a_status_without_a_text_of_its_own_gets_the_standard_one() {
        let json = serde_json::to_string(&Problem::new(StatusCode::UNAUTHORIZED)).unwrap();
        assert!(json.contains("\"title\":\"Unauthorized\""), "{json}");
        let json = serde_json::to_string(
            &Problem::new(StatusCode::SERVICE_UNAVAILABLE).title("Database unavailable"),
        )
        .unwrap();
        assert!(
            json.contains("section-15.6.4\",\"title\":\"Database unavailable\",\"status\":503"),
            "{json}"
        );
    }

    #[test]
    fn a_status_the_standard_does_not_name_has_no_type() {
        let json =
            serde_json::to_string(&Problem::new(StatusCode::TOO_MANY_REQUESTS).title("Slow down"))
                .unwrap();

        assert!(
            json.starts_with("{\"title\":\"Slow down\",\"status\":429,\"traceId\""),
            "{json}"
        );
    }

    #[test]
    fn it_is_sent_as_problem_json_with_its_status() {
        let response = Problem::new(StatusCode::CONFLICT).into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
    }

    #[test]
    fn the_detail_follows_the_status() {
        let json = text(
            Problem::new(StatusCode::BAD_REQUEST)
                .detail("Too big.")
                .with("code", "x"),
        );

        assert!(
            json.contains("\"status\":400,\"detail\":\"Too big.\",\"code\":\"x\""),
            "{json}"
        );
    }
}
