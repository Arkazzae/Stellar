//! Polls: a question with answers that a message in a server channel carries.
//!
//! A poll is posted as part of its message (`POST
//! /api/channels/{channelId}/messages` with `poll`), and is never edited; it
//! goes when its message goes. People who see the channel vote with `PUT
//! /api/messages/{messageId}/poll/vote`, its author or whoever manages
//! messages there ends it early with `POST .../poll/end`, and `GET
//! .../poll/answers/{answerId}/voters` lists who chose an answer. The worker
//! `poll-ends` ends each poll when its time is up.
//!
//! A poll has ended once it was ended early or its time has passed, whichever
//! came first. One rule decides what a reader is shown and whether a vote is
//! taken, so a poll whose time is up refuses votes before the worker has
//! marked it. Every change is one transaction of the one writer: the check
//! that the poll is open and the change it allows cannot be split.
//!
//! The counts go to everyone who sees the channel, and name nobody. Which
//! answers a person chose goes only to that person; anyone else learns who
//! chose an answer from the list they ask for.

mod read;
mod routes;
mod worker;

use std::time::Duration;

use rusqlite::{Connection, params};
use serde::Deserialize;
use stellarr_messages::{MessageKey, Messages};

use crate::http::{ApiError, Endpoints};
use crate::message_views::text;
use crate::messages::field_problem;
use crate::values::{Id, Timestamp};

pub use read::{PollAnswerDto, PollDto, one, polls_of, questions_of};
pub use worker::{pass, workers};

/// The longest question, in UTF-16 units.
pub const MAX_QUESTION_UNITS: usize = 300;
/// How many answers a poll has at least and at most.
pub const MIN_ANSWERS: usize = 2;
pub const MAX_ANSWERS: usize = 10;
/// The longest answer, in UTF-16 units.
pub const MAX_ANSWER_UNITS: usize = 55;
/// The longest emoji of an answer, as for a reaction.
pub const MAX_EMOJI_UNITS: usize = 64;

/// How long a poll runs: one of a fixed list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Length {
    Hour,
    FourHours,
    EightHours,
    Day,
    ThreeDays,
    Week,
}

impl Length {
    fn parse(name: &str) -> Option<Length> {
        Some(match name {
            "1h" => Length::Hour,
            "4h" => Length::FourHours,
            "8h" => Length::EightHours,
            "1d" => Length::Day,
            "3d" => Length::ThreeDays,
            "1w" => Length::Week,
            _ => return None,
        })
    }

    pub fn duration(self) -> Duration {
        const HOUR: u64 = 3600;
        Duration::from_secs(match self {
            Length::Hour => HOUR,
            Length::FourHours => 4 * HOUR,
            Length::EightHours => 8 * HOUR,
            Length::Day => 24 * HOUR,
            Length::ThreeDays => 72 * HOUR,
            Length::Week => 168 * HOUR,
        })
    }
}

/// A poll as its author wrote it, checked against the limits.
#[derive(Debug, Clone)]
pub struct Draft {
    pub question: String,
    /// Each answer's text and emoji, in the order written.
    pub answers: Vec<(String, Option<String>)>,
    pub multiple: bool,
    pub length: Length,
}

/// A poll as a request sends it. Every member may be missing, so that what is
/// wrong is said of its field rather than of the whole body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollRequest {
    question: Option<String>,
    answers: Option<Vec<AnswerRequest>>,
    multiple: Option<bool>,
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AnswerRequest {
    text: Option<String>,
    emoji: Option<String>,
}

