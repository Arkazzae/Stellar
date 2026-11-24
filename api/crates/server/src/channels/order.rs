//! Putting channels and sections in order.
//!
//! The order names every channel (or section) the caller sees, and no other:
//! one they do not see keeps its place among the rest, as it would have to for
//! a caller who could not name it.

use std::collections::{HashMap, HashSet};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, params};
use serde::Deserialize;
use stellarr_messages::MessageKey;

use super::layout::{self, ViewChange};
use super::{Answer, say, Refuse};
use crate::auth::{Caller, fresh};
use crate::database::DbError;
use crate::http::{ApiResult, Json};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization::{Scope, allows, move_channel};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_CHANNELS;
use crate::permissions::visibility::{self, MemberView, ServerVisibility};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderChannels {
    channel_ids: Vec<Id>,
    r#move: Option<MoveChannel>,
}

/// Files one channel under a section in the same change; no section takes it
/// out of the one it is in. A channel left out names none, which nobody sees.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveChannel {
    #[serde(default = "nobody")]
    channel_id: Id,
    section_id: Option<Id>,
}

fn nobody() -> Id {
    Id::from_bytes([0; 16])
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderSections {
    section_ids: Vec<Id>,
}

/// The new order of every place: `current`, all of them in their order now,
/// with each place one of the caller's held taking the next of theirs, in the
/// order they want. Every other place stays where it is.
pub fn merged(current: &[Id], wanted: &[Id]) -> Vec<Id> {
    let theirs: HashSet<Id> = wanted.iter().copied().collect();
    let mut next = 0;
    current
        .iter()
        .map(|id| {
            if theirs.contains(id) {
                next += 1;
                wanted[next - 1]
            } else {
                *id
            }
        })
        .collect()
}

/// Whether `wanted` names each of `seen` exactly once and nothing else.
fn names_exactly(wanted: &[Id], seen: &[Id]) -> bool {
    let named: HashSet<Id> = wanted.iter().copied().collect();
    named.len() == wanted.len() && named == seen.iter().copied().collect::<HashSet<Id>>()
}

struct Place {
    id: Id,
    position: i64,
    section: Option<Id>,
}

/// What a reordering of channels leaves to announce.
struct Reordered {
    /// Every channel in its new order with its section.
    ordered: Vec<(Id, Option<Id>)>,
    /// What everybody saw before, when a channel was moved.
    view_change: Option<ViewChange>,
}

pub async fn reorder_channels(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(request): Json<ReorderChannels>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let user = caller.user;
    let moved = request.r#move.is_some();
    let outcome = state
        .database()
        .write(move |tx| -> Result<Answer<Reordered>, DbError> {
            if !allows(tx, user, &Scope::server(server), MANAGE_CHANNELS)? {
                return Ok(Answer::Refused(Refuse::Forbidden));
            }
            let mut channels = channels_of(tx, server)?;
            let view = visibility::of_member(tx, user, server)?;
            let seen: Vec<Id> = channels
                .iter()
                .map(|channel| channel.id)
                .filter(|id| view.as_ref().is_none_or(|view| view.channels.contains(id)))
                .collect();
            if !names_exactly(&request.channel_ids, &seen) {
                return Ok(Answer::Refused(Refuse::Invalid(
                    "channelIds",
                    MessageKey::ChannelOrderIncomplete,
                )));
            }

            // Read before anything changes, to tell afterwards whom a move
            // showed a channel to or hid one from.
            let view_change = match request.r#move {
                Some(_) => Some(ViewChange::begin(tx, server)?),
                None => None,
            };
            let mut moved_section = false;
            if let Some(wanted) = &request.r#move {
                // The order names every channel the caller sees, so a moved
                // channel missing from it is one they do not, or another
                // server's.
                let Some(index) = channels
                    .iter()
                    .position(|channel| channel.id == wanted.channel_id && seen.contains(&channel.id))
                else {
                    return Ok(Answer::Refused(Refuse::NotFound));
                };
                if let Some(section) = wanted.section_id {
                    let listed = match &view {
                        None => section_exists(tx, server, section)?,
                        Some(view) => view.sections.contains(&section),
                    };
                    if !listed {
                        return Ok(Answer::Refused(Refuse::NotFound));
                    }
                }
                // The same rules as assigning a section on its own: a
                // section can change who may see or edit the channel.
                let decision = move_channel(tx, user, server, wanted.channel_id, wanted.section_id)?;
                if !decision.allowed {
                    return Ok(Answer::Refused(Refuse::Forbidden));
                }
                // Moving a channel that changes what somebody may do, on the
                // instance's own server, needs a second step made a moment ago.
                if decision.changes_permissions
                    && !fresh::has_fresh_second_step_for_server(
                        tx,
                        user,
                        Some(caller.session),
                        server,
                        Timestamp::now(),
                    )?
                {
                    return Ok(Answer::Refused(Refuse::SecondStep));
                }

                if channels[index].section != wanted.section_id {
                    let names = section_names(tx, server)?;
                    let name_of = |id: Option<Id>| id.and_then(|id| names.get(&id).cloned());
                    let to = name_of(wanted.section_id);
                    let moved_name: String = tx.query_row(
                        "SELECT \"Name\" FROM \"Channels\" WHERE \"Id\" = ?1",
                        [wanted.channel_id],
                        |row| row.get(0),
                    )?;
                    audit::record(
                        tx,
                        server,
                        user,
                        &AuditEvent {
                            action: "channel.move".to_string(),
                            sentence: match &to {
                                None => say(&messages, MessageKey::AuditChannelMovedOut, &[("name", &moved_name)])?,
                                Some(section) => say(&messages, 
                                    MessageKey::AuditChannelMoved,
                                    &[("name", &moved_name), ("section", section)],
                                )?,
                            },
                            target_type: Some("channel".to_string()),
                            target_id: Some(wanted.channel_id),
                            target_name: Some(moved_name),
                            changes: vec![AuditChange {
                                field: "section".to_string(),
                                from: name_of(channels[index].section),
                                to,
                                ..AuditChange::default()
                            }],
                        },
                    )?;
                    moved_section = true;
                }
                channels[index].section = wanted.section_id;
                tx.execute(
                    "UPDATE \"Channels\" SET \"SectionId\" = ?2 WHERE \"Id\" = ?1",
                    params![wanted.channel_id, wanted.section_id],
                )?;
            }

            channels.sort_by_key(|channel| (channel.position, channel.id));
            let current: Vec<Id> = channels.iter().map(|channel| channel.id).collect();
            let new_order = merged(&current, &request.channel_ids);
            let by_id: HashMap<Id, &Place> =
                channels.iter().map(|channel| (channel.id, channel)).collect();
            let mut reordered = false;
            for (position, id) in new_order.iter().enumerate() {
                let position = position as i64;
                if by_id[id].position != position {
                    reordered = true;
                    tx.execute(
                        "UPDATE \"Channels\" SET \"Position\" = ?2 WHERE \"Id\" = ?1",
                        params![id, position],
                    )?;
                }
            }
            if reordered && !moved_section {
                audit::record(
                    tx,
                    server,
                    user,
                    &AuditEvent {
                        action: "channel.reorder".to_string(),
                        sentence: say(&messages, MessageKey::AuditChannelsReordered, &[])?,
                        ..AuditEvent::default()
                    },
                )?;
            }
            Ok(Answer::Done(Reordered {
                ordered: new_order
                    .iter()
                    .map(|id| (*id, by_id[id].section))
                    .collect(),
                view_change,
            }))
        })
        .await?;
    let reordered = match outcome {
        Answer::Refused(refusal) => return Err(refusal.into_error(&messages)?),
        Answer::Done(reordered) => reordered,
    };

    // Committed: the people told must not depend on the caller staying. What
    // everybody sees is read once more, now, and both events follow it.
    let events = state
        .database()
        .read(move |db| -> Result<Vec<crate::events::Event>, DbError> {
            let mut events = Vec::new();
            let visibility = match reordered.view_change {
                Some(change) => {
                    let (after, access) = change.finish(db)?;
                    events.extend(access);
                    after
                }
                None => ServerVisibility::load(db, server)?,
            };
            events.extend(layout::channel_layout(&visibility, &reordered.ordered));
            Ok(events)
        })
        .await?;
    for event in events {
        state.events().publish(event);
    }
    // A channel moved to another section may have gone private (or open), and
    // whoever sits in it where they may not is taken out.
    if moved {
        crate::voice::enforce(&state, server).await;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn reorder_sections(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(request): Json<ReorderSections>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let user = caller.user;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Answer<Vec<Id>>, DbError> {
            if !allows(tx, user, &Scope::server(server), MANAGE_CHANNELS)? {
                return Ok(Answer::Refused(Refuse::Forbidden));
            }
            let mut sections = sections_of(tx, server)?;
            let view: Option<MemberView> = visibility::of_member(tx, user, server)?;
            let listed: Vec<Id> = sections
                .iter()
                .map(|section| section.id)
                .filter(|id| view.as_ref().is_none_or(|view| view.sections.contains(id)))
                .collect();
            if !names_exactly(&request.section_ids, &listed) {
                return Ok(Answer::Refused(Refuse::Invalid(
                    "sectionIds",
                    MessageKey::SectionOrderIncomplete,
                )));
            }
            sections.sort_by_key(|section| (section.position, section.id));
            let current: Vec<Id> = sections.iter().map(|section| section.id).collect();
            let new_order = merged(&current, &request.section_ids);
            let by_id: HashMap<Id, &Place> =
                sections.iter().map(|section| (section.id, section)).collect();
            let mut reordered = false;
            for (position, id) in new_order.iter().enumerate() {
                let position = position as i64;
                if by_id[id].position != position {
                    reordered = true;
                    tx.execute(
                        "UPDATE \"Sections\" SET \"Position\" = ?2 WHERE \"Id\" = ?1",
                        params![id, position],
                    )?;
                }
            }
            if reordered {
                audit::record(
                    tx,
                    server,
                    user,
                    &AuditEvent {
                        action: "section.reorder".to_string(),
                        sentence: say(&messages, MessageKey::AuditSectionsReordered, &[])?,
                        ..AuditEvent::default()
                    },
                )?;
            }
            Ok(Answer::Done(new_order))
        })
        .await?;
    let ordered = match outcome {
        Answer::Refused(refusal) => return Err(refusal.into_error(&messages)?),
        Answer::Done(ordered) => ordered,
    };
    let events = state
        .database()
        .read(move |db| -> Result<Vec<crate::events::Event>, DbError> {
            Ok(layout::section_order(
                &ServerVisibility::load(db, server)?,
                &ordered,
            ))
        })
        .await?;
    for event in events {
        state.events().publish(event);
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn channels_of(db: &Connection, server: Id) -> rusqlite::Result<Vec<Place>> {
    let mut statement = db
        .prepare("SELECT \"Id\", \"Position\", \"SectionId\" FROM \"Channels\" WHERE \"ServerId\" = ?1")?;
    statement
        .query_map([server], |row| {
            Ok(Place {
                id: row.get(0)?,
                position: row.get(1)?,
                section: row.get(2)?,
            })
        })?
        .collect()
}

fn sections_of(db: &Connection, server: Id) -> rusqlite::Result<Vec<Place>> {
    let mut statement =
        db.prepare("SELECT \"Id\", \"Position\" FROM \"Sections\" WHERE \"ServerId\" = ?1")?;
    statement
        .query_map([server], |row| {
            Ok(Place {
                id: row.get(0)?,
                position: row.get(1)?,
                section: None,
            })
        })?
        .collect()
}

fn section_exists(db: &Connection, server: Id, section: Id) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"Sections\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2)",
        params![section, server],
        |row| row.get(0),
    )
}

