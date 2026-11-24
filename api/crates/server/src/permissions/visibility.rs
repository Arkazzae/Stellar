//! Who sees which channel. A channel somebody does not view does not exist for
//! them, so every route and hub method about a channel, or about anything in
//! one, asks here first and answers a non-viewer as it answers for an id that
//! does not exist.
//!
//! Each question starts with the cheap one, whether anybody is denied view at
//! all, which one statement answers. Only where somebody is does the
//! evaluator run, through one snapshot for the whole question. Nothing is
//! cached: a change of roles or overrides counts from the next request or
//! event on.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use super::privileges::{MANAGE_CHANNELS, VIEW_CHANNEL};
use super::snapshot::{Snapshot, Subject, Targets};
use crate::events::Audience;
use crate::int_enum;
use crate::values::Id;

int_enum! {
    /// Whether a channel carries text or voice.
    pub enum ChannelKind { Text = 0, Voice = 1 }
}

/// A channel as the visibility checks see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPlace {
    pub id: Id,
    pub server: Id,
    pub section: Option<Id>,
    pub kind: ChannelKind,
    pub name: String,
    pub slow_mode_seconds: i64,
    /// Nobody is denied view there: the base role holds it and no override of
    /// a role or a member, at the channel or at its section, denies it.
    /// Everybody on the server then sees the channel, and nothing has to be
    /// evaluated.
    pub open: bool,
    /// Whether the person the place was looked up for is an enabled member of
    /// its server; true when it was looked up for nobody in particular.
    pub member: bool,
}

