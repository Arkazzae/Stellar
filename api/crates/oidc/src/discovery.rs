//! The provider's discovery document: what it says about itself, and the
//! checks that make it safe to follow.

use std::fmt;

use serde_json::{Map, Value};

use crate::MAX_DOCUMENT;
use crate::options::is_https_url;

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
    /// A required endpoint is missing, not a string, or not an https address.
    BadEndpoint(&'static str),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("the discovery document is too large"),
            Self::NotJson => f.write_str("the discovery document is not a JSON object"),
            Self::MissingIssuer => f.write_str("the discovery document names no issuer"),
            Self::BadEndpoint(name) => write!(f, "the discovery document has no usable {name}"),
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// Reads the document fetched from [`discovery_url`]. Its issuer need not
/// equal the authority it was fetched from (a proxy may sit between); the ID
/// token's issuer must equal the issuer named here exactly.
pub fn parse(body: &[u8]) -> Result<Discovery, DiscoveryError> {
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
        .map(str::trim)
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
        let found = parse(&doc(|_| {})).unwrap();
        assert_eq!(found.issuer, "https://identity.test");
        assert_eq!(found.token_endpoint, "https://identity.test/token");
        assert_eq!(
            found.userinfo_endpoint.as_deref(),
            Some("https://identity.test/userinfo")
        );
    }

    #[test]
    fn endpoints_are_trimmed() {
        let spaced = doc(|v| v["token_endpoint"] = " https://identity.test/token ".into());
        assert_eq!(
            parse(&spaced).unwrap().token_endpoint,
            "https://identity.test/token"
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
            assert_eq!(parse(&doc(edit)).unwrap().userinfo_endpoint, None);
        }
        let wrong = doc(|v| v["userinfo_endpoint"] = Value::from(5));
        assert_eq!(
            parse(&wrong),
            Err(DiscoveryError::BadEndpoint("userinfo_endpoint"))
        );
    }

    #[test]
    fn the_issuer_may_differ_from_the_authority_it_was_fetched_from() {
        // Behind a proxy the two can legitimately differ; the token's issuer
        // is compared with this one, not with the authority.
        for issuer in ["https://identity.test/", "https://other.test/realm"] {
            let doc = doc(|v| v["issuer"] = issuer.into());
            assert_eq!(parse(&doc).unwrap().issuer, issuer);
        }
    }

    #[test]
    fn refuses_a_missing_issuer() {
        let missing = doc(|v| {
            v.as_object_mut().unwrap().remove("issuer");
        });
        assert_eq!(parse(&missing), Err(DiscoveryError::MissingIssuer));
        let blank = doc(|v| v["issuer"] = " ".into());
        assert_eq!(parse(&blank), Err(DiscoveryError::MissingIssuer));
    }

    #[test]
    fn refuses_endpoints_that_are_missing_or_not_https() {
        for name in ["authorization_endpoint", "token_endpoint", "jwks_uri"] {
            let missing = doc(|v| {
                v.as_object_mut().unwrap().remove(name);
            });
            assert_eq!(parse(&missing), Err(DiscoveryError::BadEndpoint(name)));
            for bad in [
                "http://identity.test/x",
                "/relative",
                "https://u@identity.test/x",
                "https://identity.test/#f",
                "",
            ] {
                let broken = doc(|v| v[name] = bad.into());
                assert_eq!(
                    parse(&broken),
                    Err(DiscoveryError::BadEndpoint(name)),
                    "{name} {bad}"
                );
            }
            let number = doc(|v| v[name] = 1.into());
            assert!(parse(&number).is_err());
        }
        let long = doc(|v| {
            v["token_endpoint"] = format!("https://identity.test/{}", "a".repeat(2048)).into()
        });
        assert!(parse(&long).is_err());
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
            assert_eq!(parse(body), Err(DiscoveryError::NotJson), "{body:?}");
        }
        let huge = vec![b' '; MAX_DOCUMENT + 1];
        assert_eq!(parse(&huge), Err(DiscoveryError::TooLarge));
    }
}
