//! Timestamps as Discord writes them, snowflakes, and the permission bits.

use stellarr_discord::{
    ChannelKind, ChannelSnapshot, DiscordChannelType, OverwriteTarget, PermissionOverwriteSnapshot,
    RoleSnapshot, Timestamp, VIEW_CHANNEL, compare_ids, hidden_from_everyone, parse_timestamp,
    permission_bits, snowflake_time,
};

fn ticks(t: Timestamp) -> i128 {
    i128::from(t.unix_seconds()) * 10_000_000 + i128::from(t.subsec_nanos() / 100)
}

fn at(text: &str) -> i128 {
    ticks(parse_timestamp(text).unwrap_or_else(|| panic!("{text}")))
}

#[test]
fn reads_the_timestamps_discord_writes() {
    // Seconds since the epoch of the first four were worked out with Python's datetime.
    assert_eq!(at("2024-01-01T00:00:00+00:00"), 1_704_067_200 * 10_000_000);
    assert_eq!(at("2024-01-01T00:00:00Z"), 1_704_067_200 * 10_000_000);
    assert_eq!(at("2024-01-01T00:00:00"), 1_704_067_200 * 10_000_000);
    assert_eq!(at("2017-07-11T17:27:07.299000+00:00"), 1_499_794_027_299 * 10_000);
    assert_eq!(at("2024-01-01T02:00:00+02:00"), 1_704_067_200 * 10_000_000);
    assert_eq!(at("2023-12-31T19:00:00-05:00"), 1_704_067_200 * 10_000_000);
    assert_eq!(at("2024-01-01T00:00:00.1234567+00:00"), 1_704_067_200 * 10_000_000 + 1_234_567);
    // Past the seventh digit the value rounds, as a tick holds no more.
    assert_eq!(at("2024-01-01T00:00:00.12345675Z"), 1_704_067_200 * 10_000_000 + 1_234_568);
    assert_eq!(at("2024-01-01T00:00:00.99999999Z"), (1_704_067_200 + 1) * 10_000_000);
    assert_eq!(at("2024-01-01"), 1_704_067_200 * 10_000_000);
    assert_eq!(at("2024-01-01T00:00:00.000000 +01:00"), 1_704_063_600 * 10_000_000);
    assert_eq!(at("1969-12-31T23:59:59Z"), -10_000_000);
    assert_eq!(at("0001-01-01T00:00:00Z"), -62_135_596_800 * 10_000_000);
    assert_eq!(at("9999-12-31T23:59:59Z"), 253_402_300_799 * 10_000_000);
    assert_eq!(at("2024-02-29T00:00:00Z"), 1_709_164_800 * 10_000_000);
}

#[test]
fn one_odd_date_is_none_and_never_a_panic() {
    for text in [
        "", "x", "last tuesday", "2023-02-29T00:00:00Z", "2024-13-01", "2024-00-10", "2024-01-32",
        "2024-01-01T24:00:00Z", "2024-01-01T12:60:00Z", "2024-01-01T12:00:60Z", "2024-01-01T00:00:00+15:00",
        "2024-01-01T00:00:00+00:60", "2024-01-01T00:00:00junk", "0000-01-01", "10000-01-01",
        "2024-01-01T00:00:00.", "2024-01-01T00:00:00.Z", "20240101", "-2024-01-01", "2024-01-01T",
        "2024\u{ff0d}01-01", "\u{0662}024-01-01", "\u{1F600}",
        // Past the last moment a timestamp holds, by way of the offset.
        "9999-12-31T23:59:59-01:00", "0001-01-01T00:00:00+01:00",
    ] {
        assert_eq!(parse_timestamp(text), None, "{text:?}");
    }
    let long = "2024-01-01T00:00:00.".to_owned() + &"1".repeat(100_000) + "Z";
    assert!(parse_timestamp(&long).is_some());
}

#[test]
fn a_snowflake_carries_the_time_it_was_made() {
    assert_eq!(
        snowflake_time("175928847299117063").map(|t| t.unix_millis()),
        Some(1_462_015_105_796)
    );
    assert_eq!(snowflake_time("0").map(|t| t.unix_millis()), Some(1_420_070_400_000));
    // The largest snowflake is in the year 2155.
    assert!(snowflake_time("18446744073709551615").is_some());
    for text in ["", "abc", "-1", "+1", "1.5", "18446744073709551616", " 1"] {
        assert_eq!(snowflake_time(text), None, "{text:?}");
    }
}

#[test]
fn ids_order_as_the_numbers_they_are() {
    use std::cmp::Ordering::{Equal, Greater, Less};
    assert_eq!(compare_ids("9", "10"), Less);
    assert_eq!(compare_ids("1000", "999"), Greater);
    assert_eq!(compare_ids("0007", "7"), Equal);
    assert_eq!(compare_ids("18446744073709551615", "18446744073709551614"), Greater);
    // Not numbers: by length, then by text.
    assert_eq!(compare_ids("abc", "ab"), Greater);
    assert_eq!(compare_ids("abc", "abd"), Less);
    assert_eq!(compare_ids("99999999999999999999999", "1"), Greater);
}

/* ------------------------------------------------------------ permissions */

