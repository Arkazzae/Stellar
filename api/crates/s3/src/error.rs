//! What went wrong, classified the way a caller decides what to do next.

use std::fmt;

use crate::{Headers, xml};

/// The most of an error body worth reading; callers should stop there.
pub const MAX_ERROR_BODY: usize = 64 * 1024;

/// The longest message kept, so a hostile server cannot fill a log line.
const MAX_MESSAGE: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// No object under the key. A `HEAD` cannot tell this from a missing
    /// bucket, because its answer has no body.
    NotFound,
    BucketNotFound,
    UploadNotFound,
    /// The requested range starts at or past the end of the object.
    RangeNotSatisfiable,
    /// The credentials are known but may not do this.
    AccessDenied,
    /// The access key is unknown or the signature was refused: a wrong key or
    /// secret, a wrong region, or an endpoint that signs differently.
    BadCredentials,
    /// The store's clock and ours differ by too much for the signature.
    ClockSkew,
    /// The bucket exists and is already ours.
    AlreadyOwned,
    /// The name is taken, or a conflicting operation is running.
    Conflict,
    PreconditionFailed,
    /// The store asks us to slow down. Retry later.
    Throttled,
    /// The store, or something in front of it, failed or timed out. Retry.
    Unavailable,
    /// Any other refusal. Retrying the same request will not help.
    Rejected,
    /// The answer is not what the call expects, or it is cut off.
    Malformed,
}

/// A failed call: what the store said and how to treat it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    /// The HTTP status of the answer, 0 when there was none to look at.
    pub status: u16,
    /// The `Code` of the error body, when the answer had one.
    pub code: Option<String>,
    pub message: Option<String>,
    pub request_id: Option<String>,
}

impl Error {
    /// Classifies a refused call from its status, its headers and as much of
    /// its body as was read. Any body, an HTML page from a proxy included, is
    /// safe to pass: what is not an S3 error document is ignored.
    pub fn from_response<H: Headers + ?Sized>(status: u16, headers: &H, body: &[u8]) -> Self {
        let mut error = Self {
            kind: ErrorKind::Rejected,
            status,
            code: None,
            message: None,
            request_id: headers.get("x-amz-request-id").map(|id| truncate(id, 128)),
        };
        if body.len() <= MAX_ERROR_BODY
            && let Ok(root) = xml::parse(body)
            && root.name == "Error"
        {
            error.code = root
                .child_text("Code")
                .map(|code| truncate(code.trim(), 128))
                .filter(|code| !code.is_empty());
            error.message = root
                .child_text("Message")
                .map(|text| truncate(text.trim(), MAX_MESSAGE));
            if error.request_id.is_none() {
                error.request_id = root
                    .child_text("RequestId")
                    .map(|id| truncate(id.trim(), 128));
            }
        }
        error.kind = classify(status, error.code.as_deref());
        error
    }

    pub fn malformed(what: &str) -> Self {
        Self {
            kind: ErrorKind::Malformed,
            status: 0,
            code: None,
            message: Some(what.to_owned()),
            request_id: None,
        }
    }

