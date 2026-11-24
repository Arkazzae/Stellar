//! Where the browser is sent when a round trip to a provider ends, and the
//! cookie that ties the provider's answer to the browser that asked.

use aws_lc_rs::digest;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use stellarr_oidc::b64;
use stellarr_oidc::form::encode_component;
use stellarr_oidc::options::{Provider, safe_return_url};

use crate::auth::cookies;
use crate::values::Timestamp;

/// A redirect to `location`. An address a header cannot carry (it is on this
/// origin and has characters beyond ASCII) is sent as the root instead.
pub fn redirect(location: &str) -> Response {
    let value = Some(location)
        .filter(|location| location.is_ascii())
        .and_then(|location| HeaderValue::from_str(location).ok())
        .unwrap_or_else(|| HeaderValue::from_static("/"));
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::FOUND;
    response.headers_mut().insert(header::LOCATION, value);
    response
}

/// Keeps an answer out of every cache, as every route under `/api/auth` does.
pub fn no_store(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

/// Where a sign-in that did not work ends: the login page, saying roughly why.
pub fn login_failed(cancelled: bool) -> String {
    format!(
        "/login?oidc_error={}",
        if cancelled { "cancelled" } else { "failed" }
    )
}

/// Where a link ends, saying how it came out.
pub fn link_result(result: &str) -> String {
    format!("/link-account?result={result}")
}

/// Where a fresh sign-in asked for as proof ends, with the page it was asked
/// from.
pub fn reauthentication_page(confirmed: bool, return_url: Option<&str>) -> String {
    let page = link_result(if confirmed { "confirmed" } else { "unconfirmed" });
    let back = safe_return_url(return_url);
    if back == "/" {
        page
    } else {
        format!("{page}&returnUrl={}", encode_component(&back))
    }
}

/// Where a person goes to finish a sign-in that needs the instance's second
/// step, with the page they were headed for.
pub fn second_step_page(enrollment: bool, return_url: &str) -> String {
    let mut page = String::from(if enrollment {
        "/login?mfa_enrollment=1"
    } else {
        "/login?mfa_required=1"
    });
    if !return_url.trim().is_empty() {
        page.push_str("&returnUrl=");
        page.push_str(&encode_component(return_url));
    }
    page
}

/// Where a person goes when their account waits for an operator's recovery.
pub fn recovery_page(short_lived: bool) -> String {
    if short_lived {
        "/login?security_recovery_required=1&keep_signed_in=0".to_string()
    } else {
        "/login?security_recovery_required=1".to_string()
    }
}

/// Where a person whose identity no account has goes: the sign-in page, which
/// says what can be done.
pub fn unlinked_page(return_url: &str, short_lived: bool) -> String {
    let mut page = String::from("/login?oidc=unlinked");
    let back = safe_return_url(Some(return_url));
    if back != "/" {
        page.push_str("&returnUrl=");
        page.push_str(&encode_component(&back));
    }
    if short_lived {
        page.push_str("&keep_signed_in=0");
    }
    page
}

// ---- the cookie that ties an answer to its browser ----

/// The path the provider's answer is posted to, which is the only place the
/// cookie is sent.
fn callback_path(provider: &Provider) -> &str {
    &provider.callback_path
}

/// One cookie for each sign-in under way, named for its state, so that two
/// tabs can be in the middle of one each.
pub fn binding_name(state: &str) -> String {
    let hash = digest::digest(&digest::SHA256, state.as_bytes());
    let text = b64::encode(&hash.as_ref()[..16]);
    format!("stellarr-oidc-{text}")
}

fn binding_line(name: &str, value: &str, expires: Timestamp, provider: &Provider) -> String {
    // The provider answers with a post from another site, which only a cookie
    // marked for that comes along with.
    format!(
        "{name}={value}; expires={}; path={}; secure; samesite=none; httponly",
        expires.to_http_date(),
        callback_path(provider)
    )
}

pub fn set_binding(state: &str, binding: &str, expires: Timestamp, provider: &Provider) -> String {
    binding_line(&binding_name(state), binding, expires, provider)
}

pub fn clear_binding(state: &str, provider: &Provider) -> String {
    let epoch = Timestamp::from_unix_seconds(0).expect("the epoch is a moment");
    binding_line(&binding_name(state), "", epoch, provider)
}

pub fn read_binding(headers: &HeaderMap, state: &str) -> Option<String> {
    cookies::read(headers, &binding_name(state))
}

/// The `state` parameter of the address a sign-in redirects to.
pub fn state_of(redirect_to: &str) -> Option<&str> {
    let at = redirect_to
        .rfind("&state=")
        .or_else(|| redirect_to.rfind("?state="))?;
    let rest = &redirect_to[at + "&state=".len()..];
    Some(rest.split('&').next().unwrap_or(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_sign_in_asked_from_a_page_comes_back_to_it() {
        assert_eq!(reauthentication_page(true, None), "/link-account?result=confirmed");
        assert_eq!(
            reauthentication_page(false, Some("/settings?tab=security")),
            "/link-account?result=unconfirmed&returnUrl=%2Fsettings%3Ftab%3Dsecurity"
        );
        assert_eq!(
            reauthentication_page(false, Some("https://evil.test/")),
            "/link-account?result=unconfirmed"
        );
    }

    #[test]
    fn the_second_step_page_always_names_where_the_person_was_going() {
        assert_eq!(
            second_step_page(false, "/"),
            "/login?mfa_required=1&returnUrl=%2F"
        );
        assert_eq!(
            second_step_page(true, "/invite/abc?x=1"),
            "/login?mfa_enrollment=1&returnUrl=%2Finvite%2Fabc%3Fx%3D1"
        );
    }

    #[test]
    fn the_unlinked_page_names_a_return_address_only_when_it_is_not_the_root() {
        assert_eq!(unlinked_page("/", false), "/login?oidc=unlinked");
        assert_eq!(unlinked_page("/", true), "/login?oidc=unlinked&keep_signed_in=0");
        assert_eq!(
            unlinked_page("/a b", false),
            "/login?oidc=unlinked&returnUrl=%2Fa%20b"
        );
    }

    #[test]
    fn the_state_is_found_in_the_address_whatever_the_endpoint_looks_like() {
        let address = "https://id.test/a?x=1&state=ABC-_&code_challenge=zz";
        assert_eq!(state_of(address), Some("ABC-_"));
        assert_eq!(state_of("https://id.test/a?state=Q"), Some("Q"));
        assert_eq!(state_of("https://id.test/a"), None);
    }

    #[test]
    fn the_binding_cookie_is_for_the_callback_only_and_for_cross_site_posts() {
        let mut settings = stellarr_oidc::options::Settings::default();
        settings.original.enabled = true;
        settings.original.authority = "https://id.test".into();
        settings.original.client_id = "c".into();
        settings.original.client_secret = "s".into();
        let provider = settings.find("default", |_| None).unwrap().unwrap();
        let expires = Timestamp::parse("2026-11-24 10:26:16").unwrap();

        let line = set_binding("state", "value", expires, &provider);

        assert!(line.starts_with("stellarr-oidc-"), "{line}");
        assert!(line.ends_with("=value; expires=Tue, 24 Nov 2026 10:26:16 GMT; path=/api/auth/oidc/callback; secure; samesite=none; httponly"), "{line}");
        assert_ne!(binding_name("a"), binding_name("b"));
    }
}
