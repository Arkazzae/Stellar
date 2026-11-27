//! The permission bits, their names on the wire, and the checks on a mask that
//! every decision shares.
//!
//! A permission is one bit of a 64-bit mask, stored as a signed integer. A
//! stored mask may carry bits that no permission has (a row written by hand),
//! so nothing here assumes the high bits are clear.

pub const MANAGE_SERVER: i64 = 1 << 0;
pub const MANAGE_CHANNELS: i64 = 1 << 1;
pub const MANAGE_ROLES: i64 = 1 << 2;
pub const MANAGE_MEMBERS: i64 = 1 << 3;
pub const MANAGE_MESSAGES: i64 = 1 << 4;
pub const MODERATE_MEMBERS: i64 = 1 << 5;
pub const MANAGE_BANS: i64 = 1 << 6;
pub const VIEW_AUDIT_LOG: i64 = 1 << 7;
/// Every permission everywhere; no override applies to its holders.
pub const ADMINISTRATOR: i64 = 1 << 8;
pub const SEND_MESSAGES: i64 = 1 << 9;
pub const ATTACH_FILES: i64 = 1 << 10;
pub const ADD_REACTIONS: i64 = 1 << 11;
pub const CONNECT: i64 = 1 << 12;
pub const CREATE_INVITE: i64 = 1 << 13;
/// Seeing a channel at all. Somebody who does not hold it in a channel holds
/// nothing there, and the channel does not exist for them.
pub const VIEW_CHANNEL: i64 = 1 << 14;
/// Creating events on the server, and managing one's own.
pub const CREATE_EVENTS: i64 = 1 << 15;
/// Managing everybody's events on the server.
pub const MANAGE_EVENTS: i64 = 1 << 16;
pub const NONE: i64 = 0;
/// Every permission there is.
pub const ALL: i64 = 0x1_ffff;
/// The permissions that are the server's alone, which no channel or section
/// override can give or take: what an administrator is, and what events
/// allow, since an event belongs to the server rather than to a channel.
pub const SERVER_ONLY: i64 = ADMINISTRATOR | CREATE_EVENTS | MANAGE_EVENTS;
/// What a plain member of a new server holds: see channels, send messages,
/// attach files, add reactions, take a seat in voice.
pub const BASE_ROLE_DEFAULTS: i64 =
    VIEW_CHANNEL | SEND_MESSAGES | ATTACH_FILES | ADD_REACTIONS | CONNECT;
/// What an override can change: everything but the server-wide permissions.
/// A stored mask that has one is read as if it had not.
pub const OVERRIDABLE: i64 = ALL & !SERVER_ONLY;

/// In the order lists are written in. The newer names come after the older
/// ones, so a list a client already knows keeps its order.
const NAMES: [(i64, &str); 17] = [
    (MANAGE_SERVER, "manageServer"),
    (MANAGE_CHANNELS, "manageChannels"),
    (MANAGE_ROLES, "manageRoles"),
    (MANAGE_MEMBERS, "manageMembers"),
    (MANAGE_MESSAGES, "manageMessages"),
    (MODERATE_MEMBERS, "moderateMembers"),
    (MANAGE_BANS, "manageBans"),
    (VIEW_AUDIT_LOG, "viewAuditLog"),
    (ADMINISTRATOR, "administrator"),
    (SEND_MESSAGES, "sendMessages"),
    (ATTACH_FILES, "attachFiles"),
    (ADD_REACTIONS, "addReactions"),
    (CONNECT, "connect"),
    (CREATE_INVITE, "createInvite"),
    (VIEW_CHANNEL, "viewChannel"),
    (CREATE_EVENTS, "createEvents"),
    (MANAGE_EVENTS, "manageEvents"),
];

/// The wire name of one permission; none for a mask that is not exactly one
/// of them.
pub fn name_of(permission: i64) -> Option<&'static str> {
    NAMES
        .iter()
        .find(|(bit, _)| *bit == permission)
        .map(|(_, name)| *name)
}

