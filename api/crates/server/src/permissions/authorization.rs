//! What a handler asks: does this person hold this permission here, may they
//! act on that member or that role. Permission checks name their resource;
//! delegation and moderation also check the target and the hierarchy. No
//! decision is cached across requests or hub invocations.
//!
//! Missing resources, disabled accounts and missing memberships always deny.
//!
//! The answers are plain values. What a route says to the person is its own
//! business and part of what the web client sees, so a route turns a
//! [`Refusal`] into the answer it has always given.

use rusqlite::{Connection, OptionalExtension, params};
use stellarr_messages::{MessageKey, Messages, MissingValue};

use super::evaluator::Masks;
use super::privileges::{
    self, MANAGE_BANS, MANAGE_CHANNELS, MANAGE_MEMBERS, MANAGE_ROLES, MODERATE_MEMBERS, NONE,
    VIEW_CHANNEL, contains, is_overridable, is_valid,
};
use super::roles::{Member, Role};
use super::snapshot::{Snapshot, Subject, Target, Targets};
use crate::http::{ApiError, Problem};
use crate::values::Id;

/// The resource whose permissions apply. A channel or section always names
/// its server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    server: Id,
    kind: ScopeKind,
    resource: Id,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Server,
    Section,
    Channel,
}

const NIL: Id = Id::from_bytes([0; 16]);

impl Scope {
    pub fn server(server: Id) -> Scope {
        Scope {
            server,
            kind: ScopeKind::Server,
            resource: server,
        }
    }

    pub fn section(server: Id, section: Id) -> Scope {
        Scope {
            server,
            kind: ScopeKind::Section,
            resource: section,
        }
    }

    pub fn channel(server: Id, channel: Id) -> Scope {
        Scope {
            server,
            kind: ScopeKind::Channel,
            resource: channel,
        }
    }

    pub fn server_id(&self) -> Id {
        self.server
    }

    pub fn is_server(&self) -> bool {
        self.kind == ScopeKind::Server
    }
}

/// Where a scope is evaluated: the section and the channel whose overrides
/// apply, from the outside in, and which of them an override at the scope
/// itself targets.
#[derive(Debug, Clone, Copy)]
struct Place {
    target: Option<Target>,
    section: Option<Id>,
    channel: Option<Id>,
}

impl Place {
    fn targets(&self) -> Targets {
        Targets::at(self.section, self.channel)
    }

    fn evaluate(&self, snapshot: &Snapshot, subject: &Subject) -> rusqlite::Result<i64> {
        snapshot.evaluate(subject, self.section, self.channel)
    }
}

/// The place of a scope, or none for a channel or section that is not on its
/// server.
fn place_of(db: &Connection, scope: &Scope) -> rusqlite::Result<Option<Place>> {
    if scope.server == NIL || scope.resource == NIL {
        return Ok(None);
    }
    Ok(match scope.kind {
        ScopeKind::Server => Some(Place {
            target: None,
            section: None,
            channel: None,
        }),
        ScopeKind::Channel => db
            .query_row(
                "SELECT \"SectionId\" FROM \"Channels\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
                params![scope.resource, scope.server],
                |row| row.get::<_, Option<Id>>(0),
            )
            .optional()?
            .map(|section| Place {
                target: Some(Target::Channel),
                section,
                channel: Some(scope.resource),
            }),
        ScopeKind::Section => db
            .query_row(
                "SELECT 1 FROM \"Sections\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
                params![scope.resource, scope.server],
                |_| Ok(()),
            )
            .optional()?
            .map(|()| Place {
                target: Some(Target::Section),
                section: Some(scope.resource),
                channel: None,
            }),
    })
}

/// What the person holds at the scope: `None` for somebody who is not an
/// enabled member of its server, and for a channel or section that is not on
/// it.
pub fn effective(db: &Connection, user: Id, scope: &Scope) -> rusqlite::Result<Option<i64>> {
    let Some(place) = place_of(db, scope)? else {
        return Ok(None);
    };
    let snapshot = Snapshot::load(db, scope.server, Some(&[user]), place.targets())?;
    match snapshot.subject(user) {
        Some(subject) => place.evaluate(&snapshot, subject).map(Some),
        None => Ok(None),
    }
}

