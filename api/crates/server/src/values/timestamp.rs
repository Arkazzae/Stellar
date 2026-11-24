//! A moment in UTC, to the tenth of a microsecond.
//!
//! The database holds it as text, `2026-11-18 21:57:04.81431`: a space between
//! date and time, and a fraction of up to seven digits with its trailing
//! zeros left off (and the point too when there is no fraction). The JSON the
//! clients see has the same, with a `T` and a `Z`: `2026-11-18T21:57:04.81431Z`.
//! Both are read back, and a moment survives either.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

const TICKS_PER_SECOND: i64 = 10_000_000;
const SECONDS_PER_DAY: i64 = 86_400;
/// 0001-01-01 and 9999-12-31 at 23:59:59.9999999, in ticks from the Unix epoch.
const FIRST_TICK: i64 = -62_135_596_800 * TICKS_PER_SECOND;
const LAST_TICK: i64 = 253_402_300_799 * TICKS_PER_SECOND + 9_999_999;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp(i64);

impl Timestamp {
    pub fn now() -> Timestamp {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp::from_ticks(i64::try_from(elapsed.as_nanos() / 100).unwrap_or(i64::MAX))
            .unwrap_or(Timestamp(LAST_TICK))
    }

    /// Ticks of 100 nanoseconds since 1970-01-01T00:00:00Z. None when the
    /// moment is outside the years 1 to 9999.
    pub fn from_ticks(ticks: i64) -> Option<Timestamp> {
        (FIRST_TICK..=LAST_TICK)
            .contains(&ticks)
            .then_some(Timestamp(ticks))
    }

    pub fn from_unix_seconds(seconds: i64) -> Option<Timestamp> {
        Timestamp::from_ticks(seconds.checked_mul(TICKS_PER_SECOND)?)
    }

    pub fn ticks(self) -> i64 {
        self.0
    }

    pub fn unix_seconds(self) -> i64 {
        self.0.div_euclid(TICKS_PER_SECOND)
    }

    pub fn unix_millis(self) -> i64 {
        self.0.div_euclid(10_000)
    }

    /// Later by `by`; held at the last moment there is.
    pub fn plus(self, by: Duration) -> Timestamp {
        let ticks = i64::try_from(by.as_nanos() / 100).unwrap_or(i64::MAX);
        Timestamp(self.0.saturating_add(ticks).min(LAST_TICK))
    }

    /// Earlier by `by`; held at the first moment there is.
    pub fn minus(self, by: Duration) -> Timestamp {
        let ticks = i64::try_from(by.as_nanos() / 100).unwrap_or(i64::MAX);
        Timestamp(self.0.saturating_sub(ticks).max(FIRST_TICK))
    }

    /// How long before `later` this moment is; zero when it is not.
    pub fn until(self, later: Timestamp) -> Duration {
        let ticks = later.0.saturating_sub(self.0).max(0);
        Duration::from_nanos(u64::try_from(ticks).unwrap_or(0).saturating_mul(100))
    }

    /// The text the database holds.
    pub fn to_db_text(self) -> String {
        self.spell(' ', "")
    }

    /// The text the JSON carries.
    pub fn to_wire_text(self) -> String {
        self.spell('T', "Z")
    }

    /// The moment as an HTTP date, `Thu, 24 Dec 2026 01:18:38 GMT`, which is
    /// how a cookie names the time it expires.
    pub fn to_http_date(self) -> String {
        const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let seconds = self.0.div_euclid(TICKS_PER_SECOND);
        let days = seconds.div_euclid(SECONDS_PER_DAY);
        let of_day = seconds.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        format!(
            "{}, {day:02} {} {year:04} {:02}:{:02}:{:02} GMT",
            DAYS[days.rem_euclid(7) as usize],
            MONTHS[(month - 1) as usize],
            of_day / 3600,
            of_day / 60 % 60,
            of_day % 60
        )
    }