impl PollRequest {
    /// The poll, or the refusal of the first field that breaks a limit. An
    /// emoji follows the rule of a reaction: one name, the character or
    /// `:shortcode:`, without white space.
    pub fn draft(&self, messages: &Messages) -> Result<Draft, ApiError> {
        let question = text::trim(self.question.as_deref().unwrap_or("")).to_string();
        if question.is_empty() || text::units(&question) > MAX_QUESTION_UNITS {
            return Err(field_problem(
                "poll.question",
                messages,
                MessageKey::PollQuestionInvalid,
                &[("max", &MAX_QUESTION_UNITS)],
            ));
        }
        let given = self.answers.as_deref().unwrap_or(&[]);
        if !(MIN_ANSWERS..=MAX_ANSWERS).contains(&given.len()) {
            return Err(field_problem(
                "poll.answers",
                messages,
                MessageKey::PollAnswerCount,
                &[("min", &MIN_ANSWERS), ("max", &MAX_ANSWERS)],
            ));
        }
        let mut answers = Vec::with_capacity(given.len());
        for (index, answer) in given.iter().enumerate() {
            let field = format!("poll.answers[{index}]");
            let words = text::trim(answer.text.as_deref().unwrap_or("")).to_string();
            if words.is_empty() || text::units(&words) > MAX_ANSWER_UNITS {
                return Err(field_problem(
                    &field,
                    messages,
                    MessageKey::PollAnswerInvalid,
                    &[("max", &MAX_ANSWER_UNITS)],
                ));
            }
            let emoji = match answer.emoji.as_deref().map(text::trim) {
                None | Some("") => None,
                Some(emoji)
                    if text::units(emoji) <= MAX_EMOJI_UNITS
                        && !emoji.chars().any(text::is_white_space) =>
                {
                    Some(emoji.to_string())
                }
                Some(_) => {
                    return Err(field_problem(
                        &field,
                        messages,
                        MessageKey::PollEmojiInvalid,
                        &[("max", &MAX_EMOJI_UNITS)],
                    ));
                }
            };
            answers.push((words, emoji));
        }
        let length = match self.duration.as_deref() {
            None => Length::Day,
            Some(name) => Length::parse(name).ok_or_else(|| {
                field_problem("poll.duration", messages, MessageKey::PollDurationInvalid, &[])
            })?,
        };
        Ok(Draft {
            question,
            answers,
            multiple: self.multiple.unwrap_or(false),
            length,
        })
    }
}

/// Writes the poll of a message stored at `now`, in the transaction that
/// stores the message.
pub fn insert(db: &Connection, message: Id, draft: &Draft, now: Timestamp) -> rusqlite::Result<()> {
    db.prepare_cached(
        "INSERT INTO \"Polls\" (\"MessageId\", \"Question\", \"Multiple\", \"EndsAt\", \"EndedAt\", \"EndedById\")
         VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
    )?
    .execute(params![
        message,
        draft.question,
        draft.multiple,
        now.plus(draft.length.duration())
    ])?;
    for (position, (words, emoji)) in draft.answers.iter().enumerate() {
        db.prepare_cached(
            "INSERT INTO \"PollAnswers\" (\"MessageId\", \"AnswerId\", \"Text\", \"Emoji\") VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(params![message, position as i64 + 1, words, emoji])?;
    }
    Ok(())
}

/// Whether the message carries a poll.
pub fn exists(db: &Connection, message: Id) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"Polls\" WHERE \"MessageId\" = ?1)",
        [message],
        |row| row.get(0),
    )
}

/// When the poll ended, as of `now`: when it was ended, or when its time
/// was up if that has passed and nobody has marked it yet.
pub fn ended_at(ends_at: Timestamp, ended_at: Option<Timestamp>, now: Timestamp) -> Option<Timestamp> {
    ended_at.or((ends_at <= now).then_some(ends_at))
}

/// Writes one entry for the message in its channel's journal, as a change of
/// a reaction does, so that a client that missed the event reads the poll
/// again when it catches up. One entry for a whole vote, not one for each
/// answer it touched. Its sequence orders the events of the change.
pub(crate) fn journal(db: &Connection, message: Id) -> rusqlite::Result<i64> {
    db.prepare_cached(
        "INSERT INTO \"MessageChanges\" (\"ChannelId\", \"MessageId\", \"Kind\", \"CreatedAt\")
         SELECT \"ChannelId\", \"Id\", 0, strftime('%Y-%m-%d %H:%M:%f', 'now') FROM \"Messages\" WHERE \"Id\" = ?1",
    )?
    .execute([message])?;
    Ok(db.last_insert_rowid())
}

pub fn routes() -> Endpoints {
    Endpoints::new()
        .put("/api/messages/{messageId}/poll/vote", routes::vote)
        .authorized()
        .json_body()
        .post("/api/messages/{messageId}/poll/end", routes::end)
        .authorized()
        .get(
            "/api/messages/{messageId}/poll/answers/{answerId}/voters",
            routes::voters,
        )
        .authorized()
}