/// What the person holds at the scope, nothing where there is no such place
/// or they are no member.
pub fn held_at(db: &Connection, user: Id, scope: &Scope) -> rusqlite::Result<Held> {
    Ok(Held(effective(db, user, scope)?.unwrap_or(NONE)))
}

/// Whether the person holds every permission of `required` at the scope. A
/// requirement with a bit that is not a permission is never met, and an empty
/// one still needs a real scope and a membership.
pub fn allows(db: &Connection, user: Id, scope: &Scope, required: i64) -> rusqlite::Result<bool> {
    if !is_valid(required) {
        return Ok(false);
    }
    Ok(effective(db, user, scope)?.is_some_and(|held| contains(held, required)))
}

/// The member's server-wide permissions; nothing for anybody who is not an
/// enabled member.
pub fn server_wide(db: &Connection, user: Id, server: Id) -> rusqlite::Result<i64> {
    Ok(effective(db, user, &Scope::server(server))?.unwrap_or(NONE))
}

/// What a person holds at one place, read once for every decision a handler
/// makes there. Without view in a channel it is nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held(pub i64);

impl Held {
    pub fn has(self, permission: i64) -> bool {
        contains(self.0, permission)
    }

    pub fn views(self) -> bool {
        self.has(VIEW_CHANNEL)
    }

    /// Decides one permission after view: a person who does not see the
    /// channel is told it is not there, before any refusal that would say
    /// it is.
    pub fn require(self, permission: i64) -> Result<Held, Refusal> {
        if !self.views() {
            Err(Refusal::Hidden)
        } else if !self.has(permission) {
            Err(Refusal::Denied { permission })
        } else {
            Ok(self)
        }
    }

    /// The channel is there for the person, or it is not.
    pub fn see(self) -> Result<Held, Refusal> {
        if self.views() {
            Ok(self)
        } else {
            Err(Refusal::Hidden)
        }
    }
}

/// Why a person is stopped. A route turns it into its own answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// What was asked for does not exist for the person: a channel they do
    /// not see, a section not listed for them. The route answers as for an id
    /// that does not exist, whether that is a 404, a 400 or a hub text.
    Hidden,
    /// Stopped, with nothing said of why: a 403 without a body.
    Forbidden,
    /// Stopped for want of one permission, which the answer names.
    Denied { permission: i64 },
}

impl Refusal {
    /// The 403 for a member whom one permission stops, in their language, with
    /// the permission's wire name so a client can tell which one it was.
    pub fn problem(
        messages: &Messages,
        reason: MessageKey,
        missing: i64,
    ) -> Result<Problem, MissingValue> {
        let permission = match privileges::name_of(missing) {
            Some(name) => serde_json::Value::from(name),
            None => serde_json::Value::Null,
        };
        Ok(Problem::new(axum::http::StatusCode::FORBIDDEN)
            .title(messages.get(reason, &[])?)
            .with("permission", permission))
    }

    /// The answer of a route: `hidden` for what does not exist for the
    /// person, a bare 403 for a refusal that says nothing, and the problem
    /// that names the permission with `reason` for the one that does.
    pub fn into_error(
        self,
        messages: &Messages,
        reason: MessageKey,
        hidden: ApiError,
    ) -> Result<ApiError, MissingValue> {
        Ok(match self {
            Refusal::Hidden => hidden,
            Refusal::Forbidden => ApiError::forbidden(),
            Refusal::Denied { permission } => {
                ApiError::from(Refusal::problem(messages, reason, permission)?)
            }
        })
    }
}

/// The actor's snapshot, server-wide: its subject is none for anybody who is
/// not an enabled member.
fn actor_snapshot(db: &Connection, actor: Id, server: Id) -> rusqlite::Result<Snapshot> {
    Snapshot::load(db, server, Some(&[actor]), Targets::none())
}

/// Creating a role: the actor needs ManageRoles and must hold every bit the
/// new role grants.
pub fn can_create_role(
    db: &Connection,
    actor: Id,
    server: Id,
    permissions: i64,
) -> rusqlite::Result<bool> {
    let snapshot = actor_snapshot(db, actor, server)?;
    Ok(snapshot.subject(actor).is_some_and(|actor| {
        let held = actor.permissions();
        contains(held, MANAGE_ROLES) && contains(held, permissions)
    }))
}

