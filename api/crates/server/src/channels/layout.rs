//! The events about how a server's channels are laid out, which name every
//! channel or section they are about, and the one that tells somebody what
//! they see has changed.
//!
//! On a server where nobody is denied view anywhere, a layout event goes to
//! the whole server. Otherwise each member is sent the layout of what they
//! see, from one reading of what everybody sees taken when the event is sent;
//! members who see the same are sent one event between them.
//!
//! A layout only orders and files the channels a client already has, and a
//! client cannot make a channel out of an id. So when a change shows somebody
//! a channel or hides one from them, they are sent `ChannelAccessChanged`
//! instead, which carries nothing but the server, and their client reads its
//! own channels again.

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;

use crate::events::{Audience, Event};
use crate::permissions::visibility::{MemberView, ServerVisibility};
use crate::values::Id;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChannelLayout {
    server_id: Id,
    channels: Vec<ChannelPlacement>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChannelPlacement {
    id: Id,
    section_id: Option<Id>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SectionOrder {
    server_id: Id,
    section_ids: Vec<Id>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccessChanged {
    server_id: Id,
}

/// The people of a server who see the same, each group with what they see.
/// Members who see nothing of the kind are left out.
fn alike<'a>(
    visibility: &'a ServerVisibility,
    seen: impl Fn(&'a MemberView) -> &'a std::collections::HashSet<Id>,
) -> Vec<(&'a std::collections::HashSet<Id>, Vec<Id>)> {
    let mut groups: BTreeMap<Vec<Id>, (&std::collections::HashSet<Id>, Vec<Id>)> = BTreeMap::new();
    for (user, view) in visibility.members() {
        let ids = seen(view);
        if ids.is_empty() {
            continue;
        }
        let mut key: Vec<Id> = ids.iter().copied().collect();
        key.sort();
        groups.entry(key).or_insert((ids, Vec::new())).1.push(*user);
    }
    groups
        .into_values()
        .map(|(ids, mut users)| {
            users.sort();
            (ids, users)
        })
        .collect()
}

/// `ChannelLayoutChanged`: the channels in their order, each with its section.
/// `ordered` is every channel of the server in the order it now has.
pub fn channel_layout(
    visibility: &ServerVisibility,
    ordered: &[(Id, Option<Id>)],
) -> Vec<Event> {
    let server = visibility.server;
    let layout = |seen: &dyn Fn(Id) -> bool| ChannelLayout {
        server_id: server,
        channels: ordered
            .iter()
            .filter(|(id, _)| seen(*id))
            .map(|(id, section)| ChannelPlacement {
                id: *id,
                section_id: *section,
            })
            .collect(),
    };
    if visibility.is_open() {
        return vec![Event::new(
            Audience::Server(server),
            "ChannelLayoutChanged",
            &layout(&|_| true),
        )];
    }
    alike(visibility, |view| &view.channels)
        .into_iter()
        .map(|(seen, users)| {
            Event::new(
                Audience::Followers { server, users },
                "ChannelLayoutChanged",
                &layout(&|id| seen.contains(&id)),
            )
        })
        .collect()
}

/// `SectionOrderChanged`: the sections in their order, each listed for the
/// people it is listed for.
pub fn section_order(visibility: &ServerVisibility, ordered: &[Id]) -> Vec<Event> {
    let server = visibility.server;
    let order = |seen: &dyn Fn(Id) -> bool| SectionOrder {
        server_id: server,
        section_ids: ordered.iter().copied().filter(|id| seen(*id)).collect(),
    };
    if visibility.is_open() {
        return vec![Event::new(
            Audience::Server(server),
            "SectionOrderChanged",
            &order(&|_| true),
        )];
    }
    alike(visibility, |view| &view.sections)
        .into_iter()
        .map(|(seen, users)| {
            Event::new(
                Audience::Followers { server, users },
                "SectionOrderChanged",
                &order(&|id| seen.contains(&id)),
            )
        })
        .collect()
}

/// `ChannelAccessChanged` for each of `users`: what they see has changed.
pub fn access_changed(server: Id, users: Vec<Id>) -> Option<Event> {
    (!users.is_empty()).then(|| {
        Event::new(
            Audience::Followers { server, users },
            "ChannelAccessChanged",
            &AccessChanged { server_id: server },
        )
    })
}

/// One change that may show somebody a channel or hide one from them: what
/// everybody sees is read before it, and again once it is committed, and
/// whoever sees something else is told. On a server where nobody is denied
/// view before or after, nobody is told anything.
pub struct ViewChange {
    before: ServerVisibility,
}

impl ViewChange {
    pub fn begin(db: &Connection, server: Id) -> rusqlite::Result<ViewChange> {
        Ok(ViewChange {
            before: ServerVisibility::load(db, server)?,
        })
    }

    /// What everybody sees now, and the event for those who see something
    /// else than before. Call it once the change is committed.
    pub fn finish(
        self,
        db: &Connection,
    ) -> rusqlite::Result<(ServerVisibility, Option<Event>)> {
        let after = ServerVisibility::load(db, self.before.server)?;
        let changed = ServerVisibility::changed(db, &self.before, &after)?;
        Ok((after, access_changed(self.before.server, changed)))
    }
}
