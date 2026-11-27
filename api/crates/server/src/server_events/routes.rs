//! The routes of server events: the list, one event, making, changing and
//! cancelling one, signing up and out, and who signed up.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, params};
use serde::{Deserialize, Deserializer, Serialize};
use stellarr_messages::{MessageKey, Messages};
use stellarr_timezone::TimeZone;

use super::model::{Repeat, State as Phase};
use super::notices::{self, Notice};
use super::store::{self, Event, EventDto, Standing};
use super::{
    MAX_ACTIVE, MAX_DESCRIPTION_UNITS, MAX_DURATION, MAX_LEAD_DAYS, MAX_NAME_UNITS,
    MAX_PLACE_UNITS, MAX_SEATS, START_SLACK, TELL_CHANGES,
};
use crate::auth::Caller;
use crate::cursor::{Reader, Writer};
use crate::database::DbError;
use crate::events::{Audience, Event as Published};
use crate::http::{
    ApiError, ApiResult, DEFAULT_PAGE_SIZE, Json, Path, Problem, QueryValues, json_response,
    valid_page_size,
};
use crate::inbox::{self, Entry};
use crate::instance::users::UserDto;
use crate::message_views::text;
use crate::messages::post::{self, Posted, Stored, Submission};
use crate::messages::{Stop, field_problem};
use crate::permissions::authorization;
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::{CREATE_EVENTS, MANAGE_EVENTS, MANAGE_SERVER, contains};
use crate::permissions::visibility::{self, ChannelKind};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const LIMIT: &str = "event-limit";
const OVER: &str = "event-over";
const LIVE: &str = "event-live";
const GONE: &str = "event-occurrence-gone";
const ANNOUNCED: &str = "event-announced";

/// The event a change of an event tells everyone who sees it of.
pub(super) const CHANGED: &str = "EventChanged";
/// The event that tells one person where they stand now.
pub(super) const SIGNUP_CHANGED: &str = "EventSignupChanged";

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PlaceRequest {
    Voice { channel_id: Id },
    Elsewhere { text: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    name: Option<String>,
    description: Option<String>,
    place: Option<PlaceRequest>,
    starts_at: Option<Timestamp>,
    ends_at: Option<Timestamp>,
    time_zone: Option<String>,
    seats: Option<i64>,
    repeat: Option<String>,
    announce_channel_id: Option<Id>,
}

/// A member of a change that may be left out, set to null, or set.
#[derive(Default)]
pub enum Patch<T> {
    #[default]
    Absent,
    Null,
    Set(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Option::<T>::deserialize(deserializer)?.map_or(Patch::Null, Patch::Set))
    }
}

