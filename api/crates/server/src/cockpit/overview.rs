//! What the panel opens on: who the operator is, the instance in numbers and
//! what happened over the last month.

use std::collections::HashSet;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use stellarr_timezone::TimeZone;

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiResult, json_response};
use crate::permissions::membership::InstanceOwner;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// How many days the overview's activity chart covers, today included.
const ACTIVITY_DAYS: i64 = 30;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Operator {
    id: Id,
    username: String,
    display_name: String,
    avatar_url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Instance {
    id: Id,
    name: String,
    created_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Overview {
    user_count: usize,
    online_user_count: usize,
    realtime_connection_count: usize,
    stored_object_bytes: i64,
    message_count: i64,
    server_count: i64,
    public_server_count: i64,
}

#[derive(Serialize)]
struct Bootstrap {
    operator: Operator,
    instance: Instance,
    overview: Overview,
}

pub(super) async fn bootstrap(
    State(state): State<AppState>,
    caller: Caller,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Response> {
    let reading = state.clone();
    let answer = state
        .database()
        .read(move |db| -> Result<Bootstrap, DbError> {
            let operator = db.query_row(
                "SELECT \"Id\", \"Username\", \"DisplayName\", \"AvatarUrl\" FROM \"Users\" WHERE \"Id\" = ?1",
                [caller.user],
                |row| {
                    Ok(Operator {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        display_name: row.get(2)?,
                        avatar_url: row.get(3)?,
                    })
                },
            )?;
            let instance = db.query_row(
                "SELECT \"Id\", \"Name\", \"CreatedAt\" FROM \"Servers\" WHERE \"Id\" = ?1",
                [server],
                |row| {
                    Ok(Instance {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        created_at: row.get(2)?,
                    })
                },
            )?;
            let mut statement =
                db.prepare("SELECT \"UserId\" FROM \"ServerMembers\" WHERE \"ServerId\" = ?1")?;
            let members = statement
                .query_map([server], |row| row.get::<_, Id>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let one = |sql: &str, parameters: &[&dyn rusqlite::ToSql]| {
                db.query_row(sql, parameters, |row| row.get::<_, i64>(0))
            };
            let attachments = one(
                "SELECT COALESCE(SUM(\"SizeBytes\" + COALESCE(\"PosterSizeBytes\", 0) + COALESCE(\"PreviewSizeBytes\", 0)), 0) FROM \"ImageAttachments\"",
                &[],
            )?;
            let emoji = one(
                "SELECT COALESCE(SUM(\"SizeBytes\"), 0) FROM \"ServerEmoji\" WHERE \"ServerId\" = ?1",
                &[&server],
            )?;
            let presence = reading.presence();
            Ok(Bootstrap {
                operator,
                instance,
                overview: Overview {
                    user_count: members.len(),
                    online_user_count: members
                        .iter()
                        .filter(|id| presence.public_presence_of(**id, None) != "offline")
                        .count(),
                    realtime_connection_count: members
                        .iter()
                        .map(|id| presence.connection_ids(*id).len())
                        .sum(),
                    stored_object_bytes: attachments + emoji,
                    message_count: one(
                        "SELECT COUNT(*) FROM \"Messages\" m WHERE EXISTS (SELECT 1 FROM \"Channels\" c WHERE c.\"Id\" = m.\"ChannelId\" AND c.\"ServerId\" = ?1)",
                        &[&server],
                    )?,
                    server_count: one("SELECT COUNT(*) FROM \"Servers\"", &[])?,
                    public_server_count: one(
                        "SELECT COUNT(*) FROM \"Servers\" WHERE \"IsPublic\" = 1",
                        &[],
                    )?,
                },
            })
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Day {
    date: String,
    messages: usize,
    new_members: usize,
    active_users: usize,
    reactions: usize,
    uploads: usize,
    sign_ins: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Activity {
    time_zone: String,
    days: Vec<Day>,
    active_users: usize,
}

/// The day number of a date, counted from 1970-01-01.
/// The date of a day number, as `yyyy-MM-dd`.
fn civil_date(days: i64) -> String {
    let (year, month, day) = crate::values::civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The local day of a moment in the zone, as a day number.
fn local_day(zone: &TimeZone, at: Timestamp) -> i64 {
    let seconds = at.unix_seconds();
    let local = zone.local_time(seconds);
    crate::values::days_from_civil(local.year, i64::from(local.month), i64::from(local.day))
}

/// The named zone, or UTC when the name is empty or unknown to this host.
pub(crate) fn find_zone(id: Option<&str>) -> TimeZone {
    match id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => TimeZone::find(id).unwrap_or_else(|_| TimeZone::utc()),
        None => TimeZone::utc(),
    }
}

fn count_by_day(instants: &[Timestamp], first: i64, zone: &TimeZone) -> Vec<usize> {
    let mut counts = vec![0; ACTIVITY_DAYS as usize];
    for instant in instants {
        let index = local_day(zone, *instant) - first;
        if (0..ACTIVITY_DAYS).contains(&index) {
            counts[index as usize] += 1;
        }
    }
    counts
}

/// What happened per day of the instance's time zone, oldest first and today
/// last: messages, the people who wrote them, joins and reactions on the
/// instance server, and uploads and sign-ins on the whole instance.
pub(super) async fn activity(
    State(state): State<AppState>,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Response> {
    let configured = state.settings().operations.time_zone.clone();
    let answer = state
        .database()
        .read(move |db| -> Result<Activity, DbError> {
            let saved: Option<String> = db
                .query_row(
                    "SELECT \"TimeZone\" FROM \"OperationsSettings\" WHERE \"ServerId\" = ?1",
                    [server],
                    |row| row.get(0),
                )
                .optional()?;
            let zone = match saved.or(configured) {
                Some(id) => find_zone(Some(&id)),
                None => TimeZone::local(),
            };
            let now = Timestamp::now();
            let today = local_day(&zone, now);
            let first = today - (ACTIVITY_DAYS - 1);
            // One day more than the window covers the first local day in any
            // zone; the counting drops whatever falls before it.
            let since = now.minus(std::time::Duration::from_secs(((ACTIVITY_DAYS + 1) * 86_400) as u64));
            let times = |sql: &str, parameters: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<Vec<Timestamp>> {
                let mut statement = db.prepare(sql)?;
                statement
                    .query_map(parameters, |row| row.get(0))?
                    .collect()
            };
            let mut statement = db.prepare(
                "SELECT m.\"CreatedAt\", m.\"AuthorId\", m.\"IsSystem\" FROM \"Messages\" m
                 WHERE m.\"CreatedAt\" >= ?2 AND EXISTS (SELECT 1 FROM \"Channels\" c WHERE c.\"Id\" = m.\"ChannelId\" AND c.\"ServerId\" = ?1)",
            )?;
            let messages: Vec<(Timestamp, Id, bool)> = statement
                .query_map(params![server, since], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            let joins = times(
                "SELECT \"JoinedAt\" FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"JoinedAt\" >= ?2",
                &[&server, &since],
            )?;
            let reactions = times(
                "SELECT r.\"CreatedAt\" FROM \"MessageReactions\" r WHERE r.\"CreatedAt\" >= ?2
                 AND EXISTS (SELECT 1 FROM \"Messages\" m JOIN \"Channels\" c ON c.\"Id\" = m.\"ChannelId\" WHERE m.\"Id\" = r.\"MessageId\" AND c.\"ServerId\" = ?1)",
                &[&server, &since],
            )?;
            let uploads = times(
                "SELECT \"CreatedAt\" FROM \"ImageAttachments\" WHERE \"CreatedAt\" >= ?1",
                &[&since],
            )?;
            let sign_ins = times(
                "SELECT \"CreatedAt\" FROM \"UserSessions\" WHERE \"CreatedAt\" >= ?1",
                &[&since],
            )?;
            let message_counts =
                count_by_day(&messages.iter().map(|m| m.0).collect::<Vec<_>>(), first, &zone);
            // System messages have an author on record but nobody wrote them.
            let mut per_day: Vec<HashSet<Id>> = vec![HashSet::new(); ACTIVITY_DAYS as usize];
            let mut everyone = HashSet::new();
            for (at, author, system) in &messages {
                let index = local_day(&zone, *at) - first;
                if !system && (0..ACTIVITY_DAYS).contains(&index) {
                    per_day[index as usize].insert(*author);
                    everyone.insert(*author);
                }
            }
            let joins = count_by_day(&joins, first, &zone);
            let reactions = count_by_day(&reactions, first, &zone);
            let uploads = count_by_day(&uploads, first, &zone);
            let sign_ins = count_by_day(&sign_ins, first, &zone);
            Ok(Activity {
                time_zone: zone.id().to_string(),
                days: (0..ACTIVITY_DAYS as usize)
                    .map(|index| Day {
                        date: civil_date(first + index as i64),
                        messages: message_counts[index],
                        new_members: joins[index],
                        active_users: per_day[index].len(),
                        reactions: reactions[index],
                        uploads: uploads[index],
                        sign_ins: sign_ins[index],
                    })
                    .collect(),
                active_users: everyone.len(),
            })
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_numbers_and_dates_are_each_others_inverse() {
        for (year, month, day, number) in [(1970, 1, 1, 0), (2000, 2, 29, 11_016), (2026, 11, 24, 20_781), (1969, 12, 31, -1)] {
            assert_eq!(crate::values::days_from_civil(year, month, day), number);
            assert_eq!(civil_date(number), format!("{year:04}-{month:02}-{day:02}"));
        }
    }

    #[test]
    fn a_name_the_host_does_not_know_is_utc() {
        assert_eq!(find_zone(Some("Nowhere/Land")).id(), "UTC");
        assert_eq!(find_zone(Some("  ")).id(), "UTC");
        assert_eq!(find_zone(None).id(), "UTC");
    }
}
