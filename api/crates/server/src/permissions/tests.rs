//! The rules against a database: the answers a handler gets for servers built
//! in the test, with roles, overrides at both levels and members.

use std::collections::HashSet;

use super::authorization::*;
use super::fixture::{MODERATOR, World};
use super::privileges::*;
use super::roles::{self, HeldRoleTable, Member, Role, ServerRoles};
use super::snapshot::{Snapshot, Targets};
use super::visibility::{self, ChannelAudience, ChannelKind, ServerVisibility};
use crate::values::Id;

fn holds(world: &World, user: Id, channel: Id, permission: i64) -> bool {
    allows(
        &world.db,
        user,
        &Scope::channel(world.server, channel),
        permission,
    )
    .expect("a check")
}

fn holds_in_section(world: &World, user: Id, section: Id, permission: i64) -> bool {
    allows(
        &world.db,
        user,
        &Scope::section(world.server, section),
        permission,
    )
    .expect("a check")
}

fn role_row(world: &World, id: Id) -> Role {
    ServerRoles::load(&world.db, world.server)
        .expect("roles")
        .by_id()[&id]
}

fn member_row(world: &World, user: Id) -> Member {
    Member::load(&world.db, world.server, user)
        .expect("a read")
        .expect("a member")
}

fn set(ids: &[Id]) -> HashSet<Id> {
    ids.iter().copied().collect()
}

/// A member with a role of their own, so no test shares an override with another.
fn member_with(world: &World, permissions: i64) -> (Id, Id) {
    let role = world.role("own", 10, permissions);
    let user = world.user(&format!("u{}", Id::random()));
    world.admit(user, role);
    (user, role)
}

// ---- What a member holds server-wide and at a place

#[test]
fn members_get_the_privileges_of_their_role_and_strangers_get_none() {
    let world = World::new();
    let stranger = world.user("stranger");

    assert_eq!(
        server_wide(&world.db, world.maja, world.server).unwrap(),
        ALL
    );
    // Every member holds the base role as well, and with it what it lets everybody do.
    assert_eq!(
        server_wide(&world.db, world.ada, world.server).unwrap(),
        MODERATOR | BASE_ROLE_DEFAULTS
    );
    assert_eq!(
        server_wide(&world.db, world.ilya, world.server).unwrap(),
        BASE_ROLE_DEFAULTS
    );
    assert_eq!(
        server_wide(&world.db, stranger, world.server).unwrap(),
        NONE
    );

    let scope = Scope::server(world.server);
    assert!(allows(&world.db, world.ada, &scope, MANAGE_BANS).unwrap());
    assert!(!allows(&world.db, world.ada, &scope, MANAGE_BANS | MANAGE_SERVER).unwrap());
    assert!(!allows(&world.db, stranger, &scope, MANAGE_BANS).unwrap());
}

#[test]
fn a_channel_without_overrides_follows_the_role() {
    let world = World::new();
    let (member, _) = member_with(&world, MANAGE_MESSAGES);

    assert!(holds(&world, member, world.general, MANAGE_MESSAGES));
    assert!(!holds(&world, member, world.general, MANAGE_BANS));
}

#[test]
fn a_channel_override_beats_a_section_override_which_beats_the_role() {
    let world = World::new();
    let (member, role) = member_with(&world, MANAGE_MESSAGES);
    let section = world.section();
    let denied = world.channel(Some(section));
    let reallowed = world.channel(Some(section));
    let outside = world.channel(None);
    world.override_of(true, section, role, NONE, MANAGE_MESSAGES);
    world.override_of(false, reallowed, role, MANAGE_MESSAGES, NONE);

    assert!(!holds(&world, member, denied, MANAGE_MESSAGES));
    assert!(holds(&world, member, reallowed, MANAGE_MESSAGES));
    assert!(holds(&world, member, outside, MANAGE_MESSAGES));
}

#[test]
fn a_section_level_applies_before_the_channel_level_whichever_way_they_point() {
    let world = World::new();
    let (member, role) = member_with(&world, NONE);
    let section = world.section();
    let inherited = world.channel(Some(section));
    let revoked = world.channel(Some(section));
    world.override_of(true, section, role, MANAGE_CHANNELS, NONE);
    world.override_of(false, revoked, role, NONE, MANAGE_CHANNELS);

    assert!(holds(&world, member, inherited, MANAGE_CHANNELS));
    assert!(!holds(&world, member, revoked, MANAGE_CHANNELS));
}

#[test]
fn a_bit_both_allowed_and_denied_in_one_override_stays_denied() {
    let world = World::new();
    let (member, role) = member_with(&world, NONE);
    let channel = world.channel(None);
    world.override_of(
        false,
        channel,
        role,
        MANAGE_BANS | VIEW_AUDIT_LOG,
        MANAGE_BANS,
    );

    assert!(!holds(&world, member, channel, MANAGE_BANS));
    assert!(holds(&world, member, channel, VIEW_AUDIT_LOG));
}

#[test]
fn the_overrides_of_other_roles_do_not_apply() {
    let world = World::new();
    let (member, _) = member_with(&world, NONE);
    let (other, other_role) = member_with(&world, NONE);
    let channel = world.channel(None);
    world.override_of(false, channel, other_role, MANAGE_BANS, NONE);

    assert!(holds(&world, other, channel, MANAGE_BANS));
    assert!(!holds(&world, member, channel, MANAGE_BANS));
}

#[test]
fn one_roles_allow_beats_another_roles_deny_at_the_same_level() {
    let world = World::new();
    let allowing = world.role("allowing", 3, NONE);
    let denying = world.role("denying", 4, NONE);
    let user = world.user("both");
    world.member_of(user, &[allowing, denying]);
    let channel = world.channel(None);
    world.override_of(false, channel, allowing, MANAGE_MESSAGES, NONE);
    world.override_of(false, channel, denying, NONE, MANAGE_MESSAGES);
    world.set_permissions(denying, MANAGE_MESSAGES);

    assert!(holds(&world, user, channel, MANAGE_MESSAGES));
    // Without the allowing role the deny stands.
    let only_denied = world.user("only denied");
    world.member_of(only_denied, &[denying]);
    assert!(!holds(&world, only_denied, channel, MANAGE_MESSAGES));
}

#[test]
fn the_members_own_override_has_the_last_word() {
    let world = World::new();
    let (member, role) = member_with(&world, MANAGE_BANS);
    let channel = world.channel(None);
    world.override_of(false, channel, role, NONE, MANAGE_BANS);
    assert!(!holds(&world, member, channel, MANAGE_BANS));

    world.member_override(false, channel, member, MANAGE_BANS, NONE);
    assert!(holds(&world, member, channel, MANAGE_BANS));
}

#[test]
fn a_channel_of_another_server_never_grants_anything() {
    let world = World::new();
    let (member, _) = member_with(&world, ALL);
    let other = World::new();

    assert!(!holds(&world, member, other.general, MANAGE_MESSAGES));
    assert!(!holds(&world, member, Id::random(), MANAGE_MESSAGES));
}

#[test]
fn a_non_member_holds_nothing_on_any_channel() {
    let world = World::new();
    let stranger = world.user("stranger");

    assert!(!holds(&world, stranger, world.general, MANAGE_MESSAGES));
    assert_eq!(
        effective(
            &world.db,
            stranger,
            &Scope::channel(world.server, world.general)
        )
        .unwrap(),
        None
    );
}

#[test]
fn empty_requirements_still_need_a_real_scope_and_a_membership() {
    let world = World::new();
    let nil = Id::from_bytes([0; 16]);

    assert!(holds(&world, world.ilya, world.general, NONE));
    assert!(!allows(&world.db, world.ilya, &Scope::server(nil), NONE).unwrap());
    assert!(
        !allows(
            &world.db,
            world.ilya,
            &Scope::channel(world.server, nil),
            NONE
        )
        .unwrap()
    );
    assert!(!holds(&world, world.ilya, Id::random(), NONE));
    assert!(!allows(&world.db, Id::random(), &Scope::server(world.server), NONE).unwrap());
    assert!(!allows(&world.db, world.maja, &Scope::server(world.server), 1 << 20).unwrap());
}