fn section_names(db: &Connection, server: Id) -> rusqlite::Result<HashMap<Id, String>> {
    let mut statement =
        db.prepare("SELECT \"Id\", \"Name\" FROM \"Sections\" WHERE \"ServerId\" = ?1")?;
    statement
        .query_map([server], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(numbers: &[u8]) -> Vec<Id> {
        numbers
            .iter()
            .map(|n| Id::from_bytes([*n; 16]))
            .collect()
    }

    #[test]
    fn the_places_of_the_caller_are_filled_in_their_order_and_the_rest_stay() {
        // The caller sees 1, 3 and 5 and wants 5, 1, 3; 2 and 4 stay put.
        let current = ids(&[1, 2, 3, 4, 5]);
        let wanted = ids(&[5, 1, 3]);
        assert_eq!(merged(&current, &wanted), ids(&[5, 2, 1, 4, 3]));
    }

    #[test]
    fn an_order_has_to_name_each_place_once() {
        let seen = ids(&[1, 2]);
        assert!(names_exactly(&ids(&[2, 1]), &seen));
        assert!(!names_exactly(&ids(&[1, 1]), &seen));
        assert!(!names_exactly(&ids(&[1]), &seen));
        assert!(!names_exactly(&ids(&[1, 2, 3]), &seen));
    }
}