impl<T> Patch<T> {
    /// The value it leaves: the one it sets, none when it clears, `kept` when
    /// it says nothing.
    fn over(self, kept: Option<T>) -> Option<T> {
        match self {
            Patch::Absent => kept,
            Patch::Null => None,
            Patch::Set(value) => Some(value),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditRequest {
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    description: Patch<String>,
    #[serde(default)]
    place: Patch<PlaceRequest>,
    #[serde(default)]
    starts_at: Patch<Timestamp>,
    #[serde(default)]
    ends_at: Patch<Timestamp>,
    #[serde(default)]
    time_zone: Patch<String>,
    #[serde(default)]
    seats: Patch<i64>,
    #[serde(default)]
    repeat: Patch<String>,
    #[serde(default)]
    announce_channel_id: Patch<Id>,
}

/// What an event is to be, as a request asked for it, before it is checked.
struct Wanted {
    name: Option<String>,
    description: Option<String>,
    place: Option<PlaceRequest>,
    starts_at: Option<Timestamp>,
    ends_at: Option<Timestamp>,
    time_zone: Option<String>,
    seats: Option<i64>,
    repeat: Option<String>,
}

/// An event as checked against its limits.
struct Draft {
    name: String,
    description: Option<String>,
    voice: Option<Id>,
    place_text: Option<String>,
    zone: TimeZone,
    starts_at: Timestamp,
    ends_at: Option<Timestamp>,
    seats: Option<i64>,
    repeat: Repeat,
}

/// The event, or the refusal of the first field that breaks a limit. The
/// start is held to the window only when it is new: an event already under
/// way keeps the start it had.
fn draft(messages: &Messages, wanted: &Wanted, now: Timestamp, start_is_new: bool) -> Result<Draft, ApiError> {
    let name = text::trim(wanted.name.as_deref().unwrap_or("")).to_string();
    if name.is_empty() || text::units(&name) > MAX_NAME_UNITS {
        return Err(field_problem("name", messages, MessageKey::EventNameInvalid, &[("max", &MAX_NAME_UNITS)]));
    }
    let description = wanted
        .description
        .as_deref()
        .map(text::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_string);
    if description.as_deref().is_some_and(|description| text::units(description) > MAX_DESCRIPTION_UNITS) {
        return Err(field_problem(
            "description",
            messages,
            MessageKey::EventDescriptionInvalid,
            &[("max", &MAX_DESCRIPTION_UNITS)],
        ));
    }
    let (voice, place_text) = match &wanted.place {
        Some(PlaceRequest::Voice { channel_id }) => (Some(*channel_id), None),
        Some(PlaceRequest::Elsewhere { text: place }) => {
            let place = text::trim(place).to_string();
            if place.is_empty() || text::units(&place) > MAX_PLACE_UNITS {
                return Err(field_problem(
                    "place.text",
                    messages,
                    MessageKey::EventPlaceInvalid,
                    &[("max", &MAX_PLACE_UNITS)],
                ));
            }
            (None, Some(place))
        }
        None => {
            return Err(field_problem("place", messages, MessageKey::EventPlaceInvalid, &[("max", &MAX_PLACE_UNITS)]));
        }
    };
    let zone = wanted
        .time_zone
        .as_deref()
        .and_then(|name| TimeZone::find(name).ok())
        .ok_or_else(|| field_problem("timeZone", messages, MessageKey::EventTimeZoneInvalid, &[]))?;
    let latest = now.plus(Duration::from_secs(MAX_LEAD_DAYS * 86_400));
    let starts_at = match wanted.starts_at {
        Some(at) if !start_is_new || (at >= now.minus(START_SLACK) && at <= latest) => at,
        _ => {
            return Err(field_problem("startsAt", messages, MessageKey::EventStartInvalid, &[("days", &MAX_LEAD_DAYS)]));
        }
    };
    let repeat = match wanted.repeat.as_deref() {
        None => Repeat::None,
        Some(name) => Repeat::parse(name)
            .ok_or_else(|| field_problem("repeat", messages, MessageKey::EventRepeatInvalid, &[]))?,
    };
    match wanted.ends_at {
        None if place_text.is_some() => {
            return Err(field_problem("endsAt", messages, MessageKey::EventEndRequired, &[]));
        }
        Some(end) => {
            // An occurrence of a series ends before the next one starts.
            let length = starts_at.until(end);
            let fits = end > starts_at
                && length <= MAX_DURATION
                && repeat.shortest_gap().is_none_or(|gap| length < gap);
            if !fits {
                return Err(field_problem("endsAt", messages, MessageKey::EventEndInvalid, &[]));
            }
        }
        None => {}
    }
    if wanted.seats.is_some_and(|seats| !(1..=MAX_SEATS).contains(&seats)) {
        return Err(field_problem("seats", messages, MessageKey::EventSeatsInvalid, &[("max", &MAX_SEATS)]));
    }
    Ok(Draft {
        name,
        description,
        voice,
        place_text,
        zone,
        starts_at,
        ends_at: wanted.ends_at,
        seats: wanted.seats,
        repeat,
    })
}

/// A refusal of the state of things, with its title in the instance's
/// language and, for a conflict, its code.
fn refused(messages: &Messages, status: StatusCode, title: MessageKey, values: &[(&str, stellarr_messages::Value<'_>)], code: Option<&str>) -> ApiError {
    match messages.get(title, values) {
        Ok(text) => {
            let problem = Problem::new(status).title(text);
            match code {
                Some(code) => problem.with("code", code).into(),
                None => problem.into(),
            }
        }
        Err(error) => error.into(),
    }
}

/// The event, when the person sees it; one they do not is one that does not
/// exist.
fn visible(db: &Connection, id: Id, user: Id) -> Result<Event, ApiError> {
    match store::find(db, id)? {
        Some(event) if store::sees(db, user, &event)? => Ok(event),
        _ => Err(ApiError::not_found()),
    }
}

/// Whether the person may change or cancel the event: its creator, and
/// whoever manages events or the server.
fn may_manage(db: &Connection, messages: &Messages, event: &Event, user: Id) -> Result<(), ApiError> {
    if event.created_by == user {
        return Ok(());
    }
    let held = authorization::server_wide(db, user, event.server)?;
    if contains(held, MANAGE_EVENTS) || contains(held, MANAGE_SERVER) {
        return Ok(());
    }
    Err(refused(messages, StatusCode::FORBIDDEN, MessageKey::EventManageDenied, &[], None))
}

/// A voice channel of this server the person sees.
fn check_voice(db: &Connection, messages: &Messages, server: Id, user: Id, channel: Id) -> Result<(), ApiError> {
    match visibility::find(db, user, server, channel, Some(ChannelKind::Voice))? {
        Some(_) => Ok(()),
        None => Err(field_problem("place.channelId", messages, MessageKey::EventVoiceChannelInvalid, &[])),
    }
}

/// Posts the message that announces the event in a text channel, through the
/// post path, in the transaction it is given. An event in a voice channel is
/// announced only where everybody who reads the card sees that channel.
fn announce(
    db: &Connection,
    messages: &Messages,
    event: &Event,
    author: Id,
    channel: Id,
) -> Result<Stored, Stop> {
    if visibility::find(db, author, event.server, channel, Some(ChannelKind::Text))?.is_none() {
        return Err(field_problem("announceChannelId", messages, MessageKey::EventAnnounceInvalid, &[]).into());
    }
    if let Some(voice) = event.voice
        && !store::card_fits(db, event.server, voice, channel)?
    {
        return Err(field_problem("announceChannelId", messages, MessageKey::EventAnnounceHidden, &[]).into());
    }
    let submission = Submission {
        channel,
        author,
        content: String::new(),
        attachments: Vec::new(),
        reply_to: None,
        client_id: None,
        scheduled: None,
        poll: None,
        event: Some(event.id),
    };
    match post::post(db, messages, event.server, &submission)? {
        Posted::Stored(stored) => Ok(*stored),
        Posted::Replayed(_) => Err(ApiError::internal("an announcement was taken for a retry").into()),
    }
}

/// Where one person stands, as they are told of it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SignupChanged {
    event_id: Id,
    occurrence_id: Id,
    revision: i64,
    mine: &'static str,
    my_position: Option<i64>,
}

/// What a change tells once it has committed: the event to everyone who sees
/// it, each person whose standing changed their own, and the inbox entries
/// it wrote.
pub(super) struct Told {
    /// None when the event itself did not change, as for a reminder.
    shared: Option<(EventDto, Audience)>,
    personal: Vec<(Id, SignupChanged)>,
    entries: Vec<Entry>,
}

impl Told {
    /// Inbox entries alone, of a change that leaves the card as it was.
    pub(super) fn entries(entries: Vec<Entry>) -> Told {
        Told {
            shared: None,
            personal: Vec::new(),
            entries,
        }
    }
}

/// What to tell of the event as it is now, against where people stood
/// `before` the change.
pub(super) fn told(
    db: &Connection,
    id: Id,
    before: &HashMap<Id, Standing>,
    entries: Vec<Entry>,
    now: Timestamp,
) -> Result<Told, ApiError> {
    let event = store::find(db, id)?.ok_or_else(|| ApiError::internal("an event was not found again"))?;
    let after = store::standings(db, event.occurrence.id)?;
    let mut people: Vec<Id> = before.keys().chain(after.keys()).copied().collect();
    people.sort();
    people.dedup();
    let personal = people
        .into_iter()
        .filter(|person| before.get(person) != after.get(person))
        .map(|person| {
            let (mine, my_position) = match after.get(&person) {
                None => ("none", None),
                Some(Standing::Seated) => ("signed", None),
                Some(Standing::Waiting(place)) => ("waiting", Some(*place)),
            };
            (
                person,
                SignupChanged {
                    event_id: event.id,
                    occurrence_id: event.occurrence.id,
                    revision: event.revision,
                    mine,
                    my_position,
                },
            )
        })
        .collect();
    Ok(Told {
        shared: Some((store::dto(db, &event, None, now)?, store::audience(db, &event)?)),
        personal,
        entries,
    })
}

pub(super) fn publish(state: &AppState, told: Told) {
    let events = state.events();
    if let Some((shared, audience)) = told.shared {
        events.publish(Published::new(audience, CHANGED, &shared));
    }
    for (person, change) in &told.personal {
        events.publish(Published::new(Audience::Users(vec![*person]), SIGNUP_CHANGED, change));
    }
    for entry in &told.entries {
        inbox::announce(events, entry);
    }
}

fn reread(db: &Connection, id: Id, viewer: Id, now: Timestamp) -> Result<EventDto, ApiError> {
    let event = store::find(db, id)?.ok_or_else(|| ApiError::internal("an event was not found again"))?;
    Ok(store::dto(db, &event, Some(viewer), now)?)
}

/// The reader's upcoming and running events on the server, the soonest first.
pub async fn list(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Json<Vec<EventDto>>> {
    let user = caller.user;
    let found = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            let now = Timestamp::now();
            let mut seen = Vec::new();
            for event in store::active_of(db, server, now)? {
                if store::sees(db, user, &event)? {
                    seen.push(event);
                }
            }
            Ok(store::dtos(db, &seen, Some(user), now)?)
        })
        .await?;
    Ok(Json(found))
}

/// One event, in whatever state, for a card or a link.
pub async fn one(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<Json<EventDto>> {
    let user = caller.user;
    let found = state
        .database()
        .read(move |db| -> Result<_, ApiError> {
            let now = Timestamp::now();
            let event = visible(db, id, user)?;
            Ok(store::dto(db, &event, Some(user), now)?)
        })
        .await?;
    Ok(Json(found))
}

/// What a change of an event comes to: the event for whoever made it, the
/// announcement it posted, and what to tell others.
type Made = (EventDto, Option<Stored>, Told);

pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(request): Json<CreateRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let now = Timestamp::now();
    let wanted = Wanted {
        name: request.name,
        description: request.description,
        place: request.place,
        starts_at: request.starts_at,
        ends_at: request.ends_at,
        time_zone: request.time_zone,
        seats: request.seats,
        repeat: request.repeat,
    };
    let draft = draft(&messages, &wanted, now, true)?;
    let announce_in = request.announce_channel_id;
    let user = caller.user;
    let made = state
        .database()
        .write(move |tx| -> Result<Made, Stop> {
            if !contains(authorization::server_wide(tx, user, server)?, CREATE_EVENTS) {
                return Err(refused(&messages, StatusCode::FORBIDDEN, MessageKey::EventCreateDenied, &[], None).into());
            }
            if let Some(channel) = draft.voice {
                check_voice(tx, &messages, server, user, channel)?;
            }
            if store::count_active(tx, server, now)? >= MAX_ACTIVE {
                return Err(refused(&messages, StatusCode::CONFLICT, MessageKey::EventLimit, &[("max", &MAX_ACTIVE)], Some(LIMIT)).into());
            }
            let id = Id::random();
            tx.execute(
                "INSERT INTO \"ServerEvents\" (\"Id\", \"ServerId\", \"Name\", \"Description\", \"VoiceChannelId\", \"PlaceText\",
                     \"TimeZone\", \"FirstStartsAt\", \"DurationSeconds\", \"Repeat\", \"Seats\", \"CreatedById\", \"CreatedAt\",
                     \"CancelledAt\", \"Revision\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, 0)",
                params![
                    id,
                    server,
                    draft.name,
                    draft.description,
                    draft.voice,
                    draft.place_text,
                    draft.zone.id(),
                    draft.starts_at,
                    draft.ends_at.map(|end| draft.starts_at.until(end).as_secs() as i64),
                    draft.repeat.name(),
                    draft.seats,
                    user,
                    now,
                ],
            )?;
            tx.execute(
                "INSERT INTO \"ServerEventOccurrences\" (\"Id\", \"EventId\", \"Sequence\", \"StartsAt\", \"EndsAt\", \"Phase\")
                 VALUES (?1, ?2, 0, ?3, ?4, 'upcoming')",
                params![Id::random(), id, draft.starts_at, draft.ends_at],
            )?;
            let event = store::find(tx, id)?.ok_or_else(|| ApiError::internal("an event was not found again"))?;
            let stored = match announce_in {
                Some(channel) => Some(announce(tx, &messages, &event, user, channel)?),
                None => None,
            };
            let told = told(tx, id, &HashMap::new(), Vec::new(), now)?;
            Ok((reread(tx, id, user, now)?, stored, told))
        })
        .await;
    finish(&state, server, user, made, &messages, StatusCode::CREATED).await
}

async fn finish(
    state: &AppState,
    server: Id,
    user: Id,
    made: Result<Made, Stop>,
    messages: &Messages,
    status: StatusCode,
) -> ApiResult<Response> {
    let (reader, stored, told) = match made {
        Ok(made) => made,
        Err(stop) => return Ok(stop.into_response(messages)),
    };
    if let Some(stored) = stored {
        post::announce(state, server, user, &stored).await?;
    }
    publish(state, told);
    state.event_alarm().ring();
    Ok(json_response(status, &reader))
}

/// Changes an event. While it runs, its time and place stay as they are; a
/// new time or place keeps the sign-ups, arms the reminders again, and is
/// told to everyone signed up and waiting.
pub async fn edit(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
    Json(request): Json<EditRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let user = caller.user;
    let made = state
        .database()
        .write(move |tx| -> Result<(Id, Made), Stop> {
            let now = Timestamp::now();
            let event = visible(tx, id, user)?;
            may_manage(tx, &messages, &event, user)?;
            let phase = event.state(now);
            if !phase.is_active() {
                return Err(refused(&messages, StatusCode::CONFLICT, MessageKey::EventOver, &[], Some(OVER)).into());
            }
            let occurrence = &event.occurrence;
            let place = match (event.voice, &event.place_text) {
                (Some(channel), _) => PlaceRequest::Voice { channel_id: channel },
                (None, text) => PlaceRequest::Elsewhere { text: text.clone().unwrap_or_default() },
            };
            let start_is_new = matches!(request.starts_at, Patch::Set(at) if at != occurrence.starts_at);
            let wanted = Wanted {
                name: request.name.over(Some(event.name.clone())),
                description: request.description.over(event.description.clone()),
                place: request.place.over(Some(place)),
                starts_at: request.starts_at.over(Some(occurrence.starts_at)),
                ends_at: request.ends_at.over(occurrence.ends_at),
                time_zone: request.time_zone.over(Some(event.zone.clone())),
                seats: request.seats.over(event.seats),
                repeat: request.repeat.over(Some(event.repeat.name().to_string())),
            };
            let draft = draft(&messages, &wanted, now, start_is_new)?;
            let moved = draft.starts_at != occurrence.starts_at
                || draft.ends_at != occurrence.ends_at
                || draft.voice != event.voice
                || draft.place_text != event.place_text;
            if moved && phase == Phase::Live {
                return Err(refused(&messages, StatusCode::CONFLICT, MessageKey::EventLive, &[], Some(LIVE)).into());
            }
            if let Some(channel) = draft.voice.filter(|channel| Some(*channel) != event.voice) {
                check_voice(tx, &messages, event.server, user, channel)?;
                // A card already out must not tell its readers of a channel
                // they do not see.
                for text in store::announced_in(tx, id)? {
                    if !store::card_fits(tx, event.server, channel, text)? {
                        return Err(field_problem("place.channelId", &messages, MessageKey::EventAnnounceHidden, &[]).into());
                    }
                }
            }
            // A new start, zone or repeat makes the occurrence now the first
            // of the series that follows.
            let new_rule = draft.starts_at != occurrence.starts_at
                || draft.zone.id() != event.zone
                || draft.repeat != event.repeat;
            let before = store::standings(tx, occurrence.id)?;
            tx.execute(
                "UPDATE \"ServerEvents\" SET \"Name\" = ?2, \"Description\" = ?3, \"VoiceChannelId\" = ?4, \"PlaceText\" = ?5,
                     \"TimeZone\" = ?6, \"DurationSeconds\" = ?7, \"Repeat\" = ?8, \"Seats\" = ?9,
                     \"FirstStartsAt\" = CASE WHEN ?10 THEN ?11 ELSE \"FirstStartsAt\" END
                 WHERE \"Id\" = ?1",
                params![
                    id,
                    draft.name,
                    draft.description,
                    draft.voice,
                    draft.place_text,
                    draft.zone.id(),
                    draft.ends_at.map(|end| draft.starts_at.until(end).as_secs() as i64),
                    draft.repeat.name(),
                    draft.seats,
                    new_rule,
                    draft.starts_at,
                ],
            )?;
            tx.execute(
                "UPDATE \"ServerEventOccurrences\" SET \"StartsAt\" = ?2, \"EndsAt\" = ?3,
                     \"Sequence\" = CASE WHEN ?4 THEN 0 ELSE \"Sequence\" END
                 WHERE \"Id\" = ?1",
                params![occurrence.id, draft.starts_at, draft.ends_at, new_rule],
            )?;
            if draft.starts_at != occurrence.starts_at {
                tx.execute(
                    "UPDATE \"ServerEventSignups\" SET \"RemindedAt\" = NULL WHERE \"OccurrenceId\" = ?1",
                    [occurrence.id],
                )?;
            }
            let event = store::find(tx, id)?.ok_or_else(|| ApiError::internal("an event was not found again"))?;
            let promoted = store::fill(tx, &event, now)?;
            let mut entries = notices::write(tx, &messages, Notice::Seat, &event, &promoted, now)?;
            if TELL_CHANGES && moved {
                let others: Vec<Id> = store::everyone(tx, event.occurrence.id)?
                    .into_iter()
                    .filter(|person| *person != user)
                    .collect();
                entries.extend(notices::write(tx, &messages, Notice::Changed, &event, &others, now)?);
            }
            let stored = match request.announce_channel_id {
                // One card per event: announcing again would show two.
                Patch::Set(_) if event.announcement.is_some() => {
                    return Err(refused(&messages, StatusCode::CONFLICT, MessageKey::EventAnnounced, &[], Some(ANNOUNCED)).into());
                }
                Patch::Set(channel) => Some(announce(tx, &messages, &event, user, channel)?),
                _ => None,
            };
            store::bump(tx, id)?;
            let told = told(tx, id, &before, entries, now)?;
            Ok((event.server, (reread(tx, id, user, now)?, stored, told)))
        })
        .await;
    match made {
        Ok((server, made)) => finish(&state, server, user, Ok(made), &messages, StatusCode::OK).await,
        Err(stop) => Ok(stop.into_response(&messages)),
    }
}

/// Cancels the whole series, and tells everyone signed up or waiting.
pub async fn cancel(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let messages = *state.messages();
    let user = caller.user;
    let told = state
        .database()
        .write(move |tx| -> Result<Told, ApiError> {
            let now = Timestamp::now();
            let event = visible(tx, id, user)?;
            may_manage(tx, &messages, &event, user)?;
            if !event.state(now).is_active() {
                return Err(refused(&messages, StatusCode::CONFLICT, MessageKey::EventOver, &[], Some(OVER)));
            }
            let before = store::standings(tx, event.occurrence.id)?;
            tx.execute(
                "UPDATE \"ServerEvents\" SET \"CancelledAt\" = ?2 WHERE \"Id\" = ?1",
                params![id, now],
            )?;
            let others: Vec<Id> = store::everyone(tx, event.occurrence.id)?
                .into_iter()
                .filter(|person| *person != user)
                .collect();
            let entries = notices::write(tx, &messages, Notice::Cancelled, &event, &others, now)?;
            store::bump(tx, id)?;
            told(tx, id, &before, entries, now)
        })
        .await?;
    publish(&state, told);
    state.event_alarm().ring();
    Ok(StatusCode::NO_CONTENT)
}

/// The current occurrence of an event the person sees, which a sign-up is for
/// and which is not over.
fn open_occurrence(
    db: &Connection,
    messages: &Messages,
    id: Id,
    occurrence: Id,
    user: Id,
    now: Timestamp,
) -> Result<Event, ApiError> {
    let event = visible(db, id, user)?;
    if event.occurrence.id != occurrence {
        return Err(refused(messages, StatusCode::CONFLICT, MessageKey::EventOccurrenceGone, &[], Some(GONE)));
    }
    if !event.state(now).is_active() {
        return Err(refused(messages, StatusCode::CONFLICT, MessageKey::EventOver, &[], Some(OVER)));
    }
    Ok(event)
}

/// Signs the reader up: a seat while one is free, the waitlist otherwise.
/// Asking twice is asking once.
pub async fn sign_up(
    State(state): State<AppState>,
    caller: Caller,
    Path((id, occurrence)): Path<(Id, Id)>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let user = caller.user;
    let (reader, told) = state
        .database()
        .write(move |tx| -> Result<(EventDto, Option<Told>), ApiError> {
            let now = Timestamp::now();
            let event = open_occurrence(tx, &messages, id, occurrence, user, now)?;
            let before = store::standings(tx, event.occurrence.id)?;
            // A seat left by somebody who left the server goes to those
            // already waiting before anybody new.
            let promoted = store::fill(tx, &event, now)?;
            let entries = notices::write(tx, &messages, Notice::Seat, &event, &promoted, now)?;
            let told = if store::sign_up(tx, &event, user, now)? || !promoted.is_empty() {
                store::bump(tx, id)?;
                Some(told(tx, id, &before, entries, now)?)
            } else {
                None
            };
            Ok((reread(tx, id, user, now)?, told))
        })
        .await?;
    if let Some(told) = told {
        publish(&state, told);
        state.event_alarm().ring();
    }
    Ok(json_response(StatusCode::OK, &reader))
}

/// Takes the reader's sign-up back. A seat it frees goes to the first person
/// waiting, in the same transaction, with a notice.
pub async fn sign_out(
    State(state): State<AppState>,
    caller: Caller,
    Path((id, occurrence)): Path<(Id, Id)>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let user = caller.user;
    let (reader, told) = state
        .database()
        .write(move |tx| -> Result<(EventDto, Option<Told>), ApiError> {
            let now = Timestamp::now();
            let event = open_occurrence(tx, &messages, id, occurrence, user, now)?;
            let before = store::standings(tx, event.occurrence.id)?;
            let told = if store::sign_out(tx, &event, user)? {
                let promoted = store::fill(tx, &event, now)?;
                let entries = notices::write(tx, &messages, Notice::Seat, &event, &promoted, now)?;
                store::bump(tx, id)?;
                Some(told(tx, id, &before, entries, now)?)
            } else {
                None
            };
            Ok((reread(tx, id, user, now)?, told))
        })
        .await?;
    if let Some(told) = told {
        publish(&state, told);
    }
    Ok(json_response(StatusCode::OK, &reader))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    user: UserDto,
    /// `signed` or `waiting`.
    state: &'static str,
    position: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct People {
    people: Vec<Person>,
    /// Opaque; null once the list is at its end.
    next_cursor: Option<String>,
}

/// Who signed up for the occurrence, those with a seat first.
pub async fn signups(
    State(state): State<AppState>,
    caller: Caller,
    Path((id, occurrence)): Path<(Id, Id)>,
    query: QueryValues,
) -> ApiResult<Json<People>> {
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
        Some(text) => Some(signup_cursor(&text).ok_or_else(ApiError::bad_request)?),
    };
    let user = caller.user;
    let mut rows = state
        .database()
        .read(move |db| -> Result<_, ApiError> {
            let event = visible(db, id, user)?;
            if event.occurrence.id != occurrence {
                return Err(ApiError::not_found());
            }
            Ok(store::signups(db, occurrence, after, i64::from(limit) + 1)?)
        })
        .await?;
    let size = usize::try_from(limit).unwrap_or(0);
    let next_cursor = if rows.len() > size {
        rows.truncate(size);
        rows.last()
            .map(|(number, seated, _, _)| Writer::new().byte(u8::from(*seated)).long(*number as u64).finish())
    } else {
        None
    };
    let people = rows
        .into_iter()
        .map(|(_, seated, place, user)| Person {
            user,
            state: if seated { "signed" } else { "waiting" },
            position: (!seated).then_some(place),
        })
        .collect();
    Ok(Json(People { people, next_cursor }))
}

fn signup_cursor(text: &str) -> Option<(bool, i64)> {
    let mut reader = Reader::open(text)?;
    let seated = reader.byte()? == 1;
    let number = i64::try_from(reader.long()?).ok()?;
    reader.done()?;
    Some((seated, number))
}
