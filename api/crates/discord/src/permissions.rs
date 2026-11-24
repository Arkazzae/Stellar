//! The one place the import reads Discord's permission bits itself: whether a
//! channel is hidden from everyone. Overwrites are not imported, so a channel
//! hidden on Discord would be open to the whole Stellarr server until a later
//! stage closes it; the import has to know which those are.

use crate::snapshot::{ChannelSnapshot, OverwriteTarget, RoleSnapshot};

/// Discord's permission bit to see a channel.
pub const VIEW_CHANNEL: u64 = 1 << 10;

/// Reads a permission set, which Discord sends as decimal text: digits only,
/// and no more than 64 bits. Anything else holds no permission at all, and
/// that is the safe reading of a value that cannot be trusted.
pub fn permission_bits(text: &str) -> u64 {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return 0;
    }
    text.parse().unwrap_or(0)
}

/// Whether everyone is kept out of a channel: the role every member has
/// cannot see it, by the server's grant or by the channel's own overwrite.
/// Within the overwrite an allow beats a deny, as on Discord.
pub fn hidden_from_everyone(channel: &ChannelSnapshot, everyone: Option<&RoleSnapshot>) -> bool {
    let Some(everyone) = everyone else {
        return false;
    };

    let mut visible = permission_bits(&everyone.permissions) & VIEW_CHANNEL != 0;
    if let Some(overwrite) = channel
        .overwrites
        .iter()
        .find(|x| x.target == OverwriteTarget::Role && x.target_id == everyone.id)
    {
        if permission_bits(&overwrite.deny) & VIEW_CHANNEL != 0 {
            visible = false;
        }
        if permission_bits(&overwrite.allow) & VIEW_CHANNEL != 0 {
            visible = true;
        }
    }
    !visible
}

/// Orders snowflakes as the numbers they are; text order would put 9 after
/// 10. Ids that are not numbers order by length and then by text.
pub fn compare_ids(left: &str, right: &str) -> std::cmp::Ordering {
    match (digits_value(left), digits_value(right)) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => left
            .len()
            .cmp(&right.len())
            .then_with(|| left.as_bytes().cmp(right.as_bytes())),
    }
}

fn digits_value(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