#[test]
fn disabling_an_account_revokes_its_permission_without_changing_its_role() {
    let world = World::new();
    let (member, _) = member_with(&world, ALL);
    assert!(holds(&world, member, world.general, MANAGE_MESSAGES));

    world.disable(member);

    assert!(!holds(&world, member, world.general, MANAGE_MESSAGES));
    // Rank and what the role holds are still there for the lists that need them.
    assert!(
        roles::permissions_of_member(&world.db, world.server, member)
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_section_override_decides_what_a_role_may_do_to_the_section() {
    let world = World::new();
    let (member, role) = member_with(&world, MANAGE_CHANNELS);
    let plain = world.section();
    let denied = world.section();
    let granted = world.section();
    world.override_of(true, denied, role, NONE, MANAGE_CHANNELS);
    world.override_of(
        true,
        granted,
        role,
        MANAGE_BANS | MANAGE_ROLES,
        MANAGE_ROLES,
    );

    assert!(holds_in_section(&world, member, plain, MANAGE_CHANNELS));
    assert!(!holds_in_section(&world, member, denied, MANAGE_CHANNELS));
    assert!(holds_in_section(&world, member, granted, MANAGE_BANS));
    assert!(!holds_in_section(&world, member, granted, MANAGE_ROLES));
}

#[test]
fn a_section_check_refuses_other_servers_and_strangers() {
    let world = World::new();
    let (member, _) = member_with(&world, ALL);
    let stranger = world.user("stranger");
    let section = world.section();
    let other = World::new();
    let foreign = other.section();

    assert!(holds_in_section(&world, member, section, MANAGE_CHANNELS));
    assert!(!holds_in_section(&world, member, foreign, MANAGE_CHANNELS));
    assert!(!holds_in_section(
        &world,
        stranger,
        section,
        MANAGE_CHANNELS
    ));
}

#[test]
fn whoever_lacks_view_at_a_channel_holds_nothing_there() {
    let world = World::new();
    let (member, role) = member_with(&world, MANAGE_MESSAGES | SEND_MESSAGES | VIEW_CHANNEL);
    let channel = world.channel(None);
    assert!(holds(&world, member, channel, SEND_MESSAGES));

    world.override_of(false, channel, role, NONE, VIEW_CHANNEL);

    let held = held_at(&world.db, member, &Scope::channel(world.server, channel)).unwrap();
    assert_eq!(held.0, NONE);
    assert!(!held.views());
    // At the server nothing was taken away: the rule is the channel's.
    assert!(
        allows(
            &world.db,
            member,
            &Scope::server(world.server),
            MANAGE_MESSAGES
        )
        .unwrap()
    );
    assert_eq!(held.require(SEND_MESSAGES), Err(Refusal::Hidden));
}

#[test]
fn a_handler_is_told_the_channel_is_not_there_before_it_is_told_what_is_missing() {
    let world = World::new();
    let (member, role) = member_with(&world, VIEW_CHANNEL | SEND_MESSAGES);
    let seen = world.channel(None);
    let hidden = world.channel(None);
    world.override_of(false, seen, role, NONE, SEND_MESSAGES);
    world.override_of(false, hidden, role, NONE, VIEW_CHANNEL | SEND_MESSAGES);

    let at = |channel| held_at(&world.db, member, &Scope::channel(world.server, channel)).unwrap();

    assert_eq!(
        at(seen).require(SEND_MESSAGES),
        Err(Refusal::Denied {
            permission: SEND_MESSAGES
        })
    );
    assert_eq!(at(hidden).require(SEND_MESSAGES), Err(Refusal::Hidden));
    assert!(at(seen).require(VIEW_CHANNEL).is_ok());
}

#[test]
fn the_owner_skips_everything() {
    let world = World::new();
    let channel = world.channel(None);
    // Denied view and everything else in every way an override can say it.
    world.override_of(false, channel, world.owner_role, NONE, ALL);
    world.override_of(false, channel, world.base_role, NONE, ALL);
    world.member_override(false, channel, world.maja, NONE, ALL);

    assert_eq!(
        effective(
            &world.db,
            world.maja,
            &Scope::channel(world.server, channel)
        )
        .unwrap(),
        Some(ALL)
    );
}

#[test]
fn an_administrator_skips_overrides_but_not_rank() {
    let world = World::new();
    let admin_role = world.role("admin", 1, ADMINISTRATOR);
    let admin = world.user("admin");
    world.admit(admin, admin_role);
    let peer = world.user("peer");
    world.admit(peer, admin_role);
    let channel = world.channel(None);
    world.override_of(false, channel, world.base_role, NONE, ALL);
    world.override_of(false, channel, admin_role, NONE, ALL);
    world.member_override(false, channel, admin, NONE, ALL);

    assert_eq!(
        effective(&world.db, admin, &Scope::channel(world.server, channel)).unwrap(),
        Some(ALL)
    );
    // Rank is not a permission: a peer, the owner and anybody above are out of reach,
    // and so is a role at the administrator's own position.
    assert_eq!(
        kick_member(&world.db, admin, world.server, peer).unwrap(),
        MemberAuthorization::Forbidden
    );
    assert_eq!(
        kick_member(&world.db, admin, world.server, world.maja).unwrap(),
        MemberAuthorization::Forbidden
    );
    assert_eq!(
        kick_member(&world.db, admin, world.server, world.ada).unwrap(),
        MemberAuthorization::Forbidden
    );
    assert!(
        kick_member(&world.db, admin, world.server, world.ilya)
            .unwrap()
            .is_allowed()
    );
    let same = world.role("same", 1, NONE);
    assert!(!can_edit_role(&world.db, admin, &role_row(&world, same), NONE).unwrap());
    let below = world.role("below", 5, NONE);
    assert!(can_edit_role(&world.db, admin, &role_row(&world, below), ALL).unwrap());
}

#[test]
fn equal_positions_do_not_outrank_each_other() {
    let world = World::new();
    let first = world.role("first", 5, NONE);
    let second = world.role("second", 5, NONE);

    let (a, b) = (role_row(&world, first), role_row(&world, second));

    assert!(!a.outranks(&b) && !b.outranks(&a));
    // So two people on them cannot moderate each other.
    let (x, y) = (world.user("x"), world.user("y"));
    world.admit(x, first);
    world.admit(y, second);
    world.set_permissions(first, MODERATE_MEMBERS);
    assert_eq!(
        kick_member(&world.db, x, world.server, y).unwrap(),
        MemberAuthorization::Forbidden
    );
}

#[test]
fn the_base_role_ranks_below_every_other_role() {
    let world = World::new();

    let first = super::ownership::place_new_role(&world.db, world.server).unwrap();
    world.insert_role(Id::random(), "First", first, NONE, false, false);
    let second = super::ownership::place_new_role(&world.db, world.server).unwrap();
    world.insert_role(Id::random(), "Second", second, NONE, false, false);

    let listed = roles::ranked(ServerRoles::load(&world.db, world.server).unwrap().roles);
    // The owner role first, the base role last, the roles made between in order.
    assert!(listed[0].is_owner);
    let last = listed.last().unwrap();
    assert!(last.is_system && !last.is_owner);
    assert_eq!(last.id, world.base_role);
    let positions: Vec<i64> = listed.iter().map(|r| r.position).collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] <= pair[1]),
        "{positions:?}"
    );
    assert!(listed[listed.len() - 2].position < last.position);
    assert!(listed[listed.len() - 3].position < listed[listed.len() - 2].position);
    for role in &listed[..listed.len() - 1] {
        assert!(!last.outranks(role));
        if !role.is_owner {
            assert!(role.outranks(last));
        }
    }
    // A person given a new role can moderate somebody who was given none.
    let guard = world.role("guard", first, MODERATE_MEMBERS);
    let user = world.user("guard");
    world.admit(user, guard);
    assert!(
        kick_member(&world.db, user, world.server, world.nils)
            .unwrap()
            .is_allowed()
    );
}

#[test]
fn a_base_role_is_the_last_system_role_that_is_not_the_owners_and_ties_go_by_id() {
    let world = World::new();
    // A second system role above it, and one tied with it: the highest position value wins, then the lowest id.
    let veteran = Id::random();
    world.insert_role(veteran, "Veteran", 1, NONE, true, false);
    assert_eq!(
        roles::find_member_role(&world.db, world.server)
            .unwrap()
            .unwrap()
            .id,
        world.base_role
    );

    let tied = Id::random();
    world.insert_role(tied, "Tied", 2, NONE, true, false);
    let expected = if tied < world.base_role {
        tied
    } else {
        world.base_role
    };
    assert_eq!(
        roles::find_member_role(&world.db, world.server)
            .unwrap()
            .unwrap()
            .id,
        expected
    );
    assert_eq!(
        ServerRoles::load(&world.db, world.server)
            .unwrap()
            .base
            .unwrap()
            .id,
        expected
    );
}

