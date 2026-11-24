//! What a member may do, worked out the way Discord does it, as one pure
//! function. Nothing here reads the database: callers load the inputs, and
//! every decision of the policy comes through here.
//!
//! The owner may do everything, and so may anybody whose roles grant
//! Administrator, wherever they are. Anybody else starts from the base role's
//! permissions united with those of every role they hold; each level then
//! removes the base role's denies and adds its allows, then removes what any
//! held role denies and adds what any held role allows, so that an allow of
//! one role beats a deny of another, and finally applies the member's own
//! override. In a channel, whoever ends up without ViewChannel holds nothing:
//! the channel does not exist for them, so neither does anything it would let
//! them do.

use super::privileges::{ADMINISTRATOR, ALL, NONE, OVERRIDABLE, VIEW_CHANNEL};

/// What one override says at one level: the bits it allows and the bits it
/// denies. An override that does not exist is `Masks::default()`, which
/// changes nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Masks {
    pub allow: i64,
    pub deny: i64,
}

impl Masks {
    pub const fn new(allow: i64, deny: i64) -> Masks {
        Masks { allow, deny }
    }

    /// The bits the override really grants. The API never stores a bit as
    /// both allowed and denied, but a row written by hand may, and inside one
    /// override such a bit stays denied.
    pub const fn granted(self) -> i64 {
        self.allow & !self.deny
    }

    /// Every bit the override decides, either way.
    pub const fn touched(self) -> i64 {
        self.allow | self.deny
    }

    const fn apply(self, held: i64) -> i64 {
        (held & !(self.deny & OVERRIDABLE)) | (self.granted() & OVERRIDABLE)
    }
}

/// The overrides that concern one member at one level, a section or a
/// channel: the base role's, those of the roles the member holds, and the
/// member's own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Level {
    pub base: Masks,
    pub held: Vec<Masks>,
    pub member: Masks,
}

/// Everything the evaluation of one member at one place reads. The levels run
/// from the outside in: a channel in a section has the section's and then the
/// channel's, a channel outside any section only its own, a section its own,
/// and the server none.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub is_owner: bool,
    pub base: i64,
    pub held: &'a [i64],
    pub levels: &'a [Level],
    /// Whether the place is a channel, where somebody without ViewChannel
    /// holds nothing at all.
    pub at_channel: bool,
}

pub fn evaluate(input: &Input) -> i64 {
    if input.is_owner {
        return ALL;
    }
    let mut held = input.held.iter().fold(input.base, |all, one| all | one);
    // Administrator comes from the roles alone and passes every override, as
    // on Discord; what rank allows is decided elsewhere and still holds.
    if held & ADMINISTRATOR == ADMINISTRATOR {
        return ALL;
    }
    for level in input.levels {
        held = apply(held, level);
    }
    // The one place the rule lives. A section is not held to it: who may
    // manage a section is asked there, whether or not they see a channel in
    // it, and its overrides reach its channels through the levels.
    if input.at_channel && held & VIEW_CHANNEL != VIEW_CHANNEL {
        NONE
    } else {
        held
    }
}

/// What the member may do server-wide, before any override: the same
/// evaluation with no levels.
pub fn server_wide(is_owner: bool, base: i64, held: &[i64]) -> i64 {
    evaluate(&Input {
        is_owner,
        base,
        held,
        levels: &[],
        at_channel: false,
    })
}

/// One level applied to what the member holds from the levels outside it.
pub fn apply(held: i64, level: &Level) -> i64 {
    let held = level.base.apply(held);
    let denied = level.held.iter().fold(NONE, |all, role| all | role.deny);
    let allowed = level
        .held
        .iter()
        .fold(NONE, |all, role| all | role.granted());
    let held = (held & !(denied & OVERRIDABLE)) | (allowed & OVERRIDABLE);
    level.member.apply(held)
}

#[cfg(test)]
mod tests {
    use super::super::privileges::*;
    use super::*;

    fn allow(bits: i64) -> Masks {
        Masks::new(bits, NONE)
    }

    fn deny(bits: i64) -> Masks {
        Masks::new(NONE, bits)
    }

    fn level(base: Masks, held: Vec<Masks>, member: Masks) -> Level {
        Level { base, held, member }
    }

    fn member(base: i64, held: &[i64], levels: &[Level]) -> i64 {
        evaluate(&Input {
            is_owner: false,
            base,
            held,
            levels,
            at_channel: false,
        })
    }

    fn in_channel(base: i64, held: &[i64], levels: &[Level]) -> i64 {
        evaluate(&Input {
            is_owner: false,
            base,
            held,
            levels,
            at_channel: true,
        })
    }

    const MESSAGES: i64 = MANAGE_MESSAGES;
    const BANS: i64 = MANAGE_BANS;
    const CHANNELS: i64 = MANAGE_CHANNELS;
    const AUDIT: i64 = VIEW_AUDIT_LOG;
    const VIEW: i64 = VIEW_CHANNEL;
    const SEND: i64 = SEND_MESSAGES;
    const ADMIN: i64 = ADMINISTRATOR;

