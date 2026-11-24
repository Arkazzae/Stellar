//! A server's audit log, a page at a time.

use axum::extract::State;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Query};
use crate::instance::users::{User, UserDto};
use crate::permissions::audit::{self, AuditChange};
use crate::permissions::authorization::{self, Scope};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::VIEW_AUDIT_LOG;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const PAGE_SIZE: i64 = 50;

#[derive(Debug, Deserialize)]
pub struct Page {
    before: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDto {
    id: Id,
    summary: String,
    at: String,
    actor: UserDto,
    action: Option<String>,
    target: Option<TargetDto>,
    changes: Option<Vec<ChangeDto>>,
}

#[derive(Serialize)]
pub struct TargetDto {
    #[serde(rename = "type")]
    kind: String,
    id: Option<Id>,
    name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeDto {
    field: String,
    from: Option<String>,
    to: Option<String>,
    added: Option<Vec<String>>,
    removed: Option<Vec<String>>,
    from_color: Option<String>,
    to_color: Option<String>,
}

impl From<AuditChange> for ChangeDto {
    fn from(change: AuditChange) -> ChangeDto {
        ChangeDto {
            field: change.field,
            from: change.from,
            to: change.to,
            added: change.added,
            removed: change.removed,
            from_color: change.from_color,
            to_color: change.to_color,
        }
    }
}

/// The newest entries, or those older than `before`: the time of the last
/// entry the reader already has.
pub async fn list(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Query(page): Query<Page>,
) -> ApiResult<Json<Vec<EntryDto>>> {
    // A cursor that is there but empty is no moment at all, which is refused.
    let cursor = match page.before.as_deref() {
        Some(text) => Some(parse_moment(text).ok_or_else(ApiError::bad_request)?),
        None => None,
    };
    let found = state
        .database()
        .read(move |db| -> Result<Option<Vec<EntryDto>>, DbError> {
            if !authorization::allows(db, caller.user, &Scope::server(server), VIEW_AUDIT_LOG)? {
                return Ok(None);
            }
            let mut statement = db.prepare(
                "SELECT e.\"Id\", e.\"Summary\", e.\"CreatedAt\", e.\"ActorId\", e.\"Action\", e.\"TargetType\", e.\"TargetId\", e.\"TargetName\", e.\"ChangesJson\"
                 FROM \"ServerAuditEntries\" e JOIN \"Users\" a ON a.\"Id\" = e.\"ActorId\"
                 WHERE e.\"ServerId\" = ?1 AND (?2 IS NULL OR e.\"CreatedAt\" < ?2)
                 ORDER BY e.\"CreatedAt\" DESC LIMIT ?3",
            )?;
            let rows = statement
                .query_map(params![server, cursor, PAGE_SIZE], |row| {
                    Ok((
                        row.get::<_, Id>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Timestamp>(2)?,
                        row.get::<_, Id>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<Id>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut entries = Vec::with_capacity(rows.len());
            for (id, summary, at, actor, action, target_type, target_id, target_name, changes) in rows {
                let actor = User::find(db, actor)?
                    .ok_or_else(|| DbError::Task("an audit entry has no actor".into()))?;
                entries.push(EntryDto {
                    id,
                    summary,
                    at: at.to_wire_text(),
                    actor: actor.dto("offline"),
                    action,
                    target: target_type.map(|kind| TargetDto {
                        kind,
                        id: target_id,
                        name: target_name,
                    }),
                    changes: changes
                        .as_deref()
                        .map(|json| audit::read_changes(Some(json)).into_iter().map(ChangeDto::from).collect()),
                });
            }
            Ok(Some(entries))
        })
        .await?;
    found.map(Json).ok_or_else(ApiError::forbidden)
}

/// A moment as a query string gives it: an ISO date, with or without a time,
/// and with or without a zone. Without one it is taken as UTC; with an offset
/// it is moved to UTC.
fn parse_moment(text: &str) -> Option<Timestamp> {
    let text = text.trim();
    if let Some(moment) = parse_informal(text) {
        return Some(moment);
    }
    let (body, offset_seconds) = split_zone(text)?;
    let full = if body.len() == 10 {
        format!("{body}T00:00:00")
    } else {
        body.to_string()
    };
    let moment = Timestamp::parse(&full)?;
    let ticks = moment.ticks().checked_sub(offset_seconds.checked_mul(10_000_000)?)?;
    Timestamp::from_ticks(ticks)
}

/// The spellings of a moment that are not ISO: `11/24/2026` and
/// `11/24/2026 10:30:00` (month first), `Nov 24 2026` and `24 Nov 2026`, with
/// an optional comma, and a time of day alone, which is today's.
fn parse_informal(text: &str) -> Option<Timestamp> {
    const MONTHS: [&str; 12] = [
        "january", "february", "march", "april", "may", "june", "july", "august", "september",
        "october", "november", "december",
    ];
    let words: Vec<&str> = text
        .split([' ', ','])
        .filter(|word| !word.is_empty())
        .collect();
    let number = |word: &str| -> Option<i64> {
        (!word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()))
            .then(|| word.parse().ok())
            .flatten()
    };
    // A month by its name or by the first three letters or more of it.
    let month_of = |word: &str| -> Option<i64> {
        let lower = word.to_ascii_lowercase();
        MONTHS
            .iter()
            .position(|month| lower.len() >= 3 && month.starts_with(&lower))
            .map(|index| index as i64 + 1)
    };
    let (year, month, day, rest): (i64, i64, i64, &[&str]) = match words.as_slice() {
        [date, rest @ ..] if date.contains('/') => {
            let parts: Vec<&str> = date.split('/').collect();
            let [m, d, y] = parts.as_slice() else { return None };
            (number(y)?, number(m)?, number(d)?, rest)
        }
        [month, day, year, rest @ ..] if month_of(month).is_some() => {
            (number(year)?, month_of(month)?, number(day)?, rest)
        }
        [day, month, year, rest @ ..] if number(day).is_some() && month_of(month).is_some() => {
            (number(year)?, month_of(month)?, number(day)?, rest)
        }
        [time] if time.contains(':') => {
            let today = Timestamp::now().to_db_text();
            let date = today.get(..10)?;
            return Timestamp::parse(&format!("{date}T{}", pad_time(time)?));
        }
        _ => return None,
    };
    let time = match rest {
        [] => "00:00:00".to_string(),
        [time] if time.contains(':') => pad_time(time)?,
        _ => return None,
    };
    Timestamp::parse(&format!("{year:04}-{month:02}-{day:02}T{time}"))
}

/// `10:30` or `10:30:05` as `HH:MM:SS`.
fn pad_time(time: &str) -> Option<String> {
    let parts: Vec<&str> = time.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [h, m] => (h, m, "0"),
        [h, m, s] => (h, m, *s),
        _ => return None,
    };
    let part = |text: &str| -> Option<i64> {
        (!text.is_empty() && text.len() <= 2 && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok()).flatten()
    };
    Some(format!("{:02}:{:02}:{:02}", part(hours)?, part(minutes)?, part(seconds)?))
}

/// The text without its zone, and the zone's offset from UTC in seconds.
fn split_zone(text: &str) -> Option<(&str, i64)> {
    if let Some(body) = text.strip_suffix('Z').or_else(|| text.strip_suffix('z')) {
        return Some((body, 0));
    }
    // A sign after the time of day starts an offset; the dashes of the date
    // do not count.
    if text.len() > 11
        && let Some(at) = text[11..].rfind(['+', '-']).map(|at| at + 11)
    {
        let (body, zone) = text.split_at(at);
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let digits: String = zone[1..].chars().filter(|c| *c != ':').collect();
        if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let hours: i64 = digits[..2].parse().ok()?;
        let minutes: i64 = digits[2..].parse().ok()?;
        return Some((body, sign * (hours * 3600 + minutes * 60)));
    }
    Some((text, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cursor_is_read_in_the_spellings_a_client_sends() {
        let utc = Timestamp::parse("2026-11-24 10:00:00.5").unwrap();
        for text in [
            "2026-11-24T10:00:00.5Z",
            "2026-11-24T10:00:00.5",
            "2026-11-24 10:00:00.5",
            "2026-11-24T11:00:00.5+01:00",
            "2026-11-24T09:00:00.5-0100",
        ] {
            assert_eq!(parse_moment(text), Some(utc), "{text}");
        }
        assert_eq!(
            parse_moment("2026-11-24"),
            Timestamp::parse("2026-11-24 00:00:00")
        );
        let at_midnight = Timestamp::parse("2026-11-24 00:00:00");
        for text in ["Nov 24 2026", "November 24, 2026", "24 Nov 2026", "11/24/2026"] {
            assert_eq!(parse_moment(text), at_midnight, "{text}");
        }
        assert_eq!(
            parse_moment("11/24/2026 10:30"),
            Timestamp::parse("2026-11-24 10:30:00")
        );
        assert!(parse_moment("10:30:00").is_some());
        for text in ["", "yesterday", "2026-13-01", "2026-11-24T10:00", "2026-11-24T10:00:00+25"] {
            assert!(
                parse_moment(text).is_none() || text.is_empty(),
                "{text}"
            );
        }
    }
}
