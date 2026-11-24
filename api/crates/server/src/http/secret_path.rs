//! Routes whose path carries a secret rather than an id, and what the log may
//! say of them.
//!
//! An invite's code is in the path of the routes that look at it and join with
//! it, and whoever holds it may join. The request line the pipeline logs (and
//! the panel's buffer keeps) says `***` in its place. A parameter of a route
//! is one of three things, and every parameter of every registered route has
//! to be named in one of the lists here: a test walks the routes and fails for
//! one that is in none, so a route with a new kind of segment cannot be added
//! without somebody deciding which it is.

use std::collections::HashSet;

/// The parameters whose value is a secret: shown as [`MARK`]. `request_id` is
/// the one of a desktop sign-in waiting in the browser: whoever opens its page
/// signed in binds the request to themselves.
pub const SECRETS: &[&str] = &["code", "request_id"];

/// What stands in the log for a secret.
pub const MARK: &str = "***";

/// The parameters that are ids, names of things and the like: whoever knows
/// one holds nothing by it.
pub const OPEN: &[&str] = &[
    "architecture",
    "backup_id",
    "bundle",
    "channel",
    "channelId",
    "conversationId",
    "current_version",
    "device_id",
    "emoji",
    "emoji_id",
    "id",
    "identityId",
    "identity_id",
    "invite_id",
    "kind",
    "messageId",
    "method_id",
    "name",
    "notificationId",
    "providerId",
    "provider_id",
    "requestId",
    "request_id",
    "roleId",
    "rootId",
    "section",
    "sectionId",
    "serverId",
    "server_id",
    "sessionId",
    "session_id",
    "sound_id",
    "target",
    "userId",
    "user_id",
    "conversation_id",
];

/// Every literal segment of the registered routes' patterns.
pub fn words<'a>(patterns: impl Iterator<Item = &'a str>) -> HashSet<String> {
    patterns
        .flat_map(|pattern| pattern.split('/'))
        .filter(|segment| !segment.is_empty() && !segment.starts_with('{'))
        .map(str::to_string)
        .collect()
}

/// The path of a request no route took, for the log: a segment is shown only
/// when the server knows it as a word of its own routes, so that nothing a
/// client typed (a code in the wrong place, another letter case, an escaped
/// slash) reaches a line. An empty segment is shown as it is.
pub fn masked_unmatched(words: &HashSet<String>, path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.is_empty() || words.contains(segment) {
                segment
            } else {
                MARK
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The names of the parameters in a route's pattern (`/api/x/{id}/y/{code}`).
pub fn parameters(pattern: &str) -> Vec<&str> {
    pattern
        .split('/')
        .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
        .map(|name| name.trim_start_matches('*'))
        .collect()
}

/// The path with each secret segment of the route it matched replaced by
/// [`MARK`].
pub fn masked(pattern: &str, path: &str) -> String {
    if !parameters(pattern)
        .iter()
        .any(|name| SECRETS.contains(name))
    {
        return path.to_string();
    }
    let mut shown: Vec<&str> = path.split('/').collect();
    for (at, segment) in pattern.split('/').enumerate() {
        let secret = segment
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
            .is_some_and(|name| SECRETS.contains(&name));
        if secret && let Some(slot) = shown.get_mut(at) {
            *slot = MARK;
        }
    }
    shown.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_segment_is_replaced_and_the_rest_is_kept() {
        assert_eq!(
            masked("/api/invites/{code}", "/api/invites/abc123"),
            "/api/invites/***"
        );
        assert_eq!(
            masked("/api/invites/{code}/join", "/api/invites/abc123/join"),
            "/api/invites/***/join"
        );
        assert_eq!(
            masked("/api/channels/{id}", "/api/channels/x"),
            "/api/channels/x"
        );
        assert_eq!(
            masked("/api/invites/{code}", "/api/invites/"),
            "/api/invites/***"
        );
    }

    #[test]
    fn every_parameter_of_every_route_is_an_id_or_a_secret_and_a_secret_is_masked() {
        let mut all = crate::http::Endpoints::new();
        for group in crate::groups() {
            all = all.merge(group);
        }
        let (_, rules) = all.into_parts();
        let mut unknown = std::collections::BTreeSet::new();
        let mut used = std::collections::BTreeSet::new();
        let mut secret_routes = std::collections::BTreeSet::new();
        for (_, pattern) in rules.keys() {
            for name in parameters(pattern) {
                used.insert(name);
                if SECRETS.contains(&name) {
                    secret_routes.insert(pattern.clone());
                } else if !OPEN.contains(&name) {
                    unknown.insert(format!("{pattern}: {name}"));
                }
            }
        }
        assert!(
            unknown.is_empty(),
            "a route has a parameter that is neither an id nor a secret (see http/secret_path.rs): {unknown:?}"
        );
        // A name nobody uses would let a future route with it pass undecided.
        let idle: Vec<&&str> = OPEN
            .iter()
            .chain(SECRETS)
            .filter(|name| !used.contains(**name))
            .collect();
        assert!(
            idle.is_empty(),
            "no route has a parameter called {idle:?}: take it off the list"
        );
        let secret_routes: Vec<String> = secret_routes.into_iter().collect();
        assert_eq!(
            secret_routes
                .iter()
                .filter(|route| route.contains("{code}"))
                .collect::<Vec<_>>(),
            ["/api/invites/{code}", "/api/invites/{code}/join"]
        );
        assert!(
            secret_routes
                .iter()
                .any(|route| route.contains("/desktop/browser/verify/{request_id}")),
            "{secret_routes:?}"
        );
    }

    #[test]
    fn a_path_no_route_took_shows_only_the_words_the_routes_themselves_use() {
        let words = words(
            [
                "/api/invites/{code}",
                "/api/invites/{code}/join",
                "/api/channels/{id}",
            ]
            .into_iter(),
        );
        for (path, shown) in [
            ("/api/invites//SECRET", "/api/invites//***"),
            ("/api/invites/SECRET/extra", "/api/invites/***/***"),
            ("/API/INVITES/SECRET", "/***/***/***"),
            ("/api/Invites/SECRET", "/api/***/***"),
            ("/api/invites%2FSECRET", "/api/***"),
            ("/api/invites/SECRET/JOIN", "/api/invites/***/***"),
            ("/invite/SECRET", "/***/***"),
            ("/api/channels/x", "/api/channels/***"),
            ("/", "/"),
        ] {
            assert_eq!(masked_unmatched(&words, path), shown, "{path}");
        }
    }
}
