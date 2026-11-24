//! `GET /api/social`: everything the friends screen shows.

use std::collections::{HashMap, HashSet};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, params};
use serde::Serialize;

use super::conversations::{self, Encryption, DirectMessage};
use super::other;
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiResult, json_response};
use crate::instance::users::{User, UserDto};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

#[derive(Serialize)]
struct SocialState {
    friends: Vec<Friend>,
    requests: Vec<Request>,
    blocked: Vec<UserDto>,
    conversations: Vec<Conversation>,
}

#[derive(Serialize)]
struct Friend {
    user: UserDto,
    since: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    id: String,
    user: UserDto,
    direction: &'static str,
    sent_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Conversation {
    id: String,
    peer: UserDto,
    last_message: Option<DirectMessage>,
    unread: i64,
    updated_at: String,
    encryption: Encryption,
}

fn pairs(
    connection: &Connection,
    sql: &str,
    me: Id,
) -> rusqlite::Result<Vec<(Id, Id, Id, Timestamp)>> {
    let mut statement = connection.prepare(sql)?;
    statement
        .query_map(params![me], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect()
}

/// Presence is each person's as the viewer may see it.
fn read_state(
    connection: &Connection,
    state: &AppState,
    me: Id,
) -> Result<SocialState, DbError> {
    // Each list is in the order things happened, the oldest first, and by id
    // for two that happened at the same moment.
    let friendships = pairs(
        connection,
        "SELECT \"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\" FROM \"Friendships\"
         WHERE \"UserAId\" = ?1 OR \"UserBId\" = ?1
         ORDER BY \"CreatedAt\", \"Id\"",
        me,
    )?;
    let requests = pairs(
        connection,
        "SELECT \"Id\", \"SenderId\", \"ReceiverId\", \"CreatedAt\" FROM \"FriendRequests\"
         WHERE \"SenderId\" = ?1 OR \"ReceiverId\" = ?1
         ORDER BY \"CreatedAt\", \"Id\"",
        me,
    )?;
    let blocks = pairs(
        connection,
        "SELECT \"BlockerId\", \"BlockerId\", \"BlockedId\", \"CreatedAt\" FROM \"UserBlocks\"
         WHERE \"BlockerId\" = ?1 OR \"BlockedId\" = ?1
         ORDER BY \"CreatedAt\", \"BlockerId\", \"BlockedId\"",
        me,
    )?;
    let blocked_by_viewer: Vec<Id> = blocks
        .iter()
        .filter(|(_, blocker, _, _)| *blocker == me)
        .map(|(_, _, blocked, _)| *blocked)
        .collect();
    let peers_in_block: HashSet<Id> = blocks
        .iter()
        .map(|(_, blocker, blocked, _)| other(*blocker, *blocked, me))
        .collect();
    let conversations: Vec<conversations::Conversation> = conversations::of_person(connection, me)?
        .into_iter()
        .filter(|conversation| !peers_in_block.contains(&conversation.peer(me)))
        .collect();

    let mut people: HashMap<Id, User> = HashMap::new();
    let wanted = friendships
        .iter()
        .map(|(_, a, b, _)| other(*a, *b, me))
        .chain(requests.iter().map(|(_, s, r, _)| other(*s, *r, me)))
        .chain(blocked_by_viewer.iter().copied())
        .chain(conversations.iter().map(|c| c.peer(me)));
    for id in wanted {
        if let std::collections::hash_map::Entry::Vacant(slot) = people.entry(id)
            && let Some(user) = User::find(connection, id)?
        {
            slot.insert(user);
        }
    }
    let person = |id: Id| -> Option<UserDto> {
        people
            .get(&id)
            .map(|user| user.dto(&state.presence().public_presence_of(id, Some(me))))
    };

    let friends = friendships
        .iter()
        .filter_map(|(_, a, b, since)| {
            Some(Friend {
                user: person(other(*a, *b, me))?,
                since: since.to_wire_text(),
            })
        })
        .collect();
    let requests = requests
        .iter()
        .filter_map(|(id, sender, receiver, sent)| {
            Some(Request {
                id: id.to_string(),
                user: person(other(*sender, *receiver, me))?,
                direction: if *receiver == me { "incoming" } else { "outgoing" },
                sent_at: sent.to_wire_text(),
            })
        })
        .collect();
    let blocked = blocked_by_viewer.iter().filter_map(|id| person(*id)).collect();

    let mut listed: Vec<(Timestamp, Conversation)> = Vec::new();
    for conversation in &conversations {
        let Some(peer) = person(conversation.peer(me)) else {
            continue;
        };
        let last = conversations::last_message(connection, conversation.id)?;
        let updated = last
            .as_ref()
            .map_or(conversation.created_at, |(_, at)| *at);
        listed.push((
            updated,
            Conversation {
                id: conversation.id.to_string(),
                peer,
                last_message: last.map(|(message, _)| message),
                unread: conversations::unread(connection, me, conversation)?,
                updated_at: updated.to_wire_text(),
                encryption: conversation.encryption(),
            },
        ));
    }
    // Newest first; the sort keeps the order of rows that tie.
    listed.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    Ok(SocialState {
        friends,
        requests,
        blocked,
        conversations: listed.into_iter().map(|(_, c)| c).collect(),
    })
}

pub async fn read(State(state): State<AppState>, caller: Caller) -> ApiResult<Response> {
    let shared = state.clone();
    let body = state
        .database()
        .read(move |connection| read_state(connection, &shared, caller.user))
        .await?;
    Ok(json_response(StatusCode::OK, &body))
}
