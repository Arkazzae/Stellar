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

/// The parameters whose value is a secret: shown as [`MARK`].
pub const SECRETS: [&str; 1] = ["code"];

/// What stands in the log for a secret.
pub const MARK: &str = "***";

/// The parameters that are ids, names of things and the like: whoever knows
/// one holds nothing by it.
pub const OPEN: &[&str] = &[
    "architecture",
    "backup_id",
    "attachment",
    "bundle",
    "channel",
    "channelId",
    "collection",
    "conversation",
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
    "picture",
    "providerId",
    "provider_id",
    "request",
    "requestId",
    "request_id",
    "result",
    "returned",
    "roleId",
    "rootId",
    "section",
    "sectionId",
    "server",
    "serverId",
    "server_id",
    "sessionId",
    "session_id",
    "sound_id",
    "target",
    "user",
    "userId",
    "user_id",
    "conversation_id",
];

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
        let mut secret_routes = std::collections::BTreeSet::new();
        for (_, pattern) in rules.keys() {
            for name in parameters(pattern) {
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
        assert_eq!(
            secret_routes.into_iter().collect::<Vec<_>>(),
            ["/api/invites/{code}", "/api/invites/{code}/join"]
        );
    }
}
