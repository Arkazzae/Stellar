//! Voting in a poll, ending it early, and who chose an answer.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_messages::{MessageKey, Messages};

use super::read::{self, PollDto};
use crate::auth::Caller;
use crate::cursor::{Reader, Writer};
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::http::{
    ApiError, ApiResult, DEFAULT_PAGE_SIZE, Json, Path, Problem, QueryValues, json_response,
    valid_page_size,
};
use crate::instance::users::UserDto;
use crate::messages::field_problem;
use crate::permissions::authorization::{Scope, allows};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_MESSAGES;
use crate::permissions::visibility;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The problem code of a vote or an end that came after the end.
const ENDED: &str = "poll-ended";

/// The event that tells everyone who sees the channel how a poll stands.
const UPDATED: &str = "PollUpdated";
/// The event that tells a voter, on their own connections, what they chose.
const VOTE_CHANGED: &str = "PollVoteChanged";

#[derive(Deserialize)]
pub struct VoteRequest {
    answers: Option<Vec<i64>>,
}

/// A poll as everyone who sees its channel is told of it: counts, and no
/// names.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Updated {
    channel_id: Id,
    message_id: Id,
    /// Orders the updates of one message: they are sent after their commits
    /// and can arrive the other way round.
    sequence: i64,
    answers: Vec<Count>,
    voters: i64,
    ended_at: Option<Timestamp>,
    ended_by: Option<Id>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Count {
    id: i64,
    votes: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VoteChanged {
    channel_id: Id,
    message_id: Id,
    sequence: i64,
    mine: Vec<i64>,
}

/// A poll of a message of this server, as a vote or an end finds it.
struct Found {
    channel: Id,
    author: Id,
    multiple: bool,
    ends_at: Timestamp,
    ended_at: Option<Timestamp>,
    answers: i64,
}

/// The poll of the message on this server; none for a message that is not
/// there or carries no poll, and for a reader who does not see its channel,
/// to whom it is a message that does not exist.
fn find(db: &Connection, server: Id, message: Id, reader: Id) -> rusqlite::Result<Option<Found>> {
    let found: Option<Found> = db
        .query_row(
            "SELECT m.\"ChannelId\", m.\"AuthorId\", p.\"Multiple\", p.\"EndsAt\", p.\"EndedAt\",
                    (SELECT COUNT(*) FROM \"PollAnswers\" a WHERE a.\"MessageId\" = p.\"MessageId\")
             FROM \"Polls\" p JOIN \"Messages\" m ON m.\"Id\" = p.\"MessageId\"
             JOIN \"Channels\" c ON c.\"Id\" = m.\"ChannelId\"
             WHERE p.\"MessageId\" = ?1 AND c.\"ServerId\" = ?2",
            params![message, server],
            |row| {
                Ok(Found {
                    channel: row.get(0)?,
                    author: row.get(1)?,
                    multiple: row.get(2)?,
                    ends_at: row.get(3)?,
                    ended_at: row.get(4)?,
                    answers: row.get(5)?,
                })
            },
        )
        .optional()?;
    match found {
        Some(found) if visibility::views(db, reader, found.channel)? => Ok(Some(found)),
        _ => Ok(None),
    }
}

/// What a vote or an end came to: the poll for the one who asked, and the
/// change to tell others of, when there was one.
struct Outcome {
    poll: PollDto,
    channel: Id,
    sequence: Option<i64>,
}

/// Sets the reader's vote to the answers given, all of them at once: an
/// empty list takes it back. Asking twice is asking once.
pub async fn vote(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(message): Path<Id>,
    Json(request): Json<VoteRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let user = caller.user;
    let mut chosen: Vec<i64> = Vec::new();
    for answer in request.answers.iter().flatten() {
        if !chosen.contains(answer) {
            chosen.push(*answer);
        }
    }
    let outcome = state
        .database()
        .write(move |tx| -> Result<Outcome, ApiError> {
            let Some(found) = find(tx, server, message, user)? else {
                return Err(ApiError::not_found());
            };
            let now = Timestamp::now();
            if super::ended_at(found.ends_at, found.ended_at, now).is_some() {
                return Err(conflict(&messages, MessageKey::PollEnded, ENDED));
            }
            if chosen.iter().any(|answer| !(1..=found.answers).contains(answer)) {
                return Err(field_problem("answers", &messages, MessageKey::PollAnswersUnknown, &[]));
            }
            if !found.multiple && chosen.len() > 1 {
                return Err(field_problem("answers", &messages, MessageKey::PollSingleAnswer, &[]));
            }
            // The answers kept keep the time they were first chosen, which
            // orders the list of who chose them.
            let wanted = serde_json::to_string(&chosen).unwrap_or_else(|_| "[]".to_string());
            let mut changed = tx.execute(
                "DELETE FROM \"PollVotes\" WHERE \"MessageId\" = ?1 AND \"UserId\" = ?2
                   AND \"AnswerId\" NOT IN (SELECT value FROM json_each(?3))",
                params![message, user, wanted],
            )?;
            for answer in &chosen {
                changed += tx.execute(
                    "INSERT INTO \"PollVotes\" (\"MessageId\", \"AnswerId\", \"UserId\", \"CreatedAt\")
                     VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
                    params![message, answer, user, now],
                )?;
            }
            let sequence = if changed > 0 {
                Some(super::journal(tx, message)?)
            } else {
                None
            };
            let poll = read::one(tx, message, Some(user))?
                .ok_or_else(|| ApiError::internal("a poll was not found again"))?;
            Ok(Outcome {
                poll,
                channel: found.channel,
                sequence,
            })
        })
        .await?;
    if let Some(sequence) = outcome.sequence {
        announce(&state, server, message, outcome.channel, sequence, &outcome.poll).await?;
        state.events().publish(Event::new(
            Audience::Users(vec![user]),
            VOTE_CHANGED,
            &VoteChanged {
                channel_id: outcome.channel,
                message_id: message,
                sequence,
                mine: outcome.poll.mine.clone().unwrap_or_default(),
            },
        ));
    }
    Ok(json_response(StatusCode::OK, &outcome.poll))
}

/// Ends a poll now. Its author may, and so may whoever manages messages in
/// its channel; anyone else who sees the channel is refused, and anyone who
/// does not is answered as for a message that does not exist.
pub async fn end(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(message): Path<Id>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let user = caller.user;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Outcome, ApiError> {
            let Some(found) = find(tx, server, message, user)? else {
                return Err(ApiError::not_found());
            };
            if found.author != user
                && !allows(tx, user, &Scope::channel(server, found.channel), MANAGE_MESSAGES)?
            {
                return Err(match messages.get(MessageKey::PollEndDenied, &[]) {
                    Ok(title) => Problem::new(StatusCode::FORBIDDEN).title(title).into(),
                    Err(error) => error.into(),
                });
            }
            let now = Timestamp::now();
            // Only a poll that still runs is ended, in the one statement that
            // looks: an end in time or a second end cannot come between.
            let ended = tx.execute(
                "UPDATE \"Polls\" SET \"EndedAt\" = ?2, \"EndedById\" = ?3
                 WHERE \"MessageId\" = ?1 AND \"EndedAt\" IS NULL AND \"EndsAt\" > ?2",
                params![message, now, user],
            )?;
            if ended == 0 {
                return Err(conflict(&messages, MessageKey::PollEnded, ENDED));
            }
            let sequence = super::journal(tx, message)?;
            let poll = read::one(tx, message, Some(user))?
                .ok_or_else(|| ApiError::internal("a poll was not found again"))?;
            Ok(Outcome {
                poll,
                channel: found.channel,
                sequence: Some(sequence),
            })
        })
        .await?;
    if let Some(sequence) = outcome.sequence {
        announce(&state, server, message, outcome.channel, sequence, &outcome.poll).await?;
    }
    Ok(json_response(StatusCode::OK, &outcome.poll))
}

/// One who chose an answer: the account, when they chose it, and how others
/// see them.
type Voter = (Id, Timestamp, UserDto);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VotersPage {
    voters: Vec<UserDto>,
    /// Opaque; null once the list is at its end.
    next_cursor: Option<String>,
}

/// Who chose an answer, the earliest first, a page at a time.
pub async fn voters(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path((message, answer)): Path<(Id, i64)>,
    query: QueryValues,
) -> ApiResult<Json<VotersPage>> {
    let limit = query.int("limit")?.unwrap_or(DEFAULT_PAGE_SIZE);
    if !valid_page_size(limit) {
        return Err(field_problem(
            "limit",
            state.messages(),
            MessageKey::PageLimitRange,
            &[("max", &crate::http::MAX_PAGE_SIZE)],
        ));
    }
    let after = match query.text("cursor")? {
        None => None,
        Some(text) => Some(voter_cursor(&text).ok_or_else(ApiError::bad_request)?),
    };
    let user = caller.user;
    let found = state
        .database()
        .read(move |db| -> Result<Option<Vec<Voter>>, DbError> {
            let Some(poll) = find(db, server, message, user)? else {
                return Ok(None);
            };
            if !(1..=poll.answers).contains(&answer) {
                return Ok(None);
            }
            let mut statement = db.prepare_cached(
                "SELECT v.\"UserId\", v.\"CreatedAt\", u.\"Username\", u.\"DisplayName\", u.\"AvatarColor\", u.\"AvatarUrl\",
                        u.\"BannerUrl\", u.\"CustomStatus\", u.\"Bio\", u.\"Pronouns\", u.\"Origin\"
                 FROM \"PollVotes\" v JOIN \"Users\" u ON u.\"Id\" = v.\"UserId\"
                 WHERE v.\"MessageId\" = ?1 AND v.\"AnswerId\" = ?2
                   AND (?3 IS NULL OR (v.\"CreatedAt\", v.\"UserId\") > (?3, ?4))
                 ORDER BY v.\"CreatedAt\", v.\"UserId\" LIMIT ?5",
            )?;
            let rows = statement
                .query_map(
                    params![
                        message,
                        answer,
                        after.map(|(at, _)| at),
                        after.map(|(_, id)| id),
                        i64::from(limit) + 1
                    ],
                    |row| {
                        let id: Id = row.get(0)?;
                        Ok((
                            id,
                            row.get::<_, Timestamp>(1)?,
                            UserDto {
                                id: id.to_string(),
                                username: row.get(2)?,
                                display_name: row.get(3)?,
                                avatar_color: row.get(4)?,
                                presence: String::new(),
                                avatar_url: row.get(5)?,
                                banner_url: row.get(6)?,
                                custom_status: row.get(7)?,
                                bio: row.get(8)?,
                                pronouns: row.get(9)?,
                                origin: row.get(10)?,
                            },
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some(rows))
        })
        .await?;
    let Some(mut rows) = found else {
        return Err(ApiError::not_found());
    };
    let size = usize::try_from(limit).unwrap_or(0);
    let next_cursor = if rows.len() > size {
        rows.truncate(size);
        rows.last().map(|(id, at, _)| Writer::new().moment(*at).id(*id).finish())
    } else {
        None
    };
    let voters = rows
        .into_iter()
        .map(|(id, _, mut person)| {
            person.presence = state.presence().public_presence_of(id, Some(user));
            person
        })
        .collect();
    Ok(Json(VotersPage { voters, next_cursor }))
}

fn voter_cursor(text: &str) -> Option<(Timestamp, Id)> {
    let mut reader = Reader::open(text)?;
    let at = reader.moment()?;
    let id = reader.id()?;
    reader.done()?;
    Some((at, id))
}

/// Tells everyone who sees the channel how the poll stands now.
pub(super) async fn announce(
    state: &AppState,
    server: Id,
    message: Id,
    channel: Id,
    sequence: i64,
    poll: &PollDto,
) -> Result<(), ApiError> {
    let audience = state
        .database()
        .read(move |db| -> Result<Audience, DbError> {
            Ok(visibility::audience_of(db, server, channel)?)
        })
        .await?;
    state.events().publish(Event::new(
        audience,
        UPDATED,
        &Updated {
            channel_id: channel,
            message_id: message,
            sequence,
            answers: poll
                .answers
                .iter()
                .map(|answer| Count {
                    id: answer.id,
                    votes: answer.votes,
                })
                .collect(),
            voters: poll.voters,
            ended_at: poll.ended_at,
            ended_by: poll.ended_by,
        },
    ));
    Ok(())
}

/// A refusal of the state of things, with its title in the instance's
/// language and its code last.
fn conflict(messages: &Messages, title: MessageKey, code: &str) -> ApiError {
    match messages.get(title, &[]) {
        Ok(text) => Problem::new(StatusCode::CONFLICT).title(text).with("code", code).into(),
        Err(error) => error.into(),
    }
}