/// Whether nobody is denied view at a channel, as one condition every query
/// that needs it shares. An override at a section another channel is in does
/// not count against this one.
const IS_OPEN: &str = "(COALESCE((SELECT (r.\"Permissions\" & 16384) != 0 FROM \"ServerRoles\" r
        WHERE r.\"ServerId\" = c.\"ServerId\" AND r.\"IsSystem\" = 1 AND r.\"IsOwner\" = 0
        ORDER BY r.\"Position\" DESC, r.\"Id\" LIMIT 1), 0) = 1
    AND NOT EXISTS (SELECT 1 FROM \"PermissionOverrides\" o WHERE o.\"ServerId\" = c.\"ServerId\" AND (o.\"DenyMask\" & 16384) != 0
        AND ((o.\"TargetType\" = 0 AND o.\"TargetId\" = c.\"Id\") OR (o.\"TargetType\" = 1 AND o.\"TargetId\" = c.\"SectionId\")))
    AND NOT EXISTS (SELECT 1 FROM \"MemberPermissionOverrides\" o WHERE o.\"ServerId\" = c.\"ServerId\" AND (o.\"DenyMask\" & 16384) != 0
        AND ((o.\"TargetType\" = 0 AND o.\"TargetId\" = c.\"Id\") OR (o.\"TargetType\" = 1 AND o.\"TargetId\" = c.\"SectionId\")))
)";

/// The channel with whether it is open, and whether `viewer` is an enabled
/// member of its server, in one statement. `server` and `kind` narrow which
/// channel counts.
pub fn place(
    db: &Connection,
    channel: Id,
    server: Option<Id>,
    kind: Option<ChannelKind>,
    viewer: Option<Id>,
) -> rusqlite::Result<Option<ChannelPlace>> {
    db.query_row(
        &format!(
            "SELECT c.\"Id\", c.\"ServerId\", c.\"SectionId\", c.\"Kind\", c.\"Name\", c.\"SlowModeSeconds\", {IS_OPEN},
                    (?4 IS NULL OR EXISTS (SELECT 1 FROM \"ServerMembers\" m JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\"
                       WHERE m.\"ServerId\" = c.\"ServerId\" AND m.\"UserId\" = ?4 AND u.\"IsDisabled\" = 0))
             FROM \"Channels\" c
             WHERE c.\"Id\" = ?1 AND (?2 IS NULL OR c.\"ServerId\" = ?2) AND (?3 IS NULL OR c.\"Kind\" = ?3)"
        ),
        params![channel, server, kind, viewer],
        |row| {
            Ok(ChannelPlace {
                id: row.get(0)?,
                server: row.get(1)?,
                section: row.get(2)?,
                kind: row.get(3)?,
                name: row.get(4)?,
                slow_mode_seconds: row.get(5)?,
                open: row.get(6)?,
                member: row.get(7)?,
            })
        },
    )
    .optional()
}

/// Whether nobody is denied view anywhere on the server: the base role holds
/// it and no override of a channel or section that exists denies it. Every
/// channel is then open, and every member sees every channel and every
/// section.
///
/// An override can outlive its channel, which is deleted without it; such a
/// row denies nothing and is not counted.
pub fn server_open(db: &Connection, server: Id) -> rusqlite::Result<bool> {
    let denied = |table: &str| {
        format!(
            "EXISTS (SELECT 1 FROM \"{table}\" o WHERE o.\"ServerId\" = s.\"Id\" AND (o.\"DenyMask\" & 16384) != 0
               AND ((o.\"TargetType\" = 0 AND EXISTS (SELECT 1 FROM \"Channels\" c WHERE c.\"Id\" = o.\"TargetId\" AND c.\"ServerId\" = s.\"Id\"))
                 OR (o.\"TargetType\" = 1 AND EXISTS (SELECT 1 FROM \"Sections\" x WHERE x.\"Id\" = o.\"TargetId\" AND x.\"ServerId\" = s.\"Id\"))))"
        )
    };
    let open: Option<bool> = db
        .query_row(
            &format!(
                "SELECT COALESCE((SELECT (r.\"Permissions\" & 16384) != 0 FROM \"ServerRoles\" r
                    WHERE r.\"ServerId\" = s.\"Id\" AND r.\"IsSystem\" = 1 AND r.\"IsOwner\" = 0
                    ORDER BY r.\"Position\" DESC, r.\"Id\" LIMIT 1), 0) = 1
                  AND NOT {} AND NOT {}
                 FROM \"Servers\" s WHERE s.\"Id\" = ?1",
                denied("PermissionOverrides"),
                denied("MemberPermissionOverrides")
            ),
            [server],
            |row| row.get(0),
        )
        .optional()?;
    // A server that is not there is open to nobody.
    Ok(open.unwrap_or(false))
}

/// The channel of the server when `user`, an enabled member, sees it, and
/// none when there is no such channel for them: unknown, of another server,
/// of another kind, or not viewed.
pub fn find(
    db: &Connection,
    user: Id,
    server: Id,
    channel: Id,
    kind: Option<ChannelKind>,
) -> rusqlite::Result<Option<ChannelPlace>> {
    let Some(found) = place(db, channel, Some(server), kind, Some(user))? else {
        return Ok(None);
    };
    Ok(views_place(db, user, &found)?.then_some(found))
}

/// Whether the member, for whom `place` was looked up, sees the channel. An
/// open one needs nothing more than the membership.
fn views_place(db: &Connection, user: Id, found: &ChannelPlace) -> rusqlite::Result<bool> {
    if !found.member {
        return Ok(false);
    }
    if found.open {
        return Ok(true);
    }
    let snapshot = Snapshot::load(
        db,
        found.server,
        Some(&[user]),
        Targets::at(found.section, Some(found.id)),
    )?;
    match snapshot.subject(user) {
        Some(subject) => snapshot.views(subject, found.section, found.id),
        None => Ok(false),
    }
}

/// Whether the member sees the channel, which need not be on any server they
/// are on.
pub fn views(db: &Connection, user: Id, channel: Id) -> rusqlite::Result<bool> {
    match place(db, channel, None, None, Some(user))? {
        Some(found) => views_place(db, user, &found),
        None => Ok(false),
    }
}

/// Whether somebody who has just joined, holding the base role alone, would
/// see the channel: what an invite to it may name to whoever is not a member
/// yet. A channel that is not there is seen by nobody.
pub fn newcomer_views(db: &Connection, channel: Id) -> rusqlite::Result<bool> {
    let Some(found) = place(db, channel, None, None, None)? else {
        return Ok(false);
    };
    if found.open {
        return Ok(true);
    }
    let snapshot = Snapshot::load(
        db,
        found.server,
        Some(&[]),
        Targets::at(found.section, Some(found.id)),
    )?;
    Ok(snapshot.evaluate_newcomer(found.section, Some(found.id))? & VIEW_CHANNEL == VIEW_CHANNEL)
}

/// Whether the section is listed for the member, by the rule of
/// [`ServerLayout::view_of`]: false for one that is not on the server, which
/// is how a section they do not see is answered.
pub fn section_listed(
    db: &Connection,
    user: Id,
    server: Id,
    section: Id,
) -> rusqlite::Result<bool> {
    let exists = db
        .query_row(
            "SELECT 1 FROM \"Sections\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
            params![section, server],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Ok(false);
    }
    if server_open(db, server)? {
        return db
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM \"ServerMembers\" m JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\"
                   WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" = ?2 AND u.\"IsDisabled\" = 0)",
                params![server, user],
                |row| row.get(0),
            );
    }
    let mut statement = db.prepare(
        "SELECT \"Id\" FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"SectionId\" = ?2",
    )?;
    let channels = statement
        .query_map(params![server, section], |row| row.get::<_, Id>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let layout = ServerLayout {
        channels: channels.into_iter().map(|id| (id, Some(section))).collect(),
        sections: vec![section],
    };
    let snapshot = Snapshot::load(db, server, Some(&[user]), layout.targets())?;
    match snapshot.subject(user) {
        Some(subject) => Ok(layout
            .view_of(&snapshot, subject)?
            .sections
            .contains(&section)),
        None => Ok(false),
    }
}

/// Who sees the channel: none when it is open, which is everybody on the
/// server, else the enabled members who view it. An unknown channel has
/// nobody.
pub fn viewers(db: &Connection, server: Id, channel: Id) -> rusqlite::Result<Option<HashSet<Id>>> {
    let Some(found) = place(db, channel, Some(server), None, None)? else {
        return Ok(Some(HashSet::new()));
    };
    if found.open {
        return Ok(None);
    }
    let snapshot = Snapshot::load(db, server, None, Targets::at(found.section, Some(found.id)))?;
    let mut seeing = HashSet::new();
    for subject in snapshot.subjects() {
        if snapshot.views(subject, found.section, found.id)? {
            seeing.insert(subject.user);
        }
    }
    Ok(Some(seeing))
}

/// Those of `users`, members of the server, who see the channel. Only they
/// are evaluated, and an open channel needs nothing. They come in the order
/// they were given.
pub fn who_view(
    db: &Connection,
    server: Id,
    channel: Id,
    users: &[Id],
) -> rusqlite::Result<Vec<Id>> {
    if users.is_empty() {
        return Ok(Vec::new());
    }
    let Some(found) = place(db, channel, Some(server), None, None)? else {
        return Ok(Vec::new());
    };
    if found.open {
        return Ok(users.to_vec());
    }
    let snapshot = Snapshot::load(
        db,
        server,
        Some(users),
        Targets::at(found.section, Some(found.id)),
    )?;
    let mut seeing = Vec::new();
    for user in users {
        if let Some(subject) = snapshot.subject(*user)
            && snapshot.views(subject, found.section, found.id)?
        {
            seeing.push(*user);
        }
    }
    Ok(seeing)
}

/// Who an event about a channel is for: the whole server when nobody is denied
/// view there, with nothing evaluated, and otherwise only the enabled members
/// who view it, on the connections that follow this server. A person's tabs on
/// other servers do not hear it. Run it when the event is sent, so that whoever
/// loses view stops receiving with the next event.
pub fn audience_of(db: &Connection, server: Id, channel: Id) -> rusqlite::Result<Audience> {
    Ok(match viewers(db, server, channel)? {
        None => Audience::Server(server),
        Some(seeing) => {
            let mut users: Vec<Id> = seeing.into_iter().collect();
            users.sort();
            Audience::Followers { server, users }
        }
    })
}

/// The channels one member views and the sections listed for them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberView {
    pub channels: HashSet<Id>,
    pub sections: HashSet<Id>,
}

impl MemberView {
    /// Whether two views show the same channels and sections.
    pub fn same_as(&self, other: &MemberView) -> bool {
        self == other
    }
}

/// What one member sees of the server: none when nobody is denied view
/// anywhere, which is everything, else the channels they view and the
/// sections listed for them.
pub fn of_member(db: &Connection, user: Id, server: Id) -> rusqlite::Result<Option<MemberView>> {
    if server_open(db, server)? {
        return Ok(None);
    }
    let layout = ServerLayout::load(db, server)?;
    let snapshot = Snapshot::load(db, server, Some(&[user]), layout.targets())?;
    Ok(Some(match snapshot.subject(user) {
        Some(subject) => layout.view_of(&snapshot, subject)?,
        None => MemberView::default(),
    }))
}

/// Every channel and section of a server, as the visibility of a whole server
/// needs them.
#[derive(Debug, Clone, Default)]
pub struct ServerLayout {
    pub channels: Vec<(Id, Option<Id>)>,
    pub sections: Vec<Id>,
}

impl ServerLayout {
    pub fn load(db: &Connection, server: Id) -> rusqlite::Result<ServerLayout> {
        let mut statement =
            db.prepare("SELECT \"Id\", \"SectionId\" FROM \"Channels\" WHERE \"ServerId\" = ?1")?;
        let channels = statement
            .query_map([server], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut statement =
            db.prepare("SELECT \"Id\" FROM \"Sections\" WHERE \"ServerId\" = ?1")?;
        let sections = statement
            .query_map([server], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ServerLayout { channels, sections })
    }

    /// The sections channels point at are read as well as those that exist,
    /// so that both decide alike.
    pub fn targets(&self) -> Targets {
        let mut sections = Vec::new();
        let mut seen = HashSet::new();
        for id in self
            .sections
            .iter()
            .copied()
            .chain(self.channels.iter().filter_map(|(_, section)| *section))
        {
            if seen.insert(id) {
                sections.push(id);
            }
        }
        Targets {
            sections,
            channels: self.channels.iter().map(|(id, _)| *id).collect(),
        }
    }

    pub fn everything(&self) -> MemberView {
        MemberView {
            channels: self.channels.iter().map(|(id, _)| *id).collect(),
            sections: self.sections.iter().copied().collect(),
        }
    }

    /// The channels the member views, and the sections listed for them: one
    /// with a channel they view, one where they hold ManageChannels, and an
    /// empty one where they hold ViewChannel, as every member does on a server
    /// where nobody is denied it, so that an empty section stays where
    /// everybody sees it today.
    pub fn view_of(&self, snapshot: &Snapshot, subject: &Subject) -> rusqlite::Result<MemberView> {
        let mut viewed = HashSet::new();
        let mut sections_with_viewed = HashSet::new();
        for (id, section) in &self.channels {
            if snapshot.views(subject, *section, *id)? {
                viewed.insert(*id);
                if let Some(section) = section {
                    sections_with_viewed.insert(*section);
                }
            }
        }
        let sections_with_any: HashSet<Id> = self
            .channels
            .iter()
            .filter_map(|(_, section)| *section)
            .collect();
        let mut sections = HashSet::new();
        for section in &self.sections {
            if sections_with_viewed.contains(section) {
                sections.insert(*section);
                continue;
            }
            let held = snapshot.evaluate(subject, Some(*section), None)?;
            if held & MANAGE_CHANNELS == MANAGE_CHANNELS
                || (!sections_with_any.contains(section) && held & VIEW_CHANNEL == VIEW_CHANNEL)
            {
                sections.insert(*section);
            }
        }
        Ok(MemberView {
            channels: viewed,
            sections,
        })
    }
}

/// What every member of one server sees, for the events that are about the
/// whole layout and for telling people whose view a change altered.
#[derive(Debug, Clone)]
pub struct ServerVisibility {
    pub server: Id,
    layout: Option<ServerLayout>,
    members: Option<std::collections::HashMap<Id, MemberView>>,
}

impl ServerVisibility {
    /// Nobody is denied view anywhere: everybody sees everything, and nothing
    /// was evaluated.
    pub fn is_open(&self) -> bool {
        self.members.is_none()
    }

    /// The members who were evaluated, with what each sees; none when open.
    pub fn members(&self) -> impl Iterator<Item = (&Id, &MemberView)> {
        self.members.iter().flatten()
    }

    pub fn view_of(&self, user: Id) -> Option<&MemberView> {
        self.members.as_ref().and_then(|members| members.get(&user))
    }

    pub fn load(db: &Connection, server: Id) -> rusqlite::Result<ServerVisibility> {
        if server_open(db, server)? {
            return Ok(ServerVisibility {
                server,
                layout: None,
                members: None,
            });
        }
        let layout = ServerLayout::load(db, server)?;
        let snapshot = Snapshot::load(db, server, None, layout.targets())?;
        let mut members = std::collections::HashMap::new();
        for subject in snapshot.subjects() {
            members.insert(subject.user, layout.view_of(&snapshot, subject)?);
        }
        Ok(ServerVisibility {
            server,
            layout: Some(layout),
            members: Some(members),
        })
    }

    /// The members whose view differs between `before` and `after`, the same
    /// server read on both sides of one change. Two open readings differ for
    /// nobody. An open side shows its members everything there is now, which
    /// for a change of roles, overrides or a channel's section is everything
    /// there was before too.
    pub fn changed(
        db: &Connection,
        before: &ServerVisibility,
        after: &ServerVisibility,
    ) -> rusqlite::Result<Vec<Id>> {
        if before.is_open() && after.is_open() {
            return Ok(Vec::new());
        }
        let everything = match before.layout.as_ref().or(after.layout.as_ref()) {
            Some(layout) => layout.everything(),
            None => ServerLayout::load(db, before.server)?.everything(),
        };
        let enabled: Vec<Id> = if before.is_open() || after.is_open() {
            let mut statement = db.prepare(
                "SELECT m.\"UserId\" FROM \"ServerMembers\" m JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\"
                 WHERE m.\"ServerId\" = ?1 AND u.\"IsDisabled\" = 0",
            )?;
            statement
                .query_map([before.server], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        } else {
            Vec::new()
        };
        let nothing = MemberView::default();
        let view_of = |reading: &ServerVisibility, user: Id| -> MemberView {
            if reading.is_open() {
                if enabled.contains(&user) {
                    everything.clone()
                } else {
                    nothing.clone()
                }
            } else {
                reading.view_of(user).cloned().unwrap_or_default()
            }
        };
        let mut users: Vec<Id> = enabled
            .iter()
            .copied()
            .chain(before.members().map(|(user, _)| *user))
            .chain(after.members().map(|(user, _)| *user))
            .collect();
        users.sort();
        users.dedup();
        Ok(users
            .into_iter()
            .filter(|user| !view_of(before, *user).same_as(&view_of(after, *user)))
            .collect())
    }
}
