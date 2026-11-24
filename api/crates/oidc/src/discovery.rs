//! The provider's discovery document: what it says about itself, and the
//! checks that make it safe to follow.

use std::fmt;

use serde_json::{Map, Value};

use crate::MAX_DOCUMENT;
use crate::options::{canonical_issuer, is_https_url};

/// Where the document lives, from the authority as configured.
pub fn discovery_url(authority: &str) -> String {
    format!(
        "{}/.well-known/openid-configuration",
        authority.trim_end_matches('/')
    )
}

/// What this crate needs from a discovery document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    /// Absent when the provider has none; the profile then comes from the token alone.
    pub userinfo_endpoint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryError {
    TooLarge,
    NotJson,
    MissingIssuer,
    /// The document names another issuer than the authority it was fetched from.
    IssuerMismatch,
    /// A required endpoint is missing, not a string, or not an https address.
    BadEndpoint(&'static str),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("the discovery document is too large"),
            Self::NotJson => f.write_str("the discovery document is not a JSON object"),
            Self::MissingIssuer => f.write_str("the discovery document names no issuer"),
            Self::IssuerMismatch => {
                f.write_str("the discovery document names another issuer than the authority")
            }
            Self::BadEndpoint(name) => write!(f, "the discovery document has no usable {name}"),
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// Reads the document fetched from [`discovery_url`] for `authority`.
pub fn parse(body: &[u8], authority: &str) -> Result<Discovery, DiscoveryError> {
    if body.len() > MAX_DOCUMENT {
        return Err(DiscoveryError::TooLarge);
    }
    let Ok(Value::Object(doc)) = serde_json::from_slice::<Value>(body) else {
        return Err(DiscoveryError::NotJson);
    };
    let issuer = doc
        .get("issuer")
        .and_then(Value::as_str)
        .filter(|issuer| !issuer.trim().is_empty())
        .ok_or(DiscoveryError::MissingIssuer)?;
    // A document that speaks for another issuer must not decide whose tokens
    // are good. A trailing slash is the one difference providers disagree on.
    if canonical_issuer(issuer) != canonical_issuer(authority) {
        return Err(DiscoveryError::IssuerMismatch);
    }
    Ok(Discovery {
        issuer: issuer.to_string(),
        authorization_endpoint: endpoint(&doc, "authorization_endpoint")?,
        token_endpoint: endpoint(&doc, "token_endpoint")?,
        jwks_uri: endpoint(&doc, "jwks_uri")?,
        userinfo_endpoint: match doc.get("userinfo_endpoint") {
            None | Some(Value::Null) => None,
            Some(Value::String(url)) if url.is_empty() => None,
            Some(_) => Some(endpoint(&doc, "userinfo_endpoint")?),
        },
    })
}

fn endpoint(doc: &Map<String, Value>, name: &'static str) -> Result<String, DiscoveryError> {
    doc.get(name)
        .and_then(Value::as_str)
        .filter(|url| url.len() <= 2048 && is_https_url(url))
        .map(str::to_string)
        .ok_or(DiscoveryError::BadEndpoint(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
        let mut value = serde_json::json!({
            "issuer": "https://identity.test",
            "authorization_endpoint": "https://identity.test/authorize",
            "token_endpoint": "https://identity.test/token",
            "jwks_uri": "https://identity.test/jwks",
            "userinfo_endpoint": "https://identity.test/userinfo",
            "id_token_signing_alg_values_supported": ["RS256"],
        });
        edit(&mut value);
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn the_address_is_built_from_the_authority() {
        assert_eq!(
            discovery_url("https://a.test/application/o/stellarr/"),
            "https://a.test/application/o/stellarr/.well-known/openid-configuration"
        );
    }

    #[test]
    fn reads_a_complete_document() {
        let found = parse(&doc(|_| {}), "https://identity.test").unwrap();
        assert_eq!(found.issuer, "https://identity.test");
        assert_eq!(found.token_endpoint, "https://identity.test/token");
        assert_eq!(
            found.userinfo_endpoint.as_deref(),
            Some("https://identity.test/userinfo")
        );
    }

    #[test]
    fn userinfo_is_optional() {
        for edit in [
            (|v: &mut Value| {
                v.as_object_mut().unwrap().remove("userinfo_endpoint");
            }) as fn(&mut Value),
            |v| v["userinfo_endpoint"] = Value::Null,
            |v| v["userinfo_endpoint"] = Value::String(String::new()),
        ] {
            assert_eq!(
                parse(&doc(edit), "https://identity.test")
                    .unwrap()
                    .userinfo_endpoint,
                None
            );
        }
        let wrong = doc(|v| v["userinfo_endpoint"] = Value::from(5));
        assert_eq!(
            parse(&wrong, "https://identity.test"),
            Err(DiscoveryError::BadEndpoint("userinfo_endpoint"))
        );
    }

    #[test]
    fn a_trailing_slash_on_either_side_is_the_same_issuer() {
        let slashed = doc(|v| v["issuer"] = "https://identity.test/".into());
        assert_eq!(
            parse(&slashed, "https://identity.test").unwrap().issuer,
            "https://identity.test/"
        );
        assert!(parse(&doc(|_| {}), "https://identity.test/").is_ok());
    }

    #[test]
    fn refuses_an_issuer_other_than_the_authority() {
        let other = doc(|v| v["issuer"] = "https://evil.test".into());
        assert_eq!(
            parse(&other, "https://identity.test"),
            Err(DiscoveryError::IssuerMismatch)
        );
        let missing = doc(|v| {
            v.as_object_mut().unwrap().remove("issuer");
        });
        assert_eq!(
            parse(&missing, "https://identity.test"),
            Err(DiscoveryError::MissingIssuer)
        );
        let blank = doc(|v| v["issuer"] = " ".into());
        assert_eq!(
            parse(&blank, "https://identity.test"),
            Err(DiscoveryError::MissingIssuer)
        );
    }

    #[test]
    fn refuses_endpoints_that_are_missing_or_not_https() {
        for name in ["authorization_endpoint", "token_endpoint", "jwks_uri"] {
            let missing = doc(|v| {
                v.as_object_mut().unwrap().remove(name);
            });
            assert_eq!(
                parse(&missing, "https://identity.test"),
                Err(DiscoveryError::BadEndpoint(name))
            );
            for bad in [
                "http://identity.test/x",
                "/relative",
                "https://u@identity.test/x",
                "https://identity.test/#f",
                "",
            ] {
                let broken = doc(|v| v[name] = bad.into());
                assert_eq!(
                    parse(&broken, "https://identity.test"),
                    Err(DiscoveryError::BadEndpoint(name)),
                    "{name} {bad}"
                );
            }
            let number = doc(|v| v[name] = 1.into());
            assert!(parse(&number, "https://identity.test").is_err());
        }
        let long = doc(|v| {
            v["token_endpoint"] = format!("https://identity.test/{}", "a".repeat(2048)).into()
        });
        assert!(parse(&long, "https://identity.test").is_err());
    }

    #[test]
    fn refuses_malformed_and_oversized_input() {
        for body in [
            &b""[..],
            b"[]",
            b"null",
            b"\"x\"",
            b"{",
            b"{\"issuer\":",
            b"\xff\xfe",
        ] {
            assert_eq!(
                parse(body, "https://identity.test"),
                Err(DiscoveryError::NotJson),
                "{body:?}"
            );
        }
        let huge = vec![b' '; MAX_DOCUMENT + 1];
        assert_eq!(
            parse(&huge, "https://identity.test"),
            Err(DiscoveryError::TooLarge)
        );
    }
}