    /// Reads either spelling. A time zone is not accepted other than `Z`:
    /// everything in the database is UTC.
    pub fn parse(text: &str) -> Option<Timestamp> {
        let bytes = text.as_bytes();
        if bytes.len() < 19 || !matches!(bytes[10], b' ' | b'T') {
            return None;
        }
        let digits = |range: std::ops::Range<usize>| -> Option<i64> {
            let part = bytes.get(range)?;
            part.iter().all(u8::is_ascii_digit).then(|| {
                part.iter()
                    .fold(0i64, |acc, digit| acc * 10 + i64::from(digit - b'0'))
            })
        };
        if bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' || bytes[16] != b':' {
            return None;
        }
        let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
        let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
        let mut rest = &bytes[19..];
        if rest.last() == Some(&b'Z') {
            rest = &rest[..rest.len() - 1];
        }
        let mut fraction = 0i64;
        if !rest.is_empty() {
            let (point, places) = rest.split_first()?;
            if *point != b'.' || places.is_empty() || !places.iter().all(u8::is_ascii_digit) {
                return None;
            }
            for position in 0..7 {
                let digit = places
                    .get(position)
                    .map_or(0, |digit| i64::from(digit - b'0'));
                fraction = fraction * 10 + digit;
            }
        }
        if !(1..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || day < 1
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return None;
        }
        let days = days_from_civil(year, month, day);
        let seconds = days * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second;
        Timestamp::from_ticks(seconds * TICKS_PER_SECOND + fraction)
    }

    fn spell(self, separator: char, zone: &str) -> String {
        let seconds = self.0.div_euclid(TICKS_PER_SECOND);
        let fraction = self.0.rem_euclid(TICKS_PER_SECOND);
        let days = seconds.div_euclid(SECONDS_PER_DAY);
        let of_day = seconds.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        let mut text = format!(
            "{year:04}-{month:02}-{day:02}{separator}{:02}:{:02}:{:02}",
            of_day / 3600,
            of_day / 60 % 60,
            of_day % 60
        );
        if fraction != 0 {
            let digits = format!("{fraction:07}");
            text.push('.');
            text.push_str(digits.trim_end_matches('0'));
        }
        text.push_str(zone);
        text
    }
}

pub(crate) fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to the given date in the proleptic Gregorian calendar.
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub(crate) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_wire_text())
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Timestamp({self})")
    }
}

impl ToSql for Timestamp {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_db_text()))
    }
}