    #[test]
    fn server_wide_is_the_base_role_united_with_every_held_role() {
        assert_eq!(member(MESSAGES, &[], &[]), MESSAGES);
        assert_eq!(
            member(AUDIT, &[MESSAGES, BANS], &[]),
            AUDIT | MESSAGES | BANS
        );
        assert_eq!(member(NONE, &[], &[]), NONE);
    }

    #[test]
    fn the_owner_skips_every_override_and_holds_bits_no_role_grants() {
        let everything = level(deny(ALL), vec![deny(ALL)], deny(ALL));
        let input = Input {
            is_owner: true,
            base: NONE,
            held: &[NONE],
            levels: &[everything],
            at_channel: true,
        };

        assert_eq!(evaluate(&input), ALL);
    }

    #[test]
    fn an_administrator_skips_overrides_and_no_override_gives_or_takes_the_bit() {
        let everything = level(deny(ALL), vec![deny(ALL)], deny(ALL));
        assert_eq!(
            member(NONE, &[ADMIN], std::slice::from_ref(&everything)),
            ALL
        );
        assert_eq!(
            member(
                ADMIN,
                &[],
                &[level(Masks::default(), vec![], deny(MESSAGES))]
            ),
            ALL
        );
        assert_eq!(
            member(
                NONE,
                &[NONE],
                &[
                    level(allow(ADMIN | BANS), vec![allow(ADMIN)], allow(ADMIN)),
                    level(allow(ADMIN), vec![], Masks::default()),
                ],
            ),
            BANS
        );
        assert_eq!(
            member(
                MESSAGES,
                &[NONE],
                &[level(deny(ADMIN), vec![deny(ADMIN)], deny(ADMIN))],
            ),
            MESSAGES
        );
    }

    #[test]
    fn one_level_one_source_at_a_time() {
        let none = Masks::default();
        assert_eq!(
            member(MESSAGES | BANS, &[], &[level(deny(MESSAGES), vec![], none)]),
            BANS
        );
        assert_eq!(member(NONE, &[], &[level(allow(BANS), vec![], none)]), BANS);
        assert_eq!(
            member(
                NONE,
                &[MESSAGES],
                &[level(none, vec![deny(MESSAGES)], none)]
            ),
            NONE
        );
        assert_eq!(
            member(NONE, &[NONE], &[level(none, vec![allow(CHANNELS)], none)]),
            CHANNELS
        );
        assert_eq!(
            member(NONE, &[MESSAGES], &[level(none, vec![], deny(MESSAGES))]),
            NONE
        );
        assert_eq!(member(NONE, &[], &[level(none, vec![], allow(BANS))]), BANS);
    }

    #[test]
    fn one_roles_allow_beats_another_roles_deny_whichever_ranks_higher() {
        let none = Masks::default();
        for roles in [
            vec![deny(MESSAGES), allow(MESSAGES)],
            vec![allow(MESSAGES), deny(MESSAGES)],
        ] {
            assert_eq!(
                member(MESSAGES, &[NONE, NONE], &[level(none, roles, none)]),
                MESSAGES
            );
        }
        assert_eq!(
            member(
                MESSAGES | BANS,
                &[NONE, NONE],
                &[level(none, vec![deny(MESSAGES), allow(BANS)], none)],
            ),
            BANS
        );
    }

    #[test]
    fn within_a_level_the_held_roles_come_after_the_base_role_and_before_the_member() {
        let none = Masks::default();
        assert_eq!(
            member(NONE, &[NONE], &[level(allow(BANS), vec![deny(BANS)], none)]),
            NONE
        );
        assert_eq!(
            member(BANS, &[NONE], &[level(deny(BANS), vec![allow(BANS)], none)]),
            BANS
        );
        assert_eq!(
            member(NONE, &[NONE], &[level(none, vec![allow(BANS)], deny(BANS))]),
            NONE
        );
        assert_eq!(
            member(BANS, &[NONE], &[level(none, vec![deny(BANS)], allow(BANS))]),
            BANS
        );
    }

    #[test]
    fn a_bit_both_allowed_and_denied_in_one_override_stays_denied() {
        let none = Masks::default();
        assert_eq!(
            member(
                NONE,
                &[],
                &[level(Masks::new(BANS | AUDIT, BANS), vec![], none)]
            ),
            AUDIT
        );
        // The same inside one role's override does not beat another role's deny.
        assert_eq!(
            member(
                BANS,
                &[NONE, NONE],
                &[level(none, vec![Masks::new(BANS, BANS), deny(BANS)], none)],
            ),
            NONE
        );
    }

