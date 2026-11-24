//! Friend requests, friendships and blocks between people, and the state the
//! friends screen loads.
//!
//! They belong to the instance, not to a server, so no route here names one.
//! Direct messages hang off the same relationships and are another group's;
//! what this group reads of them is only what the state lists
//! ([`conversations`]).
//!
//! A change here never touches the hub's groups. Who is shown whose presence
//! is worked out from these tables each time a status changes
//! ([`crate::realtime::audience`]), so a friendship made or ended and a block
//! set or lifted count from the next status on, for connections that are
//! already open too. What the two people are told is [`announce`]: a reload.

pub mod conversations;
mod relations;
mod requests;
mod state;

use rusqlite::{Connection, params};
use serde::{Deserialize, Deserializer, Serialize};

use crate::events::{Audience, Event};
use crate::http::Endpoints;
use crate::permissions::membership::parse_guid;
use crate::state::AppState;
use crate::values::Id;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/social", state::read)
        .authorized()
        .post("/api/social/requests", requests::create)
        .json_body()
        .authorized()
        .post("/api/social/requests/{requestId}/accept", requests::accept)
        .authorized()
        .delete("/api/social/requests/{requestId}", requests::delete)
        .authorized()
        .delete("/api/social/friends/{userId}", relations::remove_friend)
        .authorized()
        .post("/api/social/blocks/{userId}", relations::block)
        .authorized()
        .delete("/api/social/blocks/{userId}", relations::unblock)
        .authorized()
}

/// An id in a path, in every spelling the first server's route accepted:
/// hyphenated or not, in braces or parentheses, in either case.
#[derive(Debug, Clone, Copy)]
pub struct GuidSegment(pub Id);

impl<'de> Deserialize<'de> for GuidSegment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<GuidSegment, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_guid(&text)
            .map(GuidSegment)
            .ok_or_else(|| serde::de::Error::custom("not an id"))
    }
}

/// The two ids in the order friendships and conversations are stored in.
pub fn pair(first: Id, second: Id) -> (Id, Id) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}

/// One value for a pair of people whichever of them sent the request: the two
/// ids in stored order, without hyphens, joined by a colon.
pub fn pair_key(first: Id, second: Id) -> String {
    let (a, b) = pair(first, second);
    format!(
        "{}:{}",
        a.to_string().replace('-', ""),
        b.to_string().replace('-', "")
    )
}

/// The other one of two people, seen from `me`.
pub fn other(first: Id, second: Id, me: Id) -> Id {
    if first == me { second } else { first }
}

/// Whether either of the two has blocked the other.
pub fn is_blocked(connection: &Connection, first: Id, second: Id) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"UserBlocks\"
                        WHERE (\"BlockerId\" = ?1 AND \"BlockedId\" = ?2)
                           OR (\"BlockerId\" = ?2 AND \"BlockedId\" = ?1))",
        params![first, second],
        |row| row.get(0),
    )
}

/// Whether the two are friends.
pub fn are_friends(connection: &Connection, first: Id, second: Id) -> rusqlite::Result<bool> {
    let (a, b) = pair(first, second);
    connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"Friendships\" WHERE \"UserAId\" = ?1 AND \"UserBId\" = ?2)",
        params![a, b],
        |row| row.get(0),
    )
}

#[derive(Serialize)]
struct Nothing {}

/// Tells the people's clients to reload the friends screen. Called after the
/// change is committed, by this group and by whoever changes a conversation.
pub fn announce(state: &AppState, people: &[Id]) {
    let mut audience: Vec<Id> = Vec::with_capacity(people.len());
    for person in people {
        if !audience.contains(person) {
            audience.push(*person);
        }
    }
    state.events().publish(Event::new(
        Audience::Users(audience),
        "SocialChanged",
        &Nothing {},
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_is_stored_in_the_order_of_the_ids_as_written() {
        // The first of these has its top bit set, which a signed comparison of
        // the first field would put before the other.
        let high = Id::parse("84a0daa8-179c-42c4-81b6-6980d37f2fdc").unwrap();
        let low = Id::parse("0360bf01-b87b-4fe3-ba0d-f2417ac364ab").unwrap();

        assert_eq!(pair(high, low), (low, high));
        assert_eq!(pair(low, high), (low, high));
        assert_eq!(
            pair_key(high, low),
            "0360bf01b87b4fe3ba0df2417ac364ab:84a0daa8179c42c481b66980d37f2fdc"
        );
    }

    #[test]
    fn a_path_segment_is_an_id_in_any_spelling_the_first_server_took() {
        let want = Id::parse("0360bf01-b87b-4fe3-ba0d-f2417ac364ab").unwrap();
        for text in [
            "0360bf01-b87b-4fe3-ba0d-f2417ac364ab",
            "0360BF01B87B4FE3BA0DF2417AC364AB",
            "{0360bf01-b87b-4fe3-ba0d-f2417ac364ab}",
            "(0360bf01-b87b-4fe3-ba0d-f2417ac364ab)",
        ] {
            let segment: GuidSegment = serde_json::from_value(text.into()).unwrap();
            assert_eq!(segment.0, want, "{text}");
        }
        assert!(serde_json::from_value::<GuidSegment>("nobody".into()).is_err());
    }
}