/// Editing a role to `permissions`: ManageRoles, a top role that outranks the
/// role, and every bit of both the proposed permissions and what the role
/// could already grant, its overrides' allows included.
pub fn can_edit_role(
    db: &Connection,
    actor: Id,
    role: &Role,
    permissions: i64,
) -> rusqlite::Result<bool> {
    let snapshot = actor_snapshot(db, actor, role.server)?;
    let Some(actor) = snapshot.subject(actor) else {
        return Ok(false);
    };
    let held = actor.permissions();
    Ok(contains(held, MANAGE_ROLES)
        && actor.top.outranks(role)
        && contains(held, permissions)
        && contains(held, delegated(db, role)?))
}

/// Deleting a role whose holders fall back to `fallback`: ManageRoles, both
/// roles outranked, not a system role, and every bit either could grant.
pub fn can_delete_role(
    db: &Connection,
    actor: Id,
    role: &Role,
    fallback: &Role,
) -> rusqlite::Result<bool> {
    let snapshot = actor_snapshot(db, actor, role.server)?;
    let Some(actor) = snapshot.subject(actor) else {
        return Ok(false);
    };
    let held = actor.permissions();
    Ok(!role.is_system
        && fallback.server == role.server
        && contains(held, MANAGE_ROLES)
        && actor.top.outranks(role)
        && actor.top.outranks(fallback)
        && contains(held, delegated(db, role)?)
        && contains(held, delegated(db, fallback)?))
}

/// Reordering the roles between the owner role and the base role, from
/// `ranked`, their order now, highest first, to `wanted`, the same roles in a
/// new order. The actor needs ManageRoles and moves only roles their top role
/// outranks: every role it does not, their own and their peers' included, has
/// to keep its place, and those all rank above the ones it does, so nothing
/// can be put at or above the actor's top role.
pub fn can_reorder_roles(
    db: &Connection,
    actor: Id,
    server: Id,
    ranked: &[Role],
    wanted: &[Id],
) -> rusqlite::Result<bool> {
    let snapshot = actor_snapshot(db, actor, server)?;
    let Some(actor) = snapshot.subject(actor) else {
        return Ok(false);
    };
    if !contains(actor.permissions(), MANAGE_ROLES) || ranked.len() != wanted.len() {
        return Ok(false);
    }
    Ok(ranked
        .iter()
        .zip(wanted)
        .all(|(role, want)| actor.top.outranks(role) || *want == role.id))
}

