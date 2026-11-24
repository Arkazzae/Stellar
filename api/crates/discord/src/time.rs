//! Moments in time: Discord's timestamps, the HTTP dates of its headers, and
//! the clock the rate limiter waits on.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A moment, to a tenth of a microsecond, as ticks since 1970-01-01T00:00:00Z.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

const TICKS_PER_SECOND: i64 = 10_000_000;

/// The first and last second a timestamp can hold: years 1 to 9999.
const MIN_SECONDS: i64 = -62_135_596_800;
const MAX_SECONDS: i64 = 253_402_300_799;

impl Timestamp {
    pub const UNIX_EPOCH: Timestamp = Timestamp(0);

    /// The moment `seconds` after the Unix epoch, or `None` outside the years
    /// 1 to 9999.
    pub fn from_unix_seconds(seconds: i64) -> Option<Timestamp> {
        (MIN_SECONDS..=MAX_SECONDS)
            .contains(&seconds)
            .then(|| Timestamp(seconds * TICKS_PER_SECOND))
    }

    pub fn from_unix_millis(millis: i64) -> Option<Timestamp> {
        let seconds = millis.div_euclid(1000);
        Timestamp::from_unix_seconds(seconds)
            .map(|Timestamp(ticks)| Timestamp(ticks + millis.rem_euclid(1000) * 10_000))
    }

    /// Whole seconds since the Unix epoch, rounded down.
    pub fn unix_seconds(self) -> i64 {
        self.0.div_euclid(TICKS_PER_SECOND)
    }

    /// Nanoseconds past the second.
    pub fn subsec_nanos(self) -> u32 {
        (self.0.rem_euclid(TICKS_PER_SECOND) * 100) as u32
    }

    /// Milliseconds since the Unix epoch, rounded down.
    pub fn unix_millis(self) -> i64 {
        self.0.div_euclid(10_000)
    }
}

