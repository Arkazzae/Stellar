//! The cookies of the second step that are not the session: the one that marks
//! a remembered device, and the one that binds a passkey sign-in to the browser
//! that asked for its options. Neither can sign anybody in or authorize a call.
//!
//! The challenge cookie of a sign-in that waits for a second step lives with
//! the flow that makes it.

use axum::http::HeaderMap;

use super::cookies;
use crate::values::{Id, Timestamp};

fn name(https: bool, secure_name: &'static str, plain_name: &'static str) -> &'static str {
    if https { secure_name } else { plain_name }
}

pub fn trusted_device_name(https: bool) -> &'static str {
    name(https, "__Host-stellarr-trust", "stellarr-dev-trust")
}

pub fn passkey_name(https: bool) -> &'static str {
    name(https, "__Host-stellarr-passkey", "stellarr-dev-passkey")
}

/// The `Set-Cookie` line that marks the browser as a remembered device until
/// `expires`.
pub fn set_trusted_device(https: bool, token: &str, expires: Timestamp) -> String {
    cookies::set(trusted_device_name(https), token, Some(expires), https)
}

pub fn clear_trusted_device(https: bool) -> String {
    cookies::delete(trusted_device_name(https), https)
}

/// The token the browser holds, if it holds one.
pub fn read_trusted_device(headers: &HeaderMap, https: bool) -> Option<String> {
    cookies::read(headers, trusted_device_name(https))
}

/// The `Set-Cookie` line that binds a passkey sign-in to this browser.
pub fn set_passkey(https: bool, sign_in: Id, secret: &str, expires: Timestamp) -> String {
    cookies::set(
        passkey_name(https),
        &bound_value(sign_in, secret),
        Some(expires),
        https,
    )
}

pub fn clear_passkey(https: bool) -> String {
    cookies::delete(passkey_name(https), https)
}

/// The sign-in and the secret the browser holds, if its cookie is well formed.
pub fn read_passkey(headers: &HeaderMap, https: bool) -> Option<(Id, String)> {
    parse_bound(&cookies::read(headers, passkey_name(https))?)
}

/// The value of a cookie that names a row and holds the secret that proves the
/// browser is the one the row was made for: the id without dashes, a dot, the
/// secret.
pub fn bound_value(id: Id, secret: &str) -> String {
    let compact: String = id.to_string().chars().filter(|c| *c != '-').collect();
    format!("{compact}.{secret}")
}

/// The id and the secret of [`bound_value`], or nothing for any other text.
pub fn parse_bound(value: &str) -> Option<(Id, String)> {
    let (compact, secret) = value.split_once('.')?;
    if compact.len() != 32 || !compact.bytes().all(|b| b.is_ascii_hexdigit()) || secret.is_empty() {
        return None;
    }
    let dashed = format!(
        "{}-{}-{}-{}-{}",
        &compact[..8],
        &compact[8..12],
        &compact[12..16],
        &compact[16..20],
        &compact[20..]
    );
    Some((Id::parse(&dashed)?, secret.to_string()))
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderValue, header};

    use super::*;

    #[test]
    fn a_remembered_device_is_marked_by_a_cookie_named_for_the_scheme() {
        let expires = Timestamp::parse("2026-12-24 01:18:38").unwrap();

        assert_eq!(
            set_trusted_device(true, "token", expires),
            "__Host-stellarr-trust=token; expires=Thu, 24 Dec 2026 01:18:38 GMT; path=/; secure; samesite=lax; httponly"
        );
        assert_eq!(
            set_trusted_device(false, "token", expires),
            "stellarr-dev-trust=token; expires=Thu, 24 Dec 2026 01:18:38 GMT; path=/; samesite=lax; httponly"
        );
        assert_eq!(
            clear_trusted_device(true),
            "__Host-stellarr-trust=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/; secure; samesite=lax; httponly"
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("stellarr-dev-trust=abc; other=1"),
        );
        assert_eq!(read_trusted_device(&headers, false).as_deref(), Some("abc"));
        // The dev name is not the secure one, so a page cannot plant one for the other.
        assert_eq!(read_trusted_device(&headers, true), None);
    }

    #[test]
    fn a_passkey_sign_in_is_bound_by_the_id_and_the_secret_in_one_cookie() {
        let id = Id::parse("049083c7-a28f-4b91-b21b-2a0920dc1686").unwrap();
        let expires = Timestamp::parse("2026-11-24 01:23:38").unwrap();

        assert_eq!(
            set_passkey(true, id, "secret", expires),
            "__Host-stellarr-passkey=049083c7a28f4b91b21b2a0920dc1686.secret; expires=Tue, 24 Nov 2026 01:23:38 GMT; path=/; secure; samesite=lax; httponly"
        );
        assert_eq!(
            parse_bound("049083c7a28f4b91b21b2a0920dc1686.secret"),
            Some((id, "secret".to_string()))
        );
        assert_eq!(
            clear_passkey(false),
            "stellarr-dev-passkey=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/; samesite=lax; httponly"
        );
    }

    #[test]
    fn a_bound_value_that_is_not_one_names_nothing() {
        for bad in [
            "",
            ".secret",
            "049083c7a28f4b91b21b2a0920dc1686",
            "049083c7a28f4b91b21b2a0920dc1686.",
            "049083c7a28f4b91b21b2a0920dc168.secret",
            "049083c7a28f4b91b21b2a0920dc16866.secret",
            "049083c7a28f4b91b21b2a0920dc168g.secret",
            "049083c7-a28f-4b91-b21b-2a0920dc1686.secret",
        ] {
            assert_eq!(parse_bound(bad), None, "{bad:?}");
        }
    }
}