/// The wire names of the permissions set in `mask`.
pub fn to_names(mask: i64) -> Vec<&'static str> {
    NAMES
        .iter()
        .filter(|(bit, _)| mask & bit == *bit)
        .map(|(_, name)| *name)
        .collect()
}

/// Reads wire names back. `None` when any of them is unknown, with nothing
/// granted. Names are matched exactly, in their own case.
pub fn parse_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<i64> {
    let mut parsed = NONE;
    for name in names {
        let (bit, _) = NAMES.iter().find(|(_, known)| *known == name)?;
        parsed |= bit;
    }
    Some(parsed)
}

/// Whether the mask has no bit that is not a permission.
pub fn is_valid(mask: i64) -> bool {
    mask & !ALL == 0
}

/// Whether `held` has every bit of `required`. A requirement with a bit that
/// is not a permission is never met.
pub fn contains(held: i64, required: i64) -> bool {
    is_valid(required) && held & required == required
}

/// What an override may say: known bits, none both allowed and denied, and
/// none of the server's alone.
pub fn is_overridable(allow: i64, deny: i64) -> bool {
    is_valid(allow | deny) && allow & deny == 0 && (allow | deny) & SERVER_ONLY == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_NAMES: [&str; 17] = [
        "manageServer",
        "manageChannels",
        "manageRoles",
        "manageMembers",
        "manageMessages",
        "moderateMembers",
        "manageBans",
        "viewAuditLog",
        "administrator",
        "sendMessages",
        "attachFiles",
        "addReactions",
        "connect",
        "createInvite",
        "viewChannel",
        "createEvents",
        "manageEvents",
    ];

    #[test]
    fn privileges_round_trip_through_their_camel_case_names() {
        assert_eq!(to_names(ALL), ALL_NAMES);
        assert!(to_names(NONE).is_empty());
        assert_eq!(
            to_names(MANAGE_ROLES | VIEW_AUDIT_LOG),
            ["manageRoles", "viewAuditLog"]
        );
        assert_eq!(parse_names(ALL_NAMES), Some(ALL));
        assert_eq!(parse_names(["manageBans", "manageBans"]), Some(MANAGE_BANS));
        assert_eq!(parse_names([]), Some(NONE));
    }

    #[test]
    fn an_unknown_name_is_refused_with_nothing_granted() {
        for name in ["superuser", "ManageServer", ""] {
            assert_eq!(parse_names(["manageServer", name]), None, "{name:?}");
        }
    }

    #[test]
    fn the_constants_are_the_ones_stored_rows_have() {
        // `Member` and `Owner` are stored as permission masks 24064 and
        // 131071; owners made before events were stored as 32767, and the
        // events migration gives them the two bits since.
        assert_eq!(BASE_ROLE_DEFAULTS, 24064);
        assert_eq!(ALL, 131_071);
    }

    #[test]
    fn a_requirement_with_an_unknown_bit_is_never_met() {
        assert!(contains(ALL, MANAGE_BANS | VIEW_CHANNEL));
        assert!(!contains(ALL, 1 << 20));
        assert!(!contains(-1, 1 << 20));
        assert!(contains(NONE, NONE));
    }

    #[test]
    fn an_override_names_known_bits_never_both_ways_and_never_administrator() {
        assert!(is_overridable(MANAGE_BANS, VIEW_AUDIT_LOG));
        assert!(!is_overridable(MANAGE_BANS, MANAGE_BANS));
        assert!(!is_overridable(ADMINISTRATOR, NONE));
        assert!(!is_overridable(NONE, ADMINISTRATOR));
        assert!(!is_overridable(CREATE_EVENTS, NONE), "events are the server's");
        assert!(!is_overridable(NONE, MANAGE_EVENTS));
        assert!(!is_overridable(1 << 20, NONE));
    }
}
