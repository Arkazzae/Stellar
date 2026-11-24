//! The identity providers an account can sign in through.
//!
//! This server has no settings for one yet: the sign-in through a provider is
//! another group's, and it will bring them. Until then no provider is
//! configured, so a linked identity is named by the host of its issuer and
//! counts as a way in for nobody. This is the one place that changes when the
//! settings arrive.

use crate::auth::ways_in::OwnedProviders;
use crate::state::AppState;

/// The id of the provider an identity belongs to when its row names none.
pub const DEFAULT_ID: &str = "default";

/// A provider the instance is set up with.
pub struct Provider {
    pub id: String,
    pub display_name: String,
}

pub fn configured(_state: &AppState) -> Vec<Provider> {
    Vec::new()
}

/// What `auth::ways_in` asks about, for these providers.
pub fn owned_of(configured: &[Provider]) -> OwnedProviders {
    OwnedProviders {
        configured: configured.iter().map(|provider| provider.id.clone()).collect(),
        default_id: DEFAULT_ID.to_string(),
    }
}

/// What a linked identity is called: its provider's name while that is
/// configured, else the name of its issuer.
pub fn identity_name(configured: &[Provider], provider: &str, issuer: &str) -> String {
    match configured.iter().find(|candidate| candidate.id == provider) {
        Some(found) => found.display_name.clone(),
        None => issuer_name(issuer),
    }
}

/// How a linked identity is named when its provider is no longer configured:
/// the host of its issuer, or the issuer as it was written when that is no
/// address.
pub fn issuer_name(issuer: &str) -> String {
    let Some((scheme, rest)) = issuer.split_once("://") else {
        return issuer.to_string();
    };
    let valid_scheme = scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid_scheme {
        return issuer.to_string();
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_and_port = authority.rsplit('@').next().unwrap_or("");
    let host = if host_and_port.starts_with('[') {
        host_and_port.split_once(']').map_or(host_and_port, |(inside, _)| &host_and_port[..=inside.len()])
    } else {
        host_and_port.split(':').next().unwrap_or("")
    };
    host.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_issuer_is_named_by_its_host() {
        assert_eq!(issuer_name("https://Id.Example.test/realms/main"), "id.example.test");
        assert_eq!(issuer_name("https://user@id.example.test:8443/x"), "id.example.test");
        assert_eq!(issuer_name("https://[::1]:8443/x"), "[::1]");
        assert_eq!(issuer_name("not an address"), "not an address");
        assert_eq!(issuer_name("1x://host"), "1x://host");
    }
}