    #[test]
    fn a_section_level_applies_before_the_channel_level() {
        let none = Masks::default();
        let section = |l: Level| l;
        // The channel has the last word over the section, in both directions.
        assert_eq!(
            member(
                MESSAGES,
                &[],
                &[
                    section(level(deny(MESSAGES), vec![], none)),
                    level(allow(MESSAGES), vec![], none)
                ]
            ),
            MESSAGES
        );
        assert_eq!(
            member(
                NONE,
                &[NONE],
                &[
                    level(none, vec![allow(MESSAGES)], none),
                    level(none, vec![deny(MESSAGES)], none)
                ]
            ),
            NONE
        );
        // The section's decision stands where the channel is silent.
        assert_eq!(
            member(
                NONE,
                &[NONE],
                &[
                    level(none, vec![allow(CHANNELS)], none),
                    level(none, vec![deny(MESSAGES)], none)
                ]
            ),
            CHANNELS
        );
        // Order across sources: the channel's base role beats the section's role,
        // and the channel's role beats the section's member.
        assert_eq!(
            member(
                MESSAGES,
                &[NONE],
                &[
                    level(none, vec![deny(MESSAGES)], none),
                    level(allow(MESSAGES), vec![], none)
                ]
            ),
            MESSAGES
        );
        assert_eq!(
            member(
                NONE,
                &[NONE],
                &[
                    level(none, vec![], allow(BANS)),
                    level(none, vec![deny(BANS)], none)
                ]
            ),
            NONE
        );
        // A channel outside a section has its own level only.
        assert_eq!(
            member(
                MESSAGES,
                &[NONE],
                &[level(none, vec![deny(MESSAGES)], none)]
            ),
            NONE
        );
    }

    #[test]
    fn whoever_lacks_view_at_a_channel_holds_nothing_there() {
        let none = Masks::default();
        assert_eq!(
            in_channel(VIEW | SEND, &[], &[level(none, vec![], none)]),
            VIEW | SEND
        );
        assert_eq!(
            in_channel(SEND | MESSAGES, &[BANS], &[level(none, vec![], none)]),
            NONE
        );
        assert_eq!(
            in_channel(VIEW | SEND, &[MESSAGES], &[level(deny(VIEW), vec![], none)]),
            NONE
        );
        // View denied at the section reaches its channels, and the channel can allow it again.
        assert_eq!(
            in_channel(
                VIEW | SEND,
                &[],
                &[level(deny(VIEW), vec![], none), level(none, vec![], none)]
            ),
            NONE
        );
        assert_eq!(
            in_channel(
                VIEW | SEND,
                &[],
                &[
                    level(deny(VIEW), vec![], none),
                    level(allow(VIEW), vec![], none)
                ]
            ),
            VIEW | SEND
        );
        assert_eq!(
            in_channel(
                VIEW | SEND,
                &[NONE],
                &[level(deny(VIEW), vec![allow(VIEW)], none)]
            ),
            VIEW | SEND
        );
        assert_eq!(
            in_channel(
                VIEW | SEND,
                &[NONE],
                &[level(none, vec![allow(VIEW)], deny(VIEW))]
            ),
            NONE
        );
        assert_eq!(
            in_channel(SEND, &[], &[level(deny(VIEW), vec![], allow(VIEW))]),
            VIEW | SEND
        );
        assert_eq!(in_channel(SEND, &[], &[level(none, vec![], none)]), NONE);
        assert_eq!(
            in_channel(NONE, &[], &[level(allow(VIEW | MESSAGES), vec![], none)]),
            VIEW | MESSAGES
        );
    }

    #[test]
    fn the_owner_and_an_administrator_view_every_channel() {
        let hidden = level(deny(VIEW), vec![deny(VIEW)], deny(VIEW));
        let owner = Input {
            is_owner: true,
            base: NONE,
            held: &[],
            levels: std::slice::from_ref(&hidden),
            at_channel: true,
        };
        assert_eq!(evaluate(&owner), ALL);
        assert_eq!(
            in_channel(NONE, &[ADMIN], std::slice::from_ref(&hidden)),
            ALL
        );
    }

    #[test]
    fn at_a_section_without_view_what_it_allows_still_holds() {
        let hidden = level(deny(VIEW), vec![], Masks::default());
        assert_eq!(member(CHANNELS, &[], &[hidden]), CHANNELS);
    }

    #[test]
    fn no_levels_give_the_server_wide_permissions() {
        assert_eq!(server_wide(false, AUDIT, &[MESSAGES]), AUDIT | MESSAGES);
        assert_eq!(server_wide(true, NONE, &[]), ALL);
        assert_eq!(server_wide(false, NONE, &[ADMIN]), ALL);
    }

    #[test]
    fn a_mask_with_bits_no_permission_has_keeps_them_and_the_sign_bit_is_nothing_special() {
        let odd = 1_i64 << 40 | i64::MIN;
        assert_eq!(member(odd, &[], &[]), odd);
        // An override takes away only what it names and only of the permissions
        // there are, so the odd bits stay.
        let denied_everything = level(deny(ALL), vec![], Masks::default());
        assert_eq!(member(odd | MESSAGES, &[], &[denied_everything]), odd);
    }
}