#[test]
fn every_held_role_counts_for_what_a_member_may_do_and_only_the_top_role_ranks() {
    let world = World::new();
    let low = world.role("low", 8, MANAGE_BANS);
    let high = world.role("high", 4, MANAGE_MESSAGES);
    let user = world.user("many");
    world.member_of(user, &[low, high]);

    let held = server_wide(&world.db, user, world.server).unwrap();
    assert_eq!(held, BASE_ROLE_DEFAULTS | MANAGE_BANS | MANAGE_MESSAGES);
    assert_eq!(member_row(&world, user).role.id, high);
    let table = HeldRoleTable::load(&world.db, world.server).unwrap();
    assert_eq!(
        table.of(user).iter().map(|r| r.id).collect::<Vec<_>>(),
        [high, low]
    );
    assert_eq!(table.permissions_of(user), held);
    assert_eq!(table.count_holders(low), 1);
    assert_eq!(table.count_holders(world.base_role), 6);
    assert_eq!(table.holders(high), [user]);
}

// ---- Who sees which channel

fn viewers_of(world: &World, channel: Id) -> Option<HashSet<Id>> {
    visibility::viewers(&world.db, world.server, channel).unwrap()
}

#[test]
fn a_server_where_nobody_is_denied_view_is_open_everywhere() {
    let world = World::new();

    assert!(visibility::server_open(&world.db, world.server).unwrap());
    for channel in [world.general, world.projects, world.voice] {
        assert!(
            visibility::place(&world.db, channel, None, None, None)
                .unwrap()
                .unwrap()
                .open
        );
    }
    assert_eq!(viewers_of(&world, world.projects), None);
    assert_eq!(
        visibility::of_member(&world.db, world.jo, world.server).unwrap(),
        None
    );
    assert!(
        ServerVisibility::load(&world.db, world.server)
            .unwrap()
            .is_open()
    );
    assert_eq!(
        visibility::audience_of(&world.db, world.server, world.projects).unwrap(),
        ChannelAudience::Server(world.server)
    );
}

#[test]
fn a_deny_at_one_channel_closes_that_channel_and_the_server_but_no_other_channel() {
    let world = World::new();
    world.deny_view_to_base(world.projects, false);
    world.allow_view_to(world.projects, false, world.moderator_role);

    let open = |channel| {
        visibility::place(&world.db, channel, None, None, None)
            .unwrap()
            .unwrap()
            .open
    };
    assert!(!open(world.projects));
    assert!(open(world.general));
    assert!(!visibility::server_open(&world.db, world.server).unwrap());

    // The owner, and the moderator through the role that is allowed.
    assert_eq!(
        viewers_of(&world, world.projects),
        Some(set(&[world.maja, world.ada]))
    );
    assert!(
        visibility::find(
            &world.db,
            world.jo,
            world.server,
            world.projects,
            Some(ChannelKind::Text)
        )
        .unwrap()
        .is_none()
    );
    assert!(
        visibility::find(
            &world.db,
            world.ada,
            world.server,
            world.projects,
            Some(ChannelKind::Text)
        )
        .unwrap()
        .is_some()
    );
    assert!(
        visibility::find(
            &world.db,
            world.jo,
            world.server,
            world.general,
            Some(ChannelKind::Text)
        )
        .unwrap()
        .is_some()
    );
    // Another kind of channel is not found by a search for a text one.
    assert!(
        visibility::find(
            &world.db,
            world.jo,
            world.server,
            world.voice,
            Some(ChannelKind::Text)
        )
        .unwrap()
        .is_none()
    );
    assert!(
        visibility::find(
            &world.db,
            world.jo,
            world.server,
            world.voice,
            Some(ChannelKind::Voice)
        )
        .unwrap()
        .is_some()
    );

    let view = visibility::of_member(&world.db, world.jo, world.server)
        .unwrap()
        .unwrap();
    assert!(!view.channels.contains(&world.projects));
    assert!(view.channels.contains(&world.general));
    assert_eq!(
        visibility::audience_of(&world.db, world.server, world.projects).unwrap(),
        ChannelAudience::Viewers {
            server: world.server,
            users: {
                let mut users = vec![world.maja, world.ada];
                users.sort();
                users
            }
        }
    );
    assert_eq!(
        visibility::audience_of(&world.db, world.server, world.general).unwrap(),
        ChannelAudience::Server(world.server)
    );
}

#[test]
fn an_override_that_outlived_its_channel_denies_nothing() {
    let world = World::new();
    world.deny_view_to_base(Id::random(), false);

    assert!(visibility::server_open(&world.db, world.server).unwrap());
}

#[test]
fn a_base_role_without_view_closes_everything() {
    let world = World::new();
    world.set_permissions(world.base_role, SEND_MESSAGES);

    assert!(!visibility::server_open(&world.db, world.server).unwrap());
    for channel in [world.general, world.projects, world.voice] {
        assert!(
            !visibility::place(&world.db, channel, None, None, None)
                .unwrap()
                .unwrap()
                .open
        );
    }
    let everyone = ServerVisibility::load(&world.db, world.server).unwrap();
    assert!(everyone.view_of(world.ilya).unwrap().channels.is_empty());
    assert!(everyone.view_of(world.nils).unwrap().channels.is_empty());
    assert_eq!(everyone.view_of(world.maja).unwrap().channels.len(), 3);
}

#[test]
fn an_empty_section_is_listed_where_view_is_held_and_a_private_one_only_for_its_managers() {
    let world = World::new();
    let open = world.section();
    let staff = world.section();
    world.deny_view_to_base(staff, true);

    let everyone = ServerVisibility::load(&world.db, world.server).unwrap();

    assert_eq!(everyone.view_of(world.jo).unwrap().sections, set(&[open]));
    assert_eq!(
        everyone.view_of(world.maja).unwrap().sections,
        set(&[open, staff])
    );
    assert!(visibility::section_listed(&world.db, world.jo, world.server, open).unwrap());
    assert!(!visibility::section_listed(&world.db, world.jo, world.server, staff).unwrap());
    assert!(visibility::section_listed(&world.db, world.maja, world.server, staff).unwrap());
    assert!(!visibility::section_listed(&world.db, world.jo, world.server, Id::random()).unwrap());
}

#[test]
fn a_section_with_a_channel_somebody_sees_is_listed_for_them() {
    let world = World::new();
    let section = world.section();
    let hidden = world.channel(Some(section));
    let shown = world.channel(Some(section));
    world.deny_view_to_base(hidden, false);

    let view = visibility::of_member(&world.db, world.jo, world.server)
        .unwrap()
        .unwrap();

    assert!(view.channels.contains(&shown) && !view.channels.contains(&hidden));
    assert!(view.sections.contains(&section));
}

#[test]
fn a_view_denied_at_a_section_reaches_its_channels_and_the_channel_may_allow_it_again() {
    let world = World::new();
    let section = world.section();
    let inside = world.channel(Some(section));
    let allowed = world.channel(Some(section));
    world.deny_view_to_base(section, true);
    world.allow_view_to(allowed, false, world.base_role);

    assert!(!visibility::views(&world.db, world.jo, inside).unwrap());
    assert!(visibility::views(&world.db, world.jo, allowed).unwrap());
    assert!(visibility::views(&world.db, world.maja, inside).unwrap());
}