#[test]
fn permission_bits_are_digits_that_fit_sixty_four_bits_or_nothing() {
    assert_eq!(permission_bits("0"), 0);
    assert_eq!(permission_bits("1024"), 1024);
    assert_eq!(permission_bits("2248329584434769"), 2_248_329_584_434_769);
    assert_eq!(permission_bits("18446744073709551615"), u64::MAX);
    assert_eq!(permission_bits("0001024"), 1024);
    for text in [
        "", "-1", "+1024", " 1024", "1024 ", "1e3", "1.0", "0x400", "18446744073709551616", "1_024", "\u{0661}",
        "1024\n",
    ] {
        assert_eq!(permission_bits(text), 0, "{text:?}");
    }
}

fn role(permissions: &str) -> RoleSnapshot {
    RoleSnapshot {
        id: "100".into(),
        name: "@everyone".into(),
        color: None,
        position: 0,
        hoisted: false,
        mentionable: false,
        managed: false,
        permissions: permissions.into(),
        is_everyone: true,
    }
}

fn channel(overwrites: Vec<PermissionOverwriteSnapshot>) -> ChannelSnapshot {
    ChannelSnapshot {
        id: "1".into(),
        name: "general".into(),
        kind: ChannelKind::Text,
        discord_type: DiscordChannelType::Conversation,
        topic: None,
        slow_mode_seconds: 0,
        user_limit: 0,
        position: 0,
        section_id: None,
        overwrites,
    }
}

fn overwrite(target: OverwriteTarget, id: &str, allow: &str, deny: &str) -> PermissionOverwriteSnapshot {
    PermissionOverwriteSnapshot {
        target,
        target_id: id.into(),
        allow: allow.into(),
        deny: deny.into(),
    }
}

/// Discord's own numbers for the bits: Create Instant Invite is bit 0, View
/// Channel bit 10, Administrator bit 3, and so on up to bit 63.
#[test]
fn only_the_view_channel_bit_decides_and_every_bit_is_tried() {
    assert_eq!(VIEW_CHANNEL, 1 << 10);
    let everyone = |permissions: u64| role(&permissions.to_string());
    for bit in 0..64u32 {
        let single = 1u64 << bit;
        let sees = bit == 10;

        // In the role's own grant.
        assert_eq!(
            hidden_from_everyone(&channel(vec![]), Some(&everyone(single))),
            !sees,
            "grant, bit {bit}"
        );
        // Every bit but one is set: hidden only when that one is the view bit.
        assert_eq!(
            hidden_from_everyone(&channel(vec![]), Some(&everyone(!single))),
            bit == 10,
            "all but bit {bit}"
        );
        // In an overwrite that denies: only the view bit hides.
        let deny = channel(vec![overwrite(OverwriteTarget::Role, "100", "0", &single.to_string())]);
        assert_eq!(
            hidden_from_everyone(&deny, Some(&everyone(VIEW_CHANNEL))),
            sees,
            "deny, bit {bit}"
        );
        // In an overwrite that allows: only the view bit shows.
        let allow = channel(vec![overwrite(OverwriteTarget::Role, "100", &single.to_string(), "0")]);
        assert_eq!(
            hidden_from_everyone(&allow, Some(&everyone(0))),
            !sees,
            "allow, bit {bit}"
        );
    }
}

#[test]
fn an_allow_beats_a_deny_and_only_the_everyone_role_counts() {
    let denies = overwrite(OverwriteTarget::Role, "100", "0", "1024");
    let allows = overwrite(OverwriteTarget::Role, "100", "1024", "0");
    let both = overwrite(OverwriteTarget::Role, "100", "1024", "1024");
    let granted = role("1024");
    let not_granted = role("0");

    assert!(!hidden_from_everyone(&channel(vec![]), Some(&granted)));
    assert!(hidden_from_everyone(&channel(vec![]), Some(&not_granted)));
    assert!(hidden_from_everyone(&channel(vec![denies.clone()]), Some(&granted)));
    assert!(!hidden_from_everyone(&channel(vec![allows.clone()]), Some(&not_granted)));
    assert!(!hidden_from_everyone(&channel(vec![both]), Some(&granted)));
    assert!(!hidden_from_everyone(&channel(vec![both_swapped()]), Some(&granted)));

    // Another role's or a member's overwrite does not hide the channel from everyone.
    let other_role = overwrite(OverwriteTarget::Role, "200", "0", "1024");
    let member = overwrite(OverwriteTarget::Member, "100", "0", "1024");
    assert!(!hidden_from_everyone(&channel(vec![other_role, member]), Some(&granted)));

    // Without the role every member has, nothing can be said to be hidden.
    assert!(!hidden_from_everyone(&channel(vec![denies]), None));

    // The first overwrite for the role is the one that counts.
    let first = channel(vec![
        overwrite(OverwriteTarget::Role, "100", "0", "0"),
        overwrite(OverwriteTarget::Role, "100", "0", "1024"),
    ]);
    assert!(!hidden_from_everyone(&first, Some(&granted)));

    // A permission set that cannot be read grants nothing, and denies nothing.
    assert!(hidden_from_everyone(&channel(vec![]), Some(&role("garbage"))));
    assert!(hidden_from_everyone(&channel(vec![]), Some(&role("-1"))));
    assert!(!hidden_from_everyone(
        &channel(vec![overwrite(OverwriteTarget::Role, "100", "x", "y")]),
        Some(&granted)
    ));
}

fn both_swapped() -> PermissionOverwriteSnapshot {
    overwrite(OverwriteTarget::Role, "100", "1025", "1024")
}