/// Giving roles to a member and taking them away are decided alike, for every
/// role in `changed`: the actor needs ManageMembers, must outrank the member
/// and each role, must not be the member, and must hold everything each role
/// could grant, its overrides' allows included.
pub fn can_change_roles(
    db: &Connection,
    actor: Id,
    target: &Member,
    changed: &[Role],
) -> rusqlite::Result<bool> {
    let snapshot = actor_snapshot(db, actor, target.server)?;
    let Some(actor_subject) = snapshot.subject(actor) else {
        return Ok(false);
    };
    let held = actor_subject.permissions();
    if actor == target.user
        || !contains(held, MANAGE_MEMBERS)
        || !actor_subject.top.outranks(&target.role)
    {
        return Ok(false);
    }
    for role in changed {
        if role.server != target.server
            || !actor_subject.top.outranks(role)
            || !contains(held, delegated(db, role)?)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Setting, clearing and deleting an override use the same decision. Both old
/// and new masks count: removing a deny can grant a power as surely as adding
/// an allow. A local grant cannot delegate powers beyond the actor's
/// server-wide permissions, and a local deny restricts the actor here too.
pub fn can_change_override(
    db: &Connection,
    actor: Id,
    scope: &Scope,
    role: Id,
    allow: i64,
    deny: i64,
) -> rusqlite::Result<bool> {
    if scope.is_server() || !is_overridable(allow, deny) {
        return Ok(false);
    }
    let Some(place) = place_of(db, scope)? else {
        return Ok(false);
    };
    let snapshot = Snapshot::load(db, scope.server, Some(&[actor]), place.targets())?;
    let Some(actor_subject) = snapshot.subject(actor) else {
        return Ok(false);
    };
    let Some(role) = snapshot.roles().iter().find(|one| one.id == role).copied() else {
        return Ok(false);
    };
    let permissions = actor_subject.permissions();
    if !contains(permissions, MANAGE_ROLES) {
        return Ok(false);
    }

    // The owner can repair legacy restrictions on their own role. Other
    // actors cannot edit their own role or a peer/higher role.
    let owner_recovery = actor_subject.is_owner() && actor_subject.top.id == role.id;
    if !owner_recovery && !actor_subject.top.outranks(&role) {
        return Ok(false);
    }

    let effective = place.evaluate(&snapshot, actor_subject)?;
    if !owner_recovery && !contains(effective, MANAGE_ROLES) {
        return Ok(false);
    }
    let target = place.target.unwrap_or(Target::Channel);
    let current = snapshot.role_override(target, scope.resource, role.id)?;
    let touched = allow | deny | current.touched();
    let ceiling = if owner_recovery {
        permissions
    } else {
        permissions & effective
    };
    Ok(contains(ceiling, touched))
}

/// The same decision for a member's own override, with the member in place of
/// the role: the actor must outrank them, which nobody does to themselves or
/// to the owner, and must hold every bit of both the old and the new masks,
/// here as well as server-wide.
pub fn can_change_member_override(
    db: &Connection,
    actor: Id,
    scope: &Scope,
    target: Id,
    allow: i64,
    deny: i64,
) -> rusqlite::Result<bool> {
    if scope.is_server() || !is_overridable(allow, deny) || actor == target {
        return Ok(false);
    }
    let Some(place) = place_of(db, scope)? else {
        return Ok(false);
    };
    let snapshot = Snapshot::load(db, scope.server, Some(&[actor, target]), place.targets())?;
    let Some(actor_subject) = snapshot.subject(actor) else {
        return Ok(false);
    };
    // A disabled account still has its rank and its overrides; only acting
    // needs an enabled one.
    let Some(member) = Member::load(db, scope.server, target)? else {
        return Ok(false);
    };
    let permissions = actor_subject.permissions();
    if !contains(permissions, MANAGE_ROLES) || !actor_subject.top.outranks(&member.role) {
        return Ok(false);
    }
    let effective = place.evaluate(&snapshot, actor_subject)?;
    if !contains(effective, MANAGE_ROLES) {
        return Ok(false);
    }
    let kind = place.target.unwrap_or(Target::Channel);
    let current = snapshot.member_override(kind, scope.resource, target)?;
    Ok(contains(
        permissions & effective,
        allow | deny | current.touched(),
    ))
}

/// What a moderation check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberAuthorization {
    /// The actor may act on the target; both are as their memberships stand.
    Allowed {
        actor: Box<Member>,
        target: Box<Member>,
    },
    Forbidden,
    NotFound,
}

impl MemberAuthorization {
    pub fn is_allowed(&self) -> bool {
        matches!(self, MemberAuthorization::Allowed { .. })
    }
}

pub fn kick_member(
    db: &Connection,
    actor: Id,
    server: Id,
    target: Id,
) -> rusqlite::Result<MemberAuthorization> {
    moderate(db, actor, target, &Scope::server(server), MODERATE_MEMBERS)
}

pub fn ban_member(
    db: &Connection,
    actor: Id,
    server: Id,
    target: Id,
) -> rusqlite::Result<MemberAuthorization> {
    moderate(db, actor, target, &Scope::server(server), MANAGE_BANS)
}

/// A voice channel the actor does not see is not found, before anything is
/// asked of what they may do: they hold nothing there anyway, and a refusal
/// would say the room exists.
pub fn kick_voice_member(
    db: &Connection,
    actor: Id,
    server: Id,
    channel: Id,
    target: Id,
) -> rusqlite::Result<MemberAuthorization> {
    if !sees_voice_channel(db, actor, server, channel)? {
        return Ok(MemberAuthorization::NotFound);
    }
    moderate(
        db,
        actor,
        target,
        &Scope::channel(server, channel),
        MODERATE_MEMBERS,
    )
}

pub fn move_voice_member(
    db: &Connection,
    actor: Id,
    server: Id,
    source: Id,
    destination: Id,
    target: Id,
) -> rusqlite::Result<MemberAuthorization> {
    if !sees_voice_channel(db, actor, server, destination)? {
        return Ok(MemberAuthorization::NotFound);
    }
    let from = kick_voice_member(db, actor, server, source, target)?;
    if !from.is_allowed() {
        return Ok(from);
    }
    if allows(
        db,
        actor,
        &Scope::channel(server, destination),
        MODERATE_MEMBERS,
    )? {
        Ok(from)
    } else {
        Ok(MemberAuthorization::Forbidden)
    }
}

fn moderate(
    db: &Connection,
    actor: Id,
    target: Id,
    scope: &Scope,
    required: i64,
) -> rusqlite::Result<MemberAuthorization> {
    if actor == target {
        return Ok(MemberAuthorization::Forbidden);
    }
    let Some(place) = place_of(db, scope)? else {
        return Ok(MemberAuthorization::Forbidden);
    };
    let snapshot = Snapshot::load(db, scope.server, Some(&[actor, target]), place.targets())?;
    let Some(actor_subject) = snapshot.subject(actor) else {
        return Ok(MemberAuthorization::Forbidden);
    };
    if !contains(place.evaluate(&snapshot, actor_subject)?, required) {
        return Ok(MemberAuthorization::Forbidden);
    }
    let Some(target_subject) = snapshot.subject(target) else {
        return Ok(MemberAuthorization::NotFound);
    };
    if !actor_subject.top.outranks(&target_subject.top) {
        return Ok(MemberAuthorization::Forbidden);
    }
    // Both are enabled members here, so both rows exist.
    match (
        Member::load(db, scope.server, actor)?,
        Member::load(db, scope.server, target)?,
    ) {
        (Some(actor), Some(target)) => Ok(MemberAuthorization::Allowed {
            actor: Box::new(actor),
            target: Box::new(target),
        }),
        _ => Ok(MemberAuthorization::NotFound),
    }
}

/// Whether the channel is a voice channel of the server that the actor sees.
fn sees_voice_channel(
    db: &Connection,
    actor: Id,
    server: Id,
    channel: Id,
) -> rusqlite::Result<bool> {
    Ok(super::visibility::find(
        db,
        actor,
        server,
        channel,
        Some(super::visibility::ChannelKind::Voice),
    )?
    .is_some())
}

/// What a role could grant: its own permissions and every allow of its
/// overrides at a channel or section that exists on its server.
pub fn delegated(db: &Connection, role: &Role) -> rusqlite::Result<i64> {
    let mut statement = db.prepare(
        "SELECT o.\"AllowMask\" FROM \"PermissionOverrides\" o
         WHERE o.\"ServerId\" = ?1 AND o.\"RoleId\" = ?2
           AND ((o.\"TargetType\" = 0 AND EXISTS (SELECT 1 FROM \"Channels\" c WHERE c.\"Id\" = o.\"TargetId\" AND c.\"ServerId\" = ?1))
             OR (o.\"TargetType\" = 1 AND EXISTS (SELECT 1 FROM \"Sections\" s WHERE s.\"Id\" = o.\"TargetId\" AND s.\"ServerId\" = ?1)))",
    )?;
    let grants = statement.query_map(params![role.server, role.id], |row| row.get::<_, i64>(0))?;
    let mut held = role.permissions;
    for grant in grants {
        held |= grant?;
    }
    Ok(held)
}

/// What moving a channel decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelMove {
    pub allowed: bool,
    /// Whether the move changes what somebody may do, and so is a change of
    /// permissions as well.
    pub changes_permissions: bool,
}

/// The bits one role's or one member's section override decides differently in
/// the two sections, less those its channel override decides either way.
pub fn changed(source: Masks, destination: Masks, channel: Masks) -> i64 {
    ((source.granted() ^ destination.granted()) | (source.deny ^ destination.deny))
        & !channel.touched()
}

/// Moving a channel is also a permission change when its section changes what
/// a role or a member may do in it. Such a move obeys the same delegation
/// boundaries as editing the overrides directly.
///
/// Evaluation works bit by bit, so a move can change a bit for somebody only
/// where the section overrides of one of their roles, of the base role or of
/// their own differ between the two sections, and the channel override of that
/// same role or member does not decide the bit anyway. The check asks that of
/// every role, whether anybody holds it yet or not, and of every member with
/// an override in either section.
pub fn move_channel(
    db: &Connection,
    actor: Id,
    server: Id,
    channel: Id,
    destination: Option<Id>,
) -> rusqlite::Result<ChannelMove> {
    let refused = ChannelMove {
        allowed: false,
        changes_permissions: false,
    };
    let Some(source) = db
        .query_row(
            "SELECT \"SectionId\" FROM \"Channels\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
            params![channel, server],
            |row| row.get::<_, Option<Id>>(0),
        )
        .optional()?
    else {
        return Ok(refused);
    };
    if let Some(destination) = destination
        && db
            .query_row(
                "SELECT 1 FROM \"Sections\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
                params![destination, server],
                |_| Ok(()),
            )
            .optional()?
            .is_none()
    {
        return Ok(refused);
    }

    let mut sections: Vec<Id> = Vec::new();
    for section in [source, destination].into_iter().flatten() {
        if !sections.contains(&section) {
            sections.push(section);
        }
    }
    let snapshot = Snapshot::load(
        db,
        server,
        Some(&[actor]),
        Targets {
            sections: sections.clone(),
            channels: vec![channel],
        },
    )?;
    let Some(actor_subject) = snapshot.subject(actor) else {
        return Ok(refused);
    };
    if !contains(
        snapshot.evaluate(actor_subject, destination, None)?,
        MANAGE_CHANNELS,
    ) {
        return Ok(refused);
    }
    let source_held = snapshot.evaluate(actor_subject, source, Some(channel))?;
    if !contains(source_held, MANAGE_CHANNELS) {
        return Ok(refused);
    }
    let destination_held = snapshot.evaluate(actor_subject, destination, Some(channel))?;
    let ceiling = actor_subject.permissions() & source_held & destination_held;

    let role_at = |section: Option<Id>, role: Id| match section {
        Some(section) => snapshot.role_override(Target::Section, section, role),
        None => Ok(Masks::default()),
    };
    let mut changes_permissions = false;
    // The owner role is held by the owner alone, who skips every override.
    for role in snapshot.roles().iter().filter(|role| !role.is_owner) {
        let moved = changed(
            role_at(source, role.id)?,
            role_at(destination, role.id)?,
            snapshot.role_override(Target::Channel, channel, role.id)?,
        );
        if moved == NONE {
            continue;
        }
        changes_permissions = true;
        if !contains(ceiling, MANAGE_ROLES | moved) || !actor_subject.top.outranks(role) {
            return Ok(ChannelMove {
                allowed: false,
                changes_permissions: true,
            });
        }
    }

    let members = member_overrides_for_move(db, server, &sections, channel)?;
    if members.is_empty() {
        return Ok(ChannelMove {
            allowed: true,
            changes_permissions,
        });
    }
    let member_at = |target: Target, place: Option<Id>, user: Id| {
        place
            .and_then(|place| {
                members
                    .iter()
                    .find(|row| row.target == target && row.place == place && row.user == user)
            })
            .map_or_else(Masks::default, |row| row.masks)
    };
    let mut users: Vec<Id> = members.iter().map(|row| row.user).collect();
    users.sort();
    users.dedup();
    for user in users {
        let Some(member) = Member::load(db, server, user)? else {
            continue;
        };
        if member.role.is_owner {
            continue;
        }
        let moved = changed(
            member_at(Target::Section, source, user),
            member_at(Target::Section, destination, user),
            member_at(Target::Channel, Some(channel), user),
        );
        if moved == NONE {
            continue;
        }
        changes_permissions = true;
        if !contains(ceiling, MANAGE_ROLES | moved)
            || user == actor
            || !actor_subject.top.outranks(&member.role)
        {
            return Ok(ChannelMove {
                allowed: false,
                changes_permissions: true,
            });
        }
    }
    Ok(ChannelMove {
        allowed: true,
        changes_permissions,
    })
}

struct MemberOverrideRow {
    target: Target,
    place: Id,
    user: Id,
    masks: Masks,
}

fn member_overrides_for_move(
    db: &Connection,
    server: Id,
    sections: &[Id],
    channel: Id,
) -> rusqlite::Result<Vec<MemberOverrideRow>> {
    let mut statement = db.prepare(
        "SELECT \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\" FROM \"MemberPermissionOverrides\" WHERE \"ServerId\" = ?1",
    )?;
    let mut rows = Vec::new();
    for row in statement.query_map([server], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Id>(1)?,
            row.get::<_, Id>(2)?,
            Masks::new(row.get(3)?, row.get(4)?),
        ))
    })? {
        let (kind, place, user, masks) = row?;
        let target = match kind {
            0 if place == channel => Target::Channel,
            1 if sections.contains(&place) => Target::Section,
            _ => continue,
        };
        rows.push(MemberOverrideRow {
            target,
            place,
            user,
            masks,
        });
    }
    Ok(rows)
}
