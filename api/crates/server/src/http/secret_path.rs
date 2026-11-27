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
/// signed in binds the request to themselves. An activity file path can carry
/// a one-time launch ticket.
pub const SECRETS: &[&str] = &["asset_key", "code", "path", "request_id"];

/// What stands in the log for a secret.
pub const MARK: &str = "***";

/// The parameters that are ids, names of things and the like: whoever knows
/// one holds nothing by it.
pub const OPEN: &[&str] = &[
    "answerId",
    "architecture",
    "backup_id",
    "bundle",
    "channel",
    "channelId",
    "conversationId",
    "current_version",
    "device_id",
    "digest",
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
        let Some(parameter) = segment
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
        else {
            continue;
        };
        if SECRETS.contains(&parameter.trim_start_matches('*')) && at < shown.len() {
            shown[at] = MARK;
            if parameter.starts_with('*') {
                shown.truncate(at + 1);
                break;
            }
        }
    }
    shown.join("/")
}

#[cfg(test)]
mod tests;
