//! What a poll looks like as JSON, read for any number of messages at once.

use std::collections::HashMap;

use rusqlite::{Connection, params};
use serde::Serialize;

use super::ended_at;
use crate::values::{Id, Timestamp, json_ids};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollDto {
    pub question: String,
    pub answers: Vec<PollAnswerDto>,
    pub multiple: bool,
    pub ends_at: Timestamp,
    /// When it ended, on time or early; null while it runs.
    pub ended_at: Option<Timestamp>,
    /// Who ended it early; null when it ended on time or runs.
    pub ended_by: Option<Id>,
    /// How many people voted, whatever number of answers each chose.
    pub voters: i64,
    /// The answers of the person the poll was read for; null when it was read
    /// for many, whose own choices it cannot say.
    pub mine: Option<Vec<i64>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollAnswerDto {
    pub id: i64,
    pub text: String,
    pub emoji: Option<String>,
    pub votes: i64,
}

/// The polls of these messages, read for `viewer`, by message. Three
/// statements however many polls there are: the polls with how many voted,
/// the answers with how many chose each, and the viewer's own votes. The
/// counts are worked out from the votes through their key.
pub fn polls_of(
    db: &Connection,
    ids: &[Id],
    viewer: Option<Id>,
) -> rusqlite::Result<HashMap<Id, PollDto>> {
    let mut polls: HashMap<Id, PollDto> = HashMap::new();
    if ids.is_empty() {
        return Ok(polls);
    }
    let wanted = json_ids(ids);
    let now = Timestamp::now();
    {
        let mut statement = db.prepare_cached(
            "SELECT p.\"MessageId\", p.\"Question\", p.\"Multiple\", p.\"EndsAt\", p.\"EndedAt\", p.\"EndedById\",
                    (SELECT COUNT(DISTINCT v.\"UserId\") FROM \"PollVotes\" v WHERE v.\"MessageId\" = p.\"MessageId\")
             FROM \"Polls\" p WHERE p.\"MessageId\" IN (SELECT value FROM json_each(?1))",
        )?;
        let rows = statement.query_map([&wanted], |row| {
            let ends_at: Timestamp = row.get(3)?;
            let ended: Option<Timestamp> = row.get(4)?;
            Ok((
                row.get::<_, Id>(0)?,
                PollDto {
                    question: row.get(1)?,
                    answers: Vec::new(),
                    multiple: row.get(2)?,
                    ends_at,
                    ended_at: ended_at(ends_at, ended, now),
                    ended_by: row.get(5)?,
                    voters: row.get(6)?,
                    mine: viewer.map(|_| Vec::new()),
                },
            ))
        })?;
        for row in rows {
            let (message, poll) = row?;
            polls.insert(message, poll);
        }
    }
    if polls.is_empty() {
        return Ok(polls);
    }
    {
        let mut statement = db.prepare_cached(
            "SELECT a.\"MessageId\", a.\"AnswerId\", a.\"Text\", a.\"Emoji\",
                    (SELECT COUNT(*) FROM \"PollVotes\" v WHERE v.\"MessageId\" = a.\"MessageId\" AND v.\"AnswerId\" = a.\"AnswerId\")
             FROM \"PollAnswers\" a WHERE a.\"MessageId\" IN (SELECT value FROM json_each(?1))
             ORDER BY a.\"MessageId\", a.\"AnswerId\"",
        )?;
        let rows = statement.query_map([&wanted], |row| {
            Ok((
                row.get::<_, Id>(0)?,
                PollAnswerDto {
                    id: row.get(1)?,
                    text: row.get(2)?,
                    emoji: row.get(3)?,
                    votes: row.get(4)?,
                },
            ))
        })?;
        for row in rows {
            let (message, answer) = row?;
            if let Some(poll) = polls.get_mut(&message) {
                poll.answers.push(answer);
            }
        }
    }
    if let Some(viewer) = viewer {
        let mut statement = db.prepare_cached(
            "SELECT \"MessageId\", \"AnswerId\" FROM \"PollVotes\"
             WHERE \"MessageId\" IN (SELECT value FROM json_each(?1)) AND \"UserId\" = ?2
             ORDER BY \"MessageId\", \"AnswerId\"",
        )?;
        let rows = statement.query_map(params![wanted, viewer], |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (message, answer) = row?;
            if let Some(mine) = polls.get_mut(&message).and_then(|poll| poll.mine.as_mut()) {
                mine.push(answer);
            }
        }
    }
    Ok(polls)
}

/// The poll of one message, read for `viewer`.
pub fn one(db: &Connection, message: Id, viewer: Option<Id>) -> rusqlite::Result<Option<PollDto>> {
    Ok(polls_of(db, &[message], viewer)?.remove(&message))
}

/// The questions of the polls of these messages, which a quote of a message
/// with no text of its own shows.
pub fn questions_of(db: &Connection, ids: &[Id]) -> rusqlite::Result<HashMap<Id, String>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut statement = db.prepare_cached(
        "SELECT \"MessageId\", \"Question\" FROM \"Polls\" WHERE \"MessageId\" IN (SELECT value FROM json_each(?1))",
    )?;
    statement
        .query_map([json_ids(ids)], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}