/// Reads an ISO 8601 timestamp as Discord writes them (`2024-01-01T00:00:00+00:00`,
/// with or without a fraction of a second, or `Z`). A time without an offset
/// is UTC. Anything else is `None`: one odd date must not fail a whole page.
pub fn parse_timestamp(text: &str) -> Option<Timestamp> {
    let bytes = text.trim().as_bytes();
    let number = |from: usize, digits: usize| -> Option<i64> {
        let part = bytes.get(from..from + digits)?;
        part.iter().all(u8::is_ascii_digit).then(|| {
            part.iter()
                .fold(0i64, |sum, digit| sum * 10 + i64::from(digit - b'0'))
        })
    };

    let year = number(0, 4)?;
    if bytes.get(4) != Some(&b'-') {
        return None;
    }
    let month = number(5, 2)?;
    if bytes.get(7) != Some(&b'-') {
        return None;
    }
    let day = number(8, 2)?;
    if !(1..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
    {
        return None;
    }

    let (mut hour, mut minute, mut second, mut ticks) = (0, 0, 0, 0);
    let mut at = 10;
    if at < bytes.len() && matches!(bytes[at], b'T' | b't' | b' ') {
        hour = number(at + 1, 2)?;
        if bytes.get(at + 3) != Some(&b':') {
            return None;
        }
        minute = number(at + 4, 2)?;
        at += 6;
        if bytes.get(at) == Some(&b':') {
            second = number(at + 1, 2)?;
            at += 3;
            if bytes.get(at) == Some(&b'.') || bytes.get(at) == Some(&b',') {
                at += 1;
                let start = at;
                while at < bytes.len() && bytes[at].is_ascii_digit() {
                    at += 1;
                }
                if at == start {
                    return None;
                }
                // Seven digits are all that a tick holds; the eighth rounds.
                let mut scale = TICKS_PER_SECOND / 10;
                for &digit in &bytes[start..at.min(start + 7)] {
                    ticks += i64::from(digit - b'0') * scale;
                    scale /= 10;
                }
                if bytes
                    .get(start + 7)
                    .is_some_and(|digit| (b'5'..=b'9').contains(digit))
                    && at > start + 7
                {
                    ticks += 1;
                }
            }
        }
        if hour > 23 || minute > 59 || second > 59 {
            return None;
        }
    }

    // A space before the zone is read too.
    while at < bytes.len() && bytes[at] == b' ' {
        at += 1;
    }
    let mut offset_minutes = 0;
    match bytes.get(at) {
        None => {}
        Some(b'Z' | b'z') => at += 1,
        Some(&sign @ (b'+' | b'-')) => {
            let hours = number(at + 1, 2)?;
            at += 3;
            let mut minutes = 0;
            if bytes.get(at) == Some(&b':') {
                minutes = number(at + 1, 2)?;
                at += 3;
            } else if let Some(more) = number(at, 2) {
                minutes = more;
                at += 2;
            }
            if hours > 14 || minutes > 59 {
                return None;
            }
            offset_minutes = (hours * 60 + minutes) * if sign == b'-' { -1 } else { 1 };
        }
        Some(_) => return None,
    }
    if at != bytes.len() {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60;
    Timestamp::from_unix_seconds(seconds).map(|Timestamp(base)| Timestamp(base + ticks))
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a date of the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// When a snowflake was made: its top 42 bits are milliseconds since
/// Discord's epoch, the first moment of 2015.
pub fn snowflake_time(id: &str) -> Option<Timestamp> {
    const DISCORD_EPOCH_MILLIS: i64 = 1_420_070_400_000;
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let snowflake: u64 = id.parse().ok()?;
    Timestamp::from_unix_millis(i64::try_from(snowflake >> 22).ok()? + DISCORD_EPOCH_MILLIS)
}

/// Reads an HTTP date in any of the three forms the standard names, as a
/// duration since the Unix epoch.
pub(crate) fn parse_http_date(text: &str) -> Option<Duration> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let month = |name: &str| {
        MONTHS
            .iter()
            .position(|known| name.len() >= 3 && name[..3].eq_ignore_ascii_case(known))
            .map(|index| index as i64 + 1)
    };
    let clock = |text: &str| -> Option<i64> {
        let mut parts = text.split(':');
        let (hour, minute, second) = (
            parts.next()?.parse::<i64>().ok()?,
            parts.next()?.parse::<i64>().ok()?,
            parts.next()?.parse::<i64>().ok()?,
        );
        (parts.next().is_none()
            && (0..24).contains(&hour)
            && (0..60).contains(&minute)
            && (0..61).contains(&second))
        .then_some(hour * 3600 + minute * 60 + second)
    };

    let text = text.trim();
    let words: Vec<&str> = text.split_ascii_whitespace().collect();
    let (year, month, day, time) = match words.as_slice() {
        // Sun, 06 Nov 1994 08:49:37 GMT
        [weekday, day, month_name, year, time, zone]
            if weekday.ends_with(',') && zone.eq_ignore_ascii_case("GMT") =>
        {
            (
                year.parse::<i64>().ok()?,
                month(month_name)?,
                day.parse::<i64>().ok()?,
                clock(time)?,
            )
        }
        // Sunday, 06-Nov-94 08:49:37 GMT
        [weekday, date, time, zone]
            if weekday.ends_with(',') && zone.eq_ignore_ascii_case("GMT") =>
        {
            let mut parts = date.split('-');
            let day = parts.next()?.parse::<i64>().ok()?;
            let month_value = month(parts.next()?)?;
            let year = parts.next()?.parse::<i64>().ok()?;
            let year = if year < 100 {
                year + if year < 70 { 2000 } else { 1900 }
            } else {
                year
            };
            (year, month_value, day, clock(time)?)
        }
        // Sun Nov  6 08:49:37 1994
        [_, month_name, day, time, year] => (
            year.parse::<i64>().ok()?,
            month(month_name)?,
            day.parse::<i64>().ok()?,
            clock(time)?,
        ),
        _ => return None,
    };
    if !(1..=9999).contains(&year) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let seconds = days_from_civil(year, month, day) * 86_400 + time;
    u64::try_from(seconds).ok().map(Duration::from_secs)
}

/// What the rate limiter reads the time from and waits on. Tests give it a
/// clock they control; the real one is [`SystemClock`].
pub trait Clock: Send + Sync {
    /// The time now, since the Unix epoch.
    fn now(&self) -> Duration;
    /// Resolves after `duration` of this clock's time.
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// The wall clock, waiting with the runtime's timers.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(duration))
    }
}