impl FromSql for Timestamp {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Timestamp> {
        let text = value.as_str()?;
        Timestamp::parse(text)
            .ok_or_else(|| FromSqlError::Other(format!("\"{text}\" is not a moment").into()))
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_wire_text())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
        let text = String::deserialize(deserializer)?;
        Timestamp::parse(&text).ok_or_else(|| de::Error::custom("not a moment"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Timestamp fixtures with seven, five, six, three and zero fractional digits.
    const STORED: [&str; 5] = [
        "2026-11-18 21:20:56.8143104",
        "2026-11-18 21:57:04.81431",
        "2026-11-17 17:48:37.617954",
        "2026-11-18 21:20:57.403",
        "2026-11-18 21:20:57",
    ];

    #[test]
    fn every_stored_value_is_read_and_written_back_the_same() {
        for text in STORED {
            let moment = Timestamp::parse(text).unwrap();

            assert_eq!(moment.to_db_text(), text);
        }
    }

    #[test]
    fn the_wire_spelling_has_a_t_and_a_z() {
        assert_eq!(
            Timestamp::parse("2026-11-24 00:33:18.6419643")
                .unwrap()
                .to_wire_text(),
            "2026-11-24T00:33:18.6419643Z"
        );
        assert_eq!(
            Timestamp::parse("2026-11-18 21:57:04.81431")
                .unwrap()
                .to_wire_text(),
            "2026-11-18T21:57:04.81431Z"
        );
        assert_eq!(
            Timestamp::parse("2026-11-18 21:20:57")
                .unwrap()
                .to_wire_text(),
            "2026-11-18T21:20:57Z"
        );
    }

    #[test]
    fn either_spelling_is_read() {
        assert_eq!(
            Timestamp::parse("2026-11-24T00:33:18.6419643Z"),
            Timestamp::parse("2026-11-24 00:33:18.6419643")
        );
        assert_eq!(
            Timestamp::parse("2026-11-24T00:33:18Z"),
            Timestamp::parse("2026-11-24 00:33:18")
        );
    }

    #[test]
    fn known_moments_have_the_right_unix_time() {
        assert_eq!(
            Timestamp::parse("1970-01-01 00:00:00")
                .unwrap()
                .unix_seconds(),
            0
        );
        assert_eq!(
            Timestamp::parse("2000-02-29 12:00:00")
                .unwrap()
                .unix_seconds(),
            951_825_600
        );
        assert_eq!(
            Timestamp::parse("2026-11-24 00:00:00")
                .unwrap()
                .unix_seconds(),
            1_795_478_400
        );
        assert_eq!(
            Timestamp::parse("1969-12-31 23:59:59.5")
                .unwrap()
                .unix_millis(),
            -500
        );
        assert_eq!(
            Timestamp::parse("0001-01-01 00:00:00")
                .unwrap()
                .unix_seconds(),
            -62_135_596_800
        );
        assert_eq!(
            Timestamp::parse("9999-12-31 23:59:59.9999999")
                .unwrap()
                .to_db_text(),
            "9999-12-31 23:59:59.9999999"
        );
    }

    #[test]
    fn every_day_of_four_hundred_years_survives_a_round_trip() {
        let first = Timestamp::parse("1970-01-01 00:00:00")
            .unwrap()
            .unix_seconds();
        for day in 0..146_097 {
            let moment =
                Timestamp::from_unix_seconds(first + day * SECONDS_PER_DAY + 3661).unwrap();

            assert_eq!(Timestamp::parse(&moment.to_db_text()), Some(moment));
        }
    }

    #[test]
    fn a_text_that_is_not_a_moment_is_refused() {
        for text in [
            "",
            "2026-11-18",
            "2026-11-18 21:20",
            "2026-13-18 21:20:57",
            "2026-02-29 21:20:57",
            "2026-11-18 24:00:00",
            "2026-11-18 21:60:00",
            "2026-11-18 21:20:57.",
            "2026-11-18 21:20:57.x",
            "2026-11-18 21:20:57+01:00",
            "2026/11/18 21:20:57",
            "0000-01-01 00:00:00",
            "２０２６-11-18 21:20:57",
        ] {
            assert_eq!(Timestamp::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn digits_beyond_the_seventh_are_dropped() {
        assert_eq!(
            Timestamp::parse("2026-11-18 21:20:57.123456789")
                .unwrap()
                .to_db_text(),
            "2026-11-18 21:20:57.1234567"
        );
    }

    #[test]
    fn a_cookie_names_its_expiry_as_an_http_date() {
        assert_eq!(
            Timestamp::from_unix_seconds(0).unwrap().to_http_date(),
            "Thu, 01 Jan 1970 00:00:00 GMT"
        );
        assert_eq!(
            Timestamp::parse("2026-12-24 01:18:38.632863")
                .unwrap()
                .to_http_date(),
            "Thu, 24 Dec 2026 01:18:38 GMT"
        );
        assert_eq!(
            Timestamp::parse("2000-02-29 12:00:00")
                .unwrap()
                .to_http_date(),
            "Tue, 29 Feb 2000 12:00:00 GMT"
        );
    }

    #[test]
    fn arithmetic_moves_the_moment() {
        let moment = Timestamp::parse("2026-11-18 21:20:57.5").unwrap();

        assert_eq!(
            moment.plus(Duration::from_secs(30 * 86_400)).to_db_text(),
            "2026-12-18 21:20:57.5"
        );
        assert_eq!(
            moment.minus(Duration::from_millis(500)).to_db_text(),
            "2026-11-18 21:20:57"
        );
        assert_eq!(
            moment.until(moment.plus(Duration::from_secs(2))),
            Duration::from_secs(2)
        );
        assert_eq!(
            moment.until(moment.minus(Duration::from_secs(2))),
            Duration::ZERO
        );
        let now = Timestamp::now();
        assert!(now.until(now.plus(Duration::from_secs(5))) > Duration::from_secs(4));
    }

    #[test]
    fn it_goes_into_and_out_of_a_text_column() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t (At TEXT NOT NULL)")
            .unwrap();
        for text in STORED {
            let moment = Timestamp::parse(text).unwrap();
            db.execute("DELETE FROM t", []).unwrap();
            db.execute("INSERT INTO t (At) VALUES (?1)", [&moment])
                .unwrap();

            let raw: String = db
                .query_row("SELECT At FROM t", [], |row| row.get(0))
                .unwrap();
            assert_eq!(raw, text);
            let back: Timestamp = db
                .query_row("SELECT At FROM t", [], |row| row.get(0))
                .unwrap();
            assert_eq!(back, moment);
        }
    }

    #[test]
    fn json_carries_the_wire_spelling() {
        let moment = Timestamp::parse("2026-11-18 21:57:04.81431").unwrap();

        assert_eq!(
            serde_json::to_string(&moment).unwrap(),
            "\"2026-11-18T21:57:04.81431Z\""
        );
        let back: Timestamp = serde_json::from_str("\"2026-11-18T21:57:04.81431Z\"").unwrap();
        assert_eq!(back, moment);
    }
}