#[test]
fn who_view_checks_only_those_asked_about_and_keeps_their_order() {
    let world = World::new();
    world.deny_view_to_base(world.projects, false);
    world.allow_view_to(world.projects, false, world.moderator_role);
    let stranger = world.user("stranger");

    let seen = visibility::who_view(
        &world.db,
        world.server,
        world.projects,
        &[world.jo, world.ada, stranger, world.maja],
    )
    .unwrap();

    assert_eq!(seen, [world.ada, world.maja]);
    // Open: everybody asked about is passed through, members or not.
    assert_eq!(
        visibility::who_view(
            &world.db,
            world.server,
            world.general,
            &[world.jo, stranger]
        )
        .unwrap(),
        [world.jo, stranger]
    );
    assert!(
        visibility::who_view(&world.db, world.server, Id::random(), &[world.jo])
            .unwrap()
            .is_empty()
    );
    assert!(
        visibility::who_view(&world.db, world.server, world.general, &[])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_viewers_of_a_channel_that_is_not_there_are_nobody_and_disabled_accounts_do_not_view() {
    let world = World::new();
    assert_eq!(
        visibility::viewers(&world.db, world.server, Id::random()).unwrap(),
        Some(HashSet::new())
    );

    world.deny_view_to_base(world.projects, false);
    world.allow_view_to(world.projects, false, world.moderator_role);
    world.disable(world.ada);

    assert_eq!(viewers_of(&world, world.projects), Some(set(&[world.maja])));
    assert!(!visibility::views(&world.db, world.ada, world.projects).unwrap());
}

#[test]
fn a_newcomer_holds_the_base_role_alone() {
    let world = World::new();
    let private = world.channel(None);
    world.deny_view_to_base(private, false);
    // Even a role that is allowed does not count, nobody holds it yet.
    world.allow_view_to(private, false, world.moderator_role);

    assert!(visibility::newcomer_views(&world.db, world.general).unwrap());
    assert!(!visibility::newcomer_views(&world.db, private).unwrap());
    assert!(!visibility::newcomer_views(&world.db, Id::random()).unwrap());
    // The base role's own allow does show a channel a section hid.
    let section = world.section();
    let inside = world.channel(Some(section));
    world.deny_view_to_base(section, true);
    assert!(!visibility::newcomer_views(&world.db, inside).unwrap());
    world.allow_view_to(inside, false, world.base_role);
    assert!(visibility::newcomer_views(&world.db, inside).unwrap());
}

#[test]
fn the_members_whose_view_a_change_altered_are_found_by_reading_before_and_after() {
    let world = World::new();
    let before = ServerVisibility::load(&world.db, world.server).unwrap();
    assert!(
        ServerVisibility::changed(&world.db, &before, &before)
            .unwrap()
            .is_empty()
    );

    world.deny_view_to_base(world.projects, false);
    world.allow_view_to(world.projects, false, world.moderator_role);
    let after = ServerVisibility::load(&world.db, world.server).unwrap();

    let mut changed = ServerVisibility::changed(&world.db, &before, &after).unwrap();
    changed.sort();
    let mut expected = vec![world.ilya, world.nils, world.jo];
    expected.sort();
    assert_eq!(changed, expected);
    // And back again: the same people, the other way round.
    let mut back = ServerVisibility::changed(&world.db, &after, &before).unwrap();
    back.sort();
    assert_eq!(back, expected);
    let again = ServerVisibility::load(&world.db, world.server).unwrap();
    assert!(
        ServerVisibility::changed(&world.db, &after, &again)
            .unwrap()
            .is_empty()
    );
}

// ---- Delegation: the checks the routes that edit roles, overrides and members make

/// Ada as an actor with a role of her own that ranks at 10, a lower role at
/// 20 and a destination at 21, the base role at 30, and a member on the lower
/// role, as the first server's own tests arranged it.
struct Scenario {
    world: World,
    actor: Id,
    target: Id,
    actor_role: Id,
    lower: Id,
    destination: Id,
    section: Id,
    channel: Id,
}

impl Scenario {
    fn new() -> Scenario {
        let world = World::new();
        let actor_role = world.role("Actor", 10, MANAGE_ROLES | MANAGE_MEMBERS | MANAGE_MESSAGES);
        let lower = world.role("Lower", 20, NONE);
        let destination = world.role("Destination", 21, MANAGE_MESSAGES);
        world.set_position(world.base_role, 30);
        let section = world.section();
        let channel = world.channel(Some(section));
        let (actor, target) = (world.user("actor"), world.user("target"));
        world.admit(actor, actor_role);
        world.admit(target, lower);
        Scenario {
            world,
            actor,
            target,
            actor_role,
            lower,
            destination,
            section,
            channel,
        }
    }

    fn role(&self, id: Id) -> Role {
        role_row(&self.world, id)
    }

    fn scope(&self, section: bool) -> Scope {
        if section {
            Scope::section(self.world.server, self.section)
        } else {
            Scope::channel(self.world.server, self.channel)
        }
    }

    fn override_of(&self, role: Id, section: bool, allow: i64, deny: i64) {
        self.world.override_of(
            section,
            if section { self.section } else { self.channel },
            role,
            allow,
            deny,
        );
    }

    fn change_override(&self, section: bool, role: Id, allow: i64, deny: i64) -> bool {
        can_change_override(
            &self.world.db,
            self.actor,
            &self.scope(section),
            role,
            allow,
            deny,
        )
        .unwrap()
    }

    fn edit(&self, role: Id, permissions: i64) -> bool {
        can_edit_role(&self.world.db, self.actor, &self.role(role), permissions).unwrap()
    }

    fn delete(&self, role: Id) -> bool {
        let fallback = roles::find_member_role(&self.world.db, self.world.server)
            .unwrap()
            .unwrap();
        can_delete_role(&self.world.db, self.actor, &self.role(role), &fallback).unwrap()
    }

    fn assign(&self, target: Id, roles: &[Id]) -> bool {
        let target = member_row(&self.world, target);
        let changed: Vec<Role> = roles.iter().map(|id| self.role(*id)).collect();
        can_change_roles(&self.world.db, self.actor, &target, &changed).unwrap()
    }
}

#[test]
fn editing_and_deleting_roles_requires_a_strictly_lower_rank() {
    for (relationship, delete, allowed) in [
        ("own", false, false),
        ("equal", false, false),
        ("higher", false, false),
        ("lower", false, true),
        ("own", true, false),
        ("equal", true, false),
        ("higher", true, false),
        ("lower", true, true),
    ] {
        let s = Scenario::new();
        let role = if relationship == "own" {
            s.actor_role
        } else {
            s.lower
        };
        match relationship {
            "equal" => s.world.set_position(role, 10),
            "higher" => s.world.set_position(role, 5),
            _ => {}
        }
        let decided = if delete {
            s.delete(role)
        } else {
            s.edit(role, NONE)
        };
        assert_eq!(decided, allowed, "{relationship} delete={delete}");
    }
}

#[test]
fn existing_role_powers_cannot_be_removed_without_holding_them() {
    for operation in [
        "replace-base",
        "rename-base",
        "delete-base",
        "rename-hidden",
        "delete-hidden",
    ] {
        let s = Scenario::new();
        if operation.ends_with("hidden") {
            s.override_of(s.lower, false, MANAGE_BANS, NONE);
        } else {
            s.world.set_permissions(s.lower, MANAGE_BANS);
        }
        let allowed = if operation.starts_with("delete") {
            s.delete(s.lower)
        } else {
            s.edit(
                s.lower,
                if operation == "replace-base" {
                    NONE
                } else {
                    s.role(s.lower).permissions
                },
            )
        };
        assert!(!allowed, "{operation}");
    }
}

#[test]
fn creating_and_updating_roles_can_delegate_only_the_actors_powers() {
    let s = Scenario::new();
    let db = &s.world.db;

    assert!(!can_create_role(db, s.actor, s.world.server, MANAGE_BANS).unwrap());
    assert!(can_create_role(db, s.actor, s.world.server, MANAGE_MESSAGES).unwrap());
    assert!(!can_create_role(db, s.actor, s.world.server, 1 << 20).unwrap());
    assert!(!s.edit(s.lower, MANAGE_BANS));
    assert!(s.edit(s.lower, MANAGE_MESSAGES));
    // Somebody without the permission to manage roles creates nothing, and a stranger neither.
    assert!(!can_create_role(db, s.target, s.world.server, NONE).unwrap());
    assert!(!can_create_role(db, Id::random(), s.world.server, NONE).unwrap());
}

#[test]
fn removing_a_deny_through_an_empty_put_or_a_delete_still_requires_that_power() {
    for section in [false, true] {
        let s = Scenario::new();
        s.override_of(s.lower, section, NONE, MANAGE_BANS);

        assert!(
            !s.change_override(section, s.lower, NONE, NONE),
            "section={section}"
        );
    }
}

#[test]
fn an_override_grant_and_deny_both_respect_the_server_role_ceiling() {
    for section in [false, true] {
        let s = Scenario::new();

        assert!(!s.change_override(section, s.lower, MANAGE_BANS, NONE));
        assert!(!s.change_override(section, s.lower, NONE, MANAGE_BANS));
        assert!(s.change_override(section, s.lower, MANAGE_MESSAGES, NONE));
        // Clearing one that was set is the same decision with the old masks.
        s.override_of(s.lower, section, MANAGE_MESSAGES, NONE);
        assert!(s.change_override(section, s.lower, NONE, NONE));
    }
}

#[test]
fn an_override_that_names_administrator_or_both_ways_or_an_unknown_bit_is_never_allowed() {
    let s = Scenario::new();
    s.world.set_permissions(s.actor_role, ALL);

    assert!(!s.change_override(false, s.lower, ADMINISTRATOR, NONE));
    assert!(!s.change_override(false, s.lower, NONE, ADMINISTRATOR));
    assert!(!s.change_override(false, s.lower, MANAGE_MESSAGES, MANAGE_MESSAGES));
    assert!(!s.change_override(false, s.lower, 1 << 20, NONE));
    let server_scope = Scope::server(s.world.server);
    assert!(
        !can_change_override(
            &s.world.db,
            s.actor,
            &server_scope,
            s.lower,
            MANAGE_MESSAGES,
            NONE
        )
        .unwrap()
    );
}

#[test]
fn override_changes_also_respect_the_role_hierarchy() {
    for relationship in ["own", "equal", "higher", "lower"] {
        let s = Scenario::new();
        let role = if relationship == "own" {
            s.actor_role
        } else {
            s.lower
        };
        match relationship {
            "equal" => s.world.set_position(role, 10),
            "higher" => s.world.set_position(role, 5),
            _ => {}
        }

        assert_eq!(
            s.change_override(false, role, MANAGE_MESSAGES, NONE),
            relationship == "lower",
            "{relationship}"
        );
    }
}

#[test]
fn a_scoped_manage_roles_deny_prevents_editing_overrides() {
    for (deny_at_section, edit_section) in [(false, false), (true, false), (true, true)] {
        let s = Scenario::new();
        s.override_of(s.actor_role, deny_at_section, NONE, MANAGE_ROLES);

        assert!(
            !s.change_override(edit_section, s.lower, MANAGE_MESSAGES, NONE),
            "{deny_at_section} {edit_section}"
        );
    }
}

#[test]
fn a_local_grant_cannot_expand_delegation_and_a_local_deny_restricts_it() {
    for beyond_base in [false, true] {
        let s = Scenario::new();
        s.override_of(
            s.actor_role,
            false,
            if beyond_base { MANAGE_BANS } else { NONE },
            if beyond_base { NONE } else { MANAGE_MESSAGES },
        );

        let wanted = if beyond_base {
            MANAGE_BANS
        } else {
            MANAGE_MESSAGES
        };
        assert!(
            !s.change_override(false, s.lower, wanted, NONE),
            "beyond_base={beyond_base}"
        );
    }
}

#[test]
fn role_assignments_require_a_lower_target_and_a_lower_destination() {
    for relationship in [
        "self",
        "target-equal",
        "target-higher",
        "role-equal",
        "role-higher",
        "lower",
    ] {
        let s = Scenario::new();
        let target = if relationship == "self" {
            s.actor
        } else {
            s.target
        };
        match relationship {
            "target-equal" => s.world.set_position(s.lower, 10),
            "target-higher" => s.world.set_position(s.lower, 5),
            "role-equal" => s.world.set_position(s.destination, 10),
            "role-higher" => s.world.set_position(s.destination, 5),
            _ => {}
        }

        assert_eq!(
            s.assign(target, &[s.destination]),
            relationship == "lower",
            "{relationship}"
        );
    }
}

#[test]
fn role_assignment_checks_hidden_grants_and_base_powers_on_both_old_and_new_roles() {
    for old in [false, true] {
        let hidden = Scenario::new();
        hidden.override_of(
            if old {
                hidden.lower
            } else {
                hidden.destination
            },
            true,
            MANAGE_BANS,
            NONE,
        );
        assert!(
            !hidden.assign(hidden.target, &[hidden.lower, hidden.destination]),
            "hidden grant, old={old}"
        );

        let base = Scenario::new();
        base.world
            .set_permissions(if old { base.lower } else { base.destination }, MANAGE_BANS);
        assert!(
            !base.assign(base.target, &[base.lower, base.destination]),
            "base power, old={old}"
        );
    }
    let plain = Scenario::new();
    assert!(plain.assign(plain.target, &[plain.lower, plain.destination]));
}

#[test]
fn deleting_a_role_cannot_grant_hidden_powers_through_its_fallback() {
    let s = Scenario::new();
    s.override_of(s.world.base_role, false, MANAGE_BANS, NONE);

    assert!(!s.delete(s.lower));
    assert_eq!(member_row(&s.world, s.target).role.id, s.lower);
}

#[test]
fn a_system_role_is_never_deleted_and_a_fallback_from_another_server_is_refused() {
    let s = Scenario::new();
    let base = s.role(s.world.base_role);
    let other = World::new();
    let foreign = role_row(&other, other.base_role);

    assert!(!can_delete_role(&s.world.db, s.actor, &base, &base).unwrap());
    assert!(!can_delete_role(&s.world.db, s.actor, &s.role(s.lower), &foreign).unwrap());
}

#[test]
fn the_owner_can_clear_their_own_legacy_denies_but_other_actors_cannot() {
    for section in [false, true] {
        let s = Scenario::new();
        s.override_of(
            s.world.owner_role,
            section,
            NONE,
            MANAGE_ROLES | MANAGE_MESSAGES,
        );

        assert!(
            !s.change_override(section, s.world.owner_role, NONE, NONE),
            "actor, section={section}"
        );
        let owner = can_change_override(
            &s.world.db,
            s.world.maja,
            &s.scope(section),
            s.world.owner_role,
            NONE,
            NONE,
        )
        .unwrap();
        assert!(owner, "owner, section={section}");
    }
}

#[test]
fn overrides_whose_resource_was_deleted_do_not_block_editing_the_role() {
    for section in [false, true] {
        let s = Scenario::new();
        s.override_of(s.lower, section, MANAGE_BANS, NONE);
        assert!(!s.edit(s.lower, NONE));

        if section {
            s.world
                .db
                .execute("DELETE FROM \"Sections\" WHERE \"Id\" = ?1", [s.section])
                .unwrap();
        } else {
            s.world
                .db
                .execute("DELETE FROM \"Channels\" WHERE \"Id\" = ?1", [s.channel])
                .unwrap();
        }

        assert!(s.edit(s.lower, NONE), "section={section}");
    }
}

#[test]
fn reordering_roles_moves_only_those_the_actor_outranks_and_nothing_climbs() {
    let s = Scenario::new();
    let ranked: Vec<Role> = roles::ranked(
        ServerRoles::load(&s.world.db, s.world.server)
            .unwrap()
            .roles,
    )
    .into_iter()
    .filter(|role| !role.is_owner && role.id != s.world.base_role)
    .collect();
    let ids: Vec<Id> = ranked.iter().map(|role| role.id).collect();
    // Moderator (1) and Actor (10) come before Lower (20) and Destination (21).
    assert_eq!(
        ids,
        [s.world.moderator_role, s.actor_role, s.lower, s.destination]
    );

    let reorder = |wanted: &[Id]| {
        can_reorder_roles(&s.world.db, s.actor, s.world.server, &ranked, wanted).unwrap()
    };
    assert!(reorder(&ids));
    assert!(reorder(&[
        s.world.moderator_role,
        s.actor_role,
        s.destination,
        s.lower
    ]));
    // Climbing above the actor's own top role, swapping with it, or moving a higher one is refused.
    assert!(!reorder(&[
        s.world.moderator_role,
        s.lower,
        s.actor_role,
        s.destination
    ]));
    assert!(!reorder(&[
        s.actor_role,
        s.world.moderator_role,
        s.lower,
        s.destination
    ]));
    assert!(!reorder(&[
        s.lower,
        s.destination,
        s.world.moderator_role,
        s.actor_role
    ]));
    assert!(!reorder(&ids[..3]));
    s.world.set_permissions(s.actor_role, NONE);
    assert!(!reorder(&ids));
}

#[test]
fn moderating_needs_the_permission_a_lower_target_and_not_oneself() {
    let world = World::new();

    assert!(
        kick_member(&world.db, world.ada, world.server, world.ilya)
            .unwrap()
            .is_allowed()
    );
    assert!(
        ban_member(&world.db, world.ada, world.server, world.ilya)
            .unwrap()
            .is_allowed()
    );
    assert_eq!(
        kick_member(&world.db, world.ada, world.server, world.ada).unwrap(),
        MemberAuthorization::Forbidden
    );
    assert_eq!(
        kick_member(&world.db, world.ada, world.server, world.maja).unwrap(),
        MemberAuthorization::Forbidden
    );
    assert_eq!(
        kick_member(&world.db, world.ilya, world.server, world.jo).unwrap(),
        MemberAuthorization::Forbidden
    );
    // A target who is not an enabled member of the server is not found, once the actor may.
    assert_eq!(
        kick_member(&world.db, world.ada, world.server, Id::random()).unwrap(),
        MemberAuthorization::NotFound
    );
    world.disable(world.nils);
    assert_eq!(
        kick_member(&world.db, world.ada, world.server, world.nils).unwrap(),
        MemberAuthorization::NotFound
    );
    // Without the permission it is a refusal whoever the target is.
    assert_eq!(
        kick_member(&world.db, world.ilya, world.server, Id::random()).unwrap(),
        MemberAuthorization::Forbidden
    );
    // Banning needs its own permission.
    world.set_permissions(world.moderator_role, MODERATE_MEMBERS);
    assert!(
        kick_member(&world.db, world.ada, world.server, world.jo)
            .unwrap()
            .is_allowed()
    );
    assert_eq!(
        ban_member(&world.db, world.ada, world.server, world.jo).unwrap(),
        MemberAuthorization::Forbidden
    );
}

#[test]
fn an_allowed_moderation_hands_back_both_memberships() {
    let world = World::new();

    let MemberAuthorization::Allowed { actor, target } =
        kick_member(&world.db, world.ada, world.server, world.jo).unwrap()
    else {
        panic!("Ada may remove Jo");
    };

    assert_eq!(
        (actor.user, actor.role.id),
        (world.ada, world.moderator_role)
    );
    assert_eq!((target.user, target.role.id), (world.jo, world.base_role));
}

#[test]
fn voice_moderation_is_decided_in_the_room_and_a_room_the_actor_does_not_see_is_not_found() {
    let world = World::new();
    let second = Id::random();
    world.insert_channel(second, None, ChannelKind::Voice, "second");
    let kick =
        |room| kick_voice_member(&world.db, world.ada, world.server, room, world.jo).unwrap();

    assert!(kick(world.voice).is_allowed());
    // A text channel is not a voice room.
    assert_eq!(kick(world.general), MemberAuthorization::NotFound);
    assert_eq!(kick(Id::random()), MemberAuthorization::NotFound);
    // A room where moderating is denied is a refusal, one that is not seen is not found.
    world.override_of(false, second, world.moderator_role, NONE, MODERATE_MEMBERS);
    assert_eq!(kick(second), MemberAuthorization::Forbidden);
    world.override_of(false, world.voice, world.moderator_role, NONE, VIEW_CHANNEL);
    assert_eq!(kick(world.voice), MemberAuthorization::NotFound);
}

#[test]
fn moving_in_voice_needs_to_see_the_destination_and_to_moderate_both_rooms() {
    let world = World::new();
    let destination = Id::random();
    world.insert_channel(destination, None, ChannelKind::Voice, "destination");
    let moved = |to| {
        move_voice_member(
            &world.db,
            world.ada,
            world.server,
            world.voice,
            to,
            world.jo,
        )
        .unwrap()
    };

    assert!(moved(destination).is_allowed());
    world.override_of(
        false,
        destination,
        world.moderator_role,
        NONE,
        MODERATE_MEMBERS,
    );
    assert_eq!(moved(destination), MemberAuthorization::Forbidden);
    world.db.execute(
        "UPDATE \"PermissionOverrides\" SET \"DenyMask\" = ?1 WHERE \"TargetId\" = ?2 AND \"RoleId\" = ?3",
        rusqlite::params![MODERATE_MEMBERS | VIEW_CHANNEL, destination, world.moderator_role],
    )
    .unwrap();
    // The unfound destination wins over every other refusal.
    assert_eq!(moved(destination), MemberAuthorization::NotFound);
    assert_eq!(moved(world.general), MemberAuthorization::NotFound);
}

#[test]
fn changing_a_members_own_override_follows_the_rules_of_a_role_override_with_the_member_for_the_role()
 {
    let s = Scenario::new();
    let change = |actor: Id, target: Id, section: bool, allow: i64, deny: i64| {
        can_change_member_override(&s.world.db, actor, &s.scope(section), target, allow, deny)
            .unwrap()
    };

    assert!(change(s.actor, s.target, false, MANAGE_MESSAGES, NONE));
    assert!(change(s.actor, s.target, true, MANAGE_MESSAGES, NONE));
    // Never oneself, never the owner, never beyond what the actor holds.
    assert!(!change(s.actor, s.actor, false, MANAGE_MESSAGES, NONE));
    assert!(!change(s.actor, s.world.maja, false, MANAGE_MESSAGES, NONE));
    assert!(!change(s.actor, s.target, false, MANAGE_BANS, NONE));
    assert!(!change(s.actor, s.target, false, NONE, MANAGE_BANS));
    assert!(!change(s.actor, s.target, false, ADMINISTRATOR, NONE));
    // Removing a deny needs the power as well.
    s.world
        .member_override(false, s.channel, s.target, NONE, MANAGE_BANS);
    assert!(!change(s.actor, s.target, false, NONE, NONE));
    // A peer is out of reach and a disabled target still has a rank.
    s.world.set_position(s.lower, 10);
    assert!(!change(s.actor, s.target, true, MANAGE_MESSAGES, NONE));
    s.world.set_position(s.lower, 20);
    s.world.disable(s.target);
    assert!(change(s.actor, s.target, true, MANAGE_MESSAGES, NONE));
    // A local deny of managing roles stops the actor here.
    s.override_of(s.actor_role, false, NONE, MANAGE_ROLES);
    assert!(!change(s.actor, s.target, false, MANAGE_MESSAGES, NONE));
}

#[test]
fn moving_a_channel_between_sections_is_a_permission_change_where_their_overrides_differ() {
    let s = Scenario::new();
    let other = s.world.section();
    let moves = |destination: Option<Id>| {
        move_channel(&s.world.db, s.actor, s.world.server, s.channel, destination).unwrap()
    };
    s.world.set_permissions(
        s.actor_role,
        MANAGE_ROLES | MANAGE_CHANNELS | MANAGE_MESSAGES,
    );

    // Nothing differs: allowed, and not a change of permissions.
    assert_eq!(
        moves(Some(other)),
        ChannelMove {
            allowed: true,
            changes_permissions: false
        }
    );
    assert_eq!(
        moves(None),
        ChannelMove {
            allowed: true,
            changes_permissions: false
        }
    );

    // A role's section override that differs, within what the actor may delegate.
    s.world
        .override_of(true, other, s.lower, MANAGE_MESSAGES, NONE);
    assert_eq!(
        moves(Some(other)),
        ChannelMove {
            allowed: true,
            changes_permissions: true
        }
    );
    // Beyond what they hold, or of a role they do not outrank, is refused.
    s.world
        .override_of(true, s.section, s.destination, MANAGE_BANS, NONE);
    assert_eq!(
        moves(Some(other)),
        ChannelMove {
            allowed: false,
            changes_permissions: true
        }
    );
    // The channel's own override decides the bit anyway, so it changes nothing.
    s.world
        .override_of(false, s.channel, s.destination, NONE, MANAGE_BANS);
    assert_eq!(
        moves(Some(other)),
        ChannelMove {
            allowed: true,
            changes_permissions: true
        }
    );
    // Moving into a section that is not on the server, or without managing channels there, is refused.
    assert!(!moves(Some(Id::random())).allowed);
    s.world
        .override_of(true, other, s.actor_role, NONE, MANAGE_CHANNELS);
    assert!(!moves(Some(other)).allowed);
}

#[test]
fn moving_a_channel_counts_the_section_overrides_of_members_too() {
    let s = Scenario::new();
    let other = s.world.section();
    s.world.set_permissions(
        s.actor_role,
        MANAGE_ROLES | MANAGE_CHANNELS | MANAGE_MESSAGES,
    );
    s.world
        .member_override(true, other, s.target, MANAGE_MESSAGES, NONE);
    let moves =
        || move_channel(&s.world.db, s.actor, s.world.server, s.channel, Some(other)).unwrap();

    assert_eq!(
        moves(),
        ChannelMove {
            allowed: true,
            changes_permissions: true
        }
    );
    s.world
        .member_override(true, other, s.world.ilya, MANAGE_BANS, NONE);
    assert_eq!(
        moves(),
        ChannelMove {
            allowed: false,
            changes_permissions: true
        }
    );
    // Without managing roles the actor may move it only where nothing differs.
    s.world.set_permissions(s.actor_role, MANAGE_CHANNELS);
    assert!(!moves().allowed);
}

#[test]
fn the_bits_a_move_can_change_are_those_two_sections_decide_differently_and_the_channel_does_not() {
    use super::evaluator::Masks;
    let a = Masks::new(MANAGE_BANS, NONE);
    let b = Masks::new(NONE, MANAGE_BANS);
    let none = Masks::default();

    assert_eq!(changed(a, b, none), MANAGE_BANS);
    assert_eq!(changed(a, a, none), NONE);
    assert_eq!(changed(a, b, Masks::new(NONE, MANAGE_BANS)), NONE);
    assert_eq!(
        changed(
            none,
            Masks::new(MANAGE_BANS | VIEW_AUDIT_LOG, VIEW_AUDIT_LOG),
            none
        ),
        MANAGE_BANS | VIEW_AUDIT_LOG
    );
}

// ---- Roles a member holds, hand-over, ranking again

#[test]
fn giving_and_taking_roles_keeps_the_top_role_the_highest_held_and_stores_it() {
    let world = World::new();
    let low = world.role("low", 8, NONE);
    let high = world.role("high", 4, NONE);
    let base = role_row(&world, world.base_role);
    let user = world.jo;

    let top = super::ownership::set_roles(
        &world.db,
        world.server,
        user,
        &[role_row(&world, low), role_row(&world, high)],
        &base,
    )
    .unwrap();

    assert_eq!(top.id, high);
    assert_eq!(member_row(&world, user).role.id, high);
    let held = |user| {
        let table = HeldRoleTable::load(&world.db, world.server).unwrap();
        table
            .of(user)
            .iter()
            .map(|role| role.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(held(user), [high, low]);

    let top = super::ownership::set_roles(
        &world.db,
        world.server,
        user,
        &[role_row(&world, low), base],
        &base,
    )
    .unwrap();
    assert_eq!(top.id, low);
    assert_eq!(held(user), [low]);
    let top = super::ownership::set_roles(&world.db, world.server, user, &[], &base).unwrap();
    assert_eq!(top.id, world.base_role);
    assert!(held(user).is_empty());
}

#[test]
fn ranking_again_puts_every_top_role_right_and_counts_a_leaving_role_as_held_by_nobody() {
    let world = World::new();
    let low = world.role("low", 8, NONE);
    let high = world.role("high", 4, NONE);
    let user = world.user("two");
    world.member_of(user, &[low, high]);
    assert_eq!(member_row(&world, user).role.id, high);

    // Positions swap: the top role is stale until ranked again.
    world.set_position(low, 1);
    super::ownership::rank_again(&world.db, world.server, None).unwrap();
    assert_eq!(member_row(&world, user).role.id, low);

    super::ownership::rank_again(&world.db, world.server, Some(low)).unwrap();
    assert_eq!(member_row(&world, user).role.id, high);
    super::ownership::rank_again(&world.db, world.server, Some(high)).unwrap();
    // `high` was the stored top, `low` is still held, so the next ranks by it.
    assert_eq!(member_row(&world, user).role.id, low);
}

#[test]
fn a_hand_over_moves_the_owner_role_and_slot_and_both_keep_their_other_roles() {
    let world = World::new();
    let helper = world.role("helper", 4, MANAGE_MESSAGES);
    let guard = world.role("guard", 6, MODERATE_MEMBERS);
    let base = role_row(&world, world.base_role);
    super::ownership::set_roles(
        &world.db,
        world.server,
        world.maja,
        &[role_row(&world, world.owner_role), role_row(&world, helper)],
        &base,
    )
    .unwrap();
    super::ownership::set_roles(
        &world.db,
        world.server,
        world.jo,
        &[role_row(&world, guard)],
        &base,
    )
    .unwrap();
    world.member_override(false, world.general, world.jo, MANAGE_BANS, NONE);
    world.member_override(false, world.general, world.maja, MANAGE_BANS, NONE);

    super::ownership::hand_over(
        &world.db,
        world.server,
        &member_row(&world, world.maja),
        &member_row(&world, world.jo),
    )
    .unwrap();

    let (old, new) = (member_row(&world, world.maja), member_row(&world, world.jo));
    assert_eq!((old.owner_slot, new.owner_slot), (None, Some(1)));
    assert_eq!((old.role.id, new.role.id), (helper, world.owner_role));
    let table = HeldRoleTable::load(&world.db, world.server).unwrap();
    assert_eq!(
        table
            .of(world.maja)
            .iter()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        [helper]
    );
    assert_eq!(
        table.of(world.jo).iter().map(|r| r.id).collect::<Vec<_>>(),
        [world.owner_role, guard]
    );
    // The successor's own overrides are gone, the old owner's stay.
    let overrides: i64 = world
        .db
        .query_row(
            "SELECT COUNT(*) FROM \"MemberPermissionOverrides\" WHERE \"UserId\" = ?1",
            [world.jo],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(overrides, 0);
    let kept: i64 = world
        .db
        .query_row(
            "SELECT COUNT(*) FROM \"MemberPermissionOverrides\" WHERE \"UserId\" = ?1",
            [world.maja],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 1);
    assert_eq!(server_wide(&world.db, world.jo, world.server).unwrap(), ALL);
}

#[test]
fn deleting_a_server_takes_its_roles_and_memberships_with_it() {
    let world = World::new();

    assert!(super::ownership::delete_server(&world.db, world.server).unwrap());

    let roles: i64 = world
        .db
        .query_row("SELECT COUNT(*) FROM \"ServerRoles\"", [], |r| r.get(0))
        .unwrap();
    let members: i64 = world
        .db
        .query_row("SELECT COUNT(*) FROM \"ServerMembers\"", [], |r| r.get(0))
        .unwrap();
    assert_eq!((roles, members), (0, 0));
    assert!(!super::ownership::delete_server(&world.db, world.server).unwrap());
}

// ---- The instance's own rules

#[test]
fn only_the_owner_of_the_first_server_administers_the_instance() {
    let world = World::new();
    let later = World::new();
    // A second server in the same file made later does not count.
    let _ = &later;

    assert_eq!(
        super::membership::find_administered_server(&world.db, world.maja).unwrap(),
        Some(world.server)
    );
    assert_eq!(
        super::membership::find_administered_server(&world.db, world.ada).unwrap(),
        None
    );
    assert!(super::membership::is_instance_admin(&world.db, world.maja).unwrap());
    assert!(!super::membership::is_instance_admin(&world.db, world.ada).unwrap());
    // Managing the server on any role of the several held makes an administrator of the instance.
    let manager = world.role("manager", 3, MANAGE_SERVER);
    let user = world.user("manager");
    world.member_of(user, &[manager, world.moderator_role]);
    assert!(super::membership::is_instance_admin(&world.db, user).unwrap());
    assert_eq!(
        super::membership::find_administered_server(&world.db, user).unwrap(),
        None
    );
}

#[test]
fn the_instance_server_is_the_first_created() {
    let world = World::new();
    world.db.execute("INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, 'Later', '', 0, '2999-01-01 00:00:00', 0, 0)", [Id::random()]).unwrap();

    assert_eq!(
        super::membership::instance_server(&world.db).unwrap(),
        Some(world.server)
    );
}

#[test]
fn the_active_server_is_the_one_named_else_the_first_joined() {
    let world = World::new();
    let other = Id::random();
    world.db.execute("INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, 'Other', '', 0, ?2, 0, 0)", rusqlite::params![other, crate::values::Timestamp::now()]).unwrap();
    let role = Id::random();
    world.db.execute("INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, 'Member', '#777', 0, 1, 1, 0)", rusqlite::params![role, other]).unwrap();
    world.db.execute("INSERT INTO \"ServerMembers\" (\"ServerId\", \"UserId\", \"RoleId\", \"JoinedAt\") VALUES (?1, ?2, ?3, '2999-01-01 00:00:00')", rusqlite::params![other, world.jo, role]).unwrap();
    use super::membership::{Choice, choose};

    assert_eq!(
        choose(&world.db, world.jo, None).unwrap(),
        Choice::Active(world.server)
    );
    assert_eq!(
        choose(&world.db, world.jo, Some("  ")).unwrap(),
        Choice::Active(world.server)
    );
    assert_eq!(
        choose(&world.db, world.jo, Some(&other.to_string())).unwrap(),
        Choice::Active(other)
    );
    assert_eq!(
        choose(
            &world.db,
            world.jo,
            Some(&format!(" {{{}}} ", other.to_string().to_uppercase()))
        )
        .unwrap(),
        Choice::Active(other)
    );
    assert_eq!(
        choose(&world.db, world.ilya, Some(&other.to_string())).unwrap(),
        Choice::NotMember
    );
    assert_eq!(
        choose(
            &world.db,
            world.ilya,
            Some("11111111-1111-1111-1111-111111111111")
        )
        .unwrap(),
        Choice::NotMember
    );
    assert_eq!(
        choose(&world.db, world.ilya, Some("nope")).unwrap(),
        Choice::Malformed
    );
    assert_eq!(
        choose(&world.db, Id::random(), None).unwrap(),
        Choice::NotMember
    );
}

#[test]
fn the_server_a_request_names_comes_from_the_header_or_else_the_query() {
    use super::membership::{SERVER_HEADER, selection};
    use axum::http::{HeaderMap, HeaderValue};
    let mut headers = HeaderMap::new();

    assert_eq!(selection(&headers, None), None);
    assert_eq!(
        selection(&headers, Some("a=1&serverId=abc&serverId=def")).as_deref(),
        Some("abc")
    );
    assert_eq!(
        selection(&headers, Some("SERVERID=a%2Db+c")).as_deref(),
        Some("a-b c")
    );
    assert_eq!(selection(&headers, Some("serverid")).as_deref(), Some(""));
    assert_eq!(selection(&headers, Some("other=1")), None);
    assert_eq!(
        selection(&headers, Some("serverId=%zz%4")).as_deref(),
        Some("%zz%4")
    );
    headers.insert(SERVER_HEADER, HeaderValue::from_static("from-header"));
    assert_eq!(
        selection(&headers, Some("serverId=abc")).as_deref(),
        Some("from-header")
    );
    // A header that is there but empty is the first server's answer too: the query is not asked.
    headers.insert(SERVER_HEADER, HeaderValue::from_static(""));
    assert_eq!(
        selection(&headers, Some("serverId=abc")).as_deref(),
        Some("")
    );
}

#[test]
fn an_id_is_read_in_every_spelling_the_first_server_took() {
    use super::membership::parse_guid;
    let id = Id::parse("0123abcd-4567-89ef-0123-456789abcdef").unwrap();

    for spelling in [
        "0123abcd-4567-89ef-0123-456789abcdef",
        "0123ABCD-4567-89EF-0123-456789ABCDEF",
        "0123abcd456789ef0123456789abcdef",
        "{0123abcd-4567-89ef-0123-456789abcdef}",
        "(0123abcd-4567-89ef-0123-456789abcdef)",
        " \t0123abcd-4567-89ef-0123-456789abcdef\n ",
        "{0x0123abcd,0x4567,0x89ef,{0x01,0x23,0x45,0x67,0x89,0xab,0xcd,0xef}}",
    ] {
        assert_eq!(parse_guid(spelling), Some(id), "{spelling:?}");
    }
    for refused in [
        "",
        "x",
        "0123abcd-4567-89ef-0123-456789abcde",
        "{0123abcd-4567-89ef-0123-456789abcdef",
        "0123abcd-4567-89ef-0123-456789abcdeg",
        "0123abcd45-67-89ef-0123-456789abcdef",
    ] {
        assert_eq!(parse_guid(refused), None, "{refused:?}");
    }
}

// ---- What a check costs

#[test]
fn a_check_of_a_permission_costs_a_fixed_handful_of_statements_whatever_the_server_holds() {
    let mut world = World::new();
    let section = world.section();
    let channel = world.channel(Some(section));
    for number in 0..40 {
        let user = world.user(&format!("filler{number}"));
        world.admit(user, world.base_role);
        let other = world.channel(Some(section));
        world.override_of(false, other, world.base_role, NONE, SEND_MESSAGES);
    }
    world.count_statements();

    effective(&world.db, world.ilya, &Scope::server(world.server)).unwrap();
    let server_wide_check = world.statements();
    effective(
        &world.db,
        world.ilya,
        &Scope::channel(world.server, channel),
    )
    .unwrap();
    let channel_check = world.statements();
    visibility::find(&world.db, world.ilya, world.server, channel, None).unwrap();
    let open_find = world.statements();
    world.deny_view_to_base(world.projects, false);
    world.statements();
    visibility::find(&world.db, world.ilya, world.server, world.projects, None).unwrap();
    let find_on_a_closed_server = world.statements();
    visibility::audience_of(&world.db, world.server, world.projects).unwrap();
    let audience = world.statements();

    // One for the roles, one for the member, one for the roles they hold.
    assert_eq!(server_wide_check.len(), 3, "{server_wide_check:#?}");
    // Those, with one for where the channel is, and one for each kind of override.
    assert_eq!(channel_check.len(), 6, "{channel_check:#?}");
    // An open channel needs only the one statement that says it is open.
    assert_eq!(open_find.len(), 1, "{open_find:#?}");
    // A channel somebody is denied view at asks as a channel check does, once more for its openness.
    assert_eq!(
        find_on_a_closed_server.len(),
        6,
        "{find_on_a_closed_server:#?}"
    );
    assert_eq!(audience.len(), 6, "{audience:#?}");
}

#[test]
fn a_snapshot_names_the_overrides_it_loaded_and_refuses_those_it_did_not() {
    let world = World::new();
    let snapshot = Snapshot::load(
        &world.db,
        world.server,
        Some(&[world.jo]),
        Targets::at(None, Some(world.general)),
    )
    .unwrap();
    let subject = snapshot.subject(world.jo).unwrap();

    assert!(
        snapshot
            .evaluate(subject, None, Some(world.general))
            .is_ok()
    );
    // An override that was never read would look like none, which reads as a grant.
    assert!(
        snapshot
            .evaluate(subject, None, Some(world.projects))
            .is_err()
    );
    assert!(
        snapshot
            .evaluate(subject, Some(Id::random()), None)
            .is_err()
    );
    assert!(snapshot.evaluate(subject, None, None).is_ok());
    assert!(snapshot.subject(world.ada).is_none());
}

#[test]
fn a_server_with_many_places_is_read_whole_and_the_answers_stay_the_same() {
    let world = World::new();
    let section = world.section();
    let mut channels = Vec::new();
    for _ in 0..450 {
        channels.push(world.channel(Some(section)));
    }
    world.override_of(false, channels[449], world.base_role, NONE, VIEW_CHANNEL);
    world.override_of(true, section, world.moderator_role, MANAGE_MESSAGES, NONE);

    let layout = visibility::ServerLayout::load(&world.db, world.server).unwrap();
    let everyone = ServerVisibility::load(&world.db, world.server).unwrap();

    assert_eq!(layout.channels.len(), 453);
    assert!(
        !everyone
            .view_of(world.jo)
            .unwrap()
            .channels
            .contains(&channels[449])
    );
    assert!(
        everyone
            .view_of(world.jo)
            .unwrap()
            .channels
            .contains(&channels[0])
    );
    assert!(
        !everyone
            .view_of(world.ada)
            .unwrap()
            .channels
            .contains(&channels[449])
    );
    assert!(
        everyone
            .view_of(world.maja)
            .unwrap()
            .channels
            .contains(&channels[449])
    );
    assert!(holds(&world, world.ada, channels[3], MANAGE_MESSAGES));
}