    /// Whether the same request may succeed if sent again after a pause.
    pub fn is_retryable(&self) -> bool {
        matches!(self.kind, ErrorKind::Throttled | ErrorKind::Unavailable)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the store answered {:?}", self.kind)?;
        if self.status != 0 {
            write!(f, " (HTTP {})", self.status)?;
        }
        if let Some(code) = &self.code {
            write!(f, " {code}")?;
        }
        if let Some(message) = &self.message {
            write!(f, ": {message}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn classify(status: u16, code: Option<&str>) -> ErrorKind {
    use ErrorKind::*;
    // The code is more exact than the status: 403 alone cannot tell a wrong
    // secret from a policy, and 409 cannot tell "yours" from "taken".
    if let Some(kind) = code.and_then(|code| match code {
        "NoSuchKey" => Some(NotFound),
        "NoSuchBucket" => Some(BucketNotFound),
        "NoSuchUpload" => Some(UploadNotFound),
        "InvalidRange" => Some(RangeNotSatisfiable),
        "AccessDenied" | "AllAccessDisabled" | "AccountProblem" | "Forbidden" | "NotSignedUp" => {
            Some(AccessDenied)
        }
        "InvalidAccessKeyId"
        | "SignatureDoesNotMatch"
        | "InvalidSecurity"
        | "ExpiredToken"
        | "InvalidToken"
        | "MissingSecurityHeader"
        | "AuthorizationHeaderMalformed" => Some(BadCredentials),
        "RequestTimeTooSkewed" => Some(ClockSkew),
        "BucketAlreadyOwnedByYou" => Some(AlreadyOwned),
        "BucketAlreadyExists" | "OperationAborted" | "ConditionalRequestConflict" => Some(Conflict),
        "PreconditionFailed" => Some(PreconditionFailed),
        "SlowDown"
        | "Throttling"
        | "ThrottlingException"
        | "RequestLimitExceeded"
        | "TooManyRequests"
        | "BandwidthLimitExceeded" => Some(Throttled),
        "InternalError" | "ServiceUnavailable" | "RequestTimeout" | "RequestTimeoutException" => {
            Some(Unavailable)
        }
        _ => None,
    }) {
        return kind;
    }
    match status {
        404 => NotFound,
        416 => RangeNotSatisfiable,
        401 => BadCredentials,
        403 => AccessDenied,
        409 => Conflict,
        412 => PreconditionFailed,
        429 | 503 => Throttled,
        408 | 500..=599 => Unavailable,
        _ => Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_body(status: u16, body: &str) -> Error {
        let headers: &[(&str, &str)] = &[];
        Error::from_response(status, headers, body.as_bytes())
    }

    fn error_doc(code: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>m</Message><RequestId>r1</RequestId></Error>"
        )
    }

    #[test]
    fn the_code_decides_before_the_status() {
        for (status, code, kind) in [
            (404, "NoSuchKey", ErrorKind::NotFound),
            (404, "NoSuchBucket", ErrorKind::BucketNotFound),
            (404, "NoSuchUpload", ErrorKind::UploadNotFound),
            (416, "InvalidRange", ErrorKind::RangeNotSatisfiable),
            (403, "AccessDenied", ErrorKind::AccessDenied),
            (403, "SignatureDoesNotMatch", ErrorKind::BadCredentials),
            (403, "InvalidAccessKeyId", ErrorKind::BadCredentials),
            (403, "RequestTimeTooSkewed", ErrorKind::ClockSkew),
            (409, "BucketAlreadyOwnedByYou", ErrorKind::AlreadyOwned),
            (409, "BucketAlreadyExists", ErrorKind::Conflict),
            (412, "PreconditionFailed", ErrorKind::PreconditionFailed),
            (503, "SlowDown", ErrorKind::Throttled),
            (500, "InternalError", ErrorKind::Unavailable),
            (400, "RequestTimeout", ErrorKind::Unavailable),
            (200, "InternalError", ErrorKind::Unavailable),
            (400, "InvalidArgument", ErrorKind::Rejected),
            (400, "EntityTooSmall", ErrorKind::Rejected),
        ] {
            let error = classify_body(status, &error_doc(code));
            assert_eq!(error.kind, kind, "{status} {code}");
            assert_eq!(error.code.as_deref(), Some(code));
            assert_eq!(error.request_id.as_deref(), Some("r1"));
        }
    }

    #[test]
    fn the_status_decides_when_there_is_no_usable_body() {
        for (status, kind) in [
            (404, ErrorKind::NotFound),
            (416, ErrorKind::RangeNotSatisfiable),
            (401, ErrorKind::BadCredentials),
            (403, ErrorKind::AccessDenied),
            (409, ErrorKind::Conflict),
            (412, ErrorKind::PreconditionFailed),
            (429, ErrorKind::Throttled),
            (503, ErrorKind::Throttled),
            (500, ErrorKind::Unavailable),
            (502, ErrorKind::Unavailable),
            (504, ErrorKind::Unavailable),
            (408, ErrorKind::Unavailable),
            (400, ErrorKind::Rejected),
            (301, ErrorKind::Rejected),
        ] {
            assert_eq!(classify_body(status, "").kind, kind, "{status}");
            assert_eq!(
                classify_body(status, "<html><body>nope</body></html>").kind,
                kind,
                "{status} html"
            );
            assert_eq!(
                classify_body(status, "<Error><Code>").kind,
                kind,
                "{status} cut off"
            );
        }
    }

    #[test]
    fn a_hostile_body_cannot_inflate_the_error() {
        let long = "x".repeat(10_000);
        let doc = format!("<Error><Code>{long}</Code><Message>{long}</Message></Error>");
        let error = classify_body(400, &doc);
        assert!(error.code.as_deref().is_some_and(|c| c.len() <= 128));
        assert!(
            error
                .message
                .as_deref()
                .is_some_and(|m| m.len() <= MAX_MESSAGE)
        );
        let huge = format!(
            "<Error><Code>NoSuchKey</Code><Message>{}</Message></Error>",
            "y".repeat(MAX_ERROR_BODY)
        );
        assert_eq!(classify_body(400, &huge).code, None);
    }

    #[test]
    fn only_slow_down_and_outages_are_worth_retrying() {
        assert!(classify_body(503, "").is_retryable());
        assert!(classify_body(500, "").is_retryable());
        assert!(!classify_body(404, "").is_retryable());
        assert!(!classify_body(403, "").is_retryable());
        assert_eq!(classify_body(404, "").kind, ErrorKind::NotFound);
    }

    #[test]
    fn the_header_request_id_wins() {
        let headers: &[(&str, &str)] = &[("X-Amz-Request-Id", "h1")];
        let error = Error::from_response(500, headers, error_doc("InternalError").as_bytes());
        assert_eq!(error.request_id.as_deref(), Some("h1"));
    }
}
