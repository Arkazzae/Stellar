//! `Set-Cookie` lines and the `Cookie` header, as the browser and the web
//! client see them.
//!
//! Every cookie the API sets has the attributes `path=/`, `samesite=lax` and
//! `httponly`, in that order, with `secure` before `samesite` when it applies
//! and `expires` first when the cookie outlives the browser.

use axum::http::{HeaderMap, header};

use crate::values::Timestamp;

/// A cookie to set. `expires` is left out for a cookie the browser drops when
/// it closes.
pub fn set(name: &str, value: &str, expires: Option<Timestamp>, secure: bool) -> String {
    let mut line = format!("{name}={value}");
    if let Some(expires) = expires {
        line.push_str(&format!("; expires={}", expires.to_http_date()));
    }
    line.push_str("; path=/");
    if secure {
        line.push_str("; secure");
    }
    line.push_str("; samesite=lax; httponly");
    line
}

/// A cookie to take away: empty, and expired at the start of 1970.
pub fn delete(name: &str, secure: bool) -> String {
    let epoch = Timestamp::from_unix_seconds(0).expect("the epoch is a moment");
    set(name, "", Some(epoch), secure)
}

/// The value of the cookie called `name`, if the request sent one.
pub fn read(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(have, _)| have.trim() == name)
        .map(|(_, value)| value.trim().trim_matches('"').to_string())
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn a_cookie_has_its_attributes_in_the_order_browsers_have_always_been_sent() {
        let expires = Timestamp::parse("2026-12-24 01:18:38").unwrap();

        assert_eq!(
            set("__Host-stellarr-session", "abc", Some(expires), true),
            "__Host-stellarr-session=abc; expires=Thu, 24 Dec 2026 01:18:38 GMT; path=/; secure; samesite=lax; httponly"
        );
        assert_eq!(
            set("stellarr-dev-session", "abc", None, false),
            "stellarr-dev-session=abc; path=/; samesite=lax; httponly"
        );
        assert_eq!(
            delete("__Host-stellarr-session", true),
            "__Host-stellarr-session=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/; secure; samesite=lax; httponly"
        );
    }

    #[test]
    fn a_cookie_is_found_among_several_in_one_header_or_in_two() {
        let mut headers = HeaderMap::new();
        headers.append(header::COOKIE, HeaderValue::from_static("a=1; b=2"));
        headers.append(header::COOKIE, HeaderValue::from_static("c=\"3\""));

        assert_eq!(read(&headers, "b").as_deref(), Some("2"));
        assert_eq!(read(&headers, "c").as_deref(), Some("3"));
        assert_eq!(read(&headers, "d"), None);
        assert_eq!(read(&headers, "a=1"), None);
    }
}
