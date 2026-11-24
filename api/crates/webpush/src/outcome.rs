//! What the push service's answer means for the sender.

use std::time::Duration;

/// The meaning of one answer from a push service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The push service accepted the message (any 2xx).
    Delivered,
    /// The subscription no longer exists (404, 410): drop it.
    Gone,
    /// Not now: a timeout, a throttle or an outage. `after` is what the
    /// service asked for in `Retry-After`, if it did.
    RetryLater { after: Option<Duration> },
    /// The service refused this message for a reason a retry does not change
    /// (a bad request, a rejected token, a too large body, a redirect).
    Rejected { status: u16 },
}

/// The three-way answer the outbox that owns the retries works with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    Delivered,
    /// The subscription is dead; delete it.
    Gone,
    /// Keep the subscription and try again under the outbox's own limits.
    Retry,
}

impl Outcome {
    /// Reduces the outcome to the outbox's three answers.
    ///
    /// A refusal other than "gone" never deletes the subscription. A rejected
    /// token or a rejected body is more often a mistake on our side than the
    /// browser's, so it is retried, and the outbox bounds the attempts instead
    /// of dropping a working browser.
    pub fn dispatch(self) -> Dispatch {
        match self {
            Self::Delivered => Dispatch::Delivered,
            Self::Gone => Dispatch::Gone,
            Self::RetryLater { .. } | Self::Rejected { .. } => Dispatch::Retry,
        }
    }
}

/// Classifies the status of an answer and its `Retry-After` header, if any.
///
/// `now` is the current time in seconds since the Unix epoch, needed only to
/// read a `Retry-After` that is a date. A request that never got an answer
/// (no connection, a timeout) is the caller's to treat as
/// [`Outcome::RetryLater`] with no delay.
pub fn classify(status: u16, retry_after: Option<&str>, now: u64) -> Outcome {
    match status {
        200..=299 => Outcome::Delivered,
        404 | 410 => Outcome::Gone,
        408 | 429 | 500..=599 => Outcome::RetryLater {
            after: retry_after.and_then(|value| parse_retry_after(value, now)),
        },
        _ => Outcome::Rejected { status },
    }
}

/// Reads `Retry-After` (RFC 9110 section 10.2.3): a number of seconds, or an
/// HTTP date in the fixed `Sun, 06 Nov 1994 08:49:37 GMT` form. A date in the
/// past means "now". Anything else is ignored.
pub fn parse_retry_after(value: &str, now: u64) -> Option<Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        // More digits than a u64 holds still mean "a very long time".
        let seconds = value.parse::<u64>().unwrap_or(u64::MAX);
        return Some(Duration::from_secs(seconds));
    }
    let at = parse_http_date(value)?;
    Some(Duration::from_secs(at.saturating_sub(now)))
}

/// Seconds since the epoch for an IMF-fixdate, or `None`.
fn parse_http_date(value: &str) -> Option<u64> {
    let mut parts = value.split(' ');
    let weekday = parts.next()?;
    let day = parts.next()?;
    let month = parts.next()?;
    let year = parts.next()?;
    let time = parts.next()?;
    let zone = parts.next()?;
    if parts.next().is_some() || zone != "GMT" {
        return None;
    }
    let weekday = weekday.strip_suffix(',')?;
    if !["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].contains(&weekday) {
        return None;
    }
    let day = two_digits(day)?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|&name| name == month)? as i64
        + 1;
    if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i64 = year.parse().ok()?;
    let mut clock = time.split(':');
    let hour = two_digits(clock.next()?)?;
    let minute = two_digits(clock.next()?)?;
    let second = two_digits(clock.next()?)?;
    if clock.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let month_days = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day < 1 || day > month_days[month as usize - 1] {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(seconds).ok()
}

fn two_digits(text: &str) -> Option<i64> {
    (text.len() == 2 && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_the_dispatcher_was_tested_with_keeps_its_decision() {
        let table = [
            (200, Dispatch::Delivered),
            (201, Dispatch::Delivered),
            (204, Dispatch::Delivered),
            (400, Dispatch::Retry),
            (302, Dispatch::Retry),
            (404, Dispatch::Gone),
            (410, Dispatch::Gone),
            (413, Dispatch::Retry),
            (401, Dispatch::Retry),
            (403, Dispatch::Retry),
            (408, Dispatch::Retry),
            (429, Dispatch::Retry),
            (500, Dispatch::Retry),
            (503, Dispatch::Retry),
        ];
        for (status, expected) in table {
            assert_eq!(classify(status, None, 0).dispatch(), expected, "{status}");
        }
    }

    #[test]
    fn every_status_in_range_has_one_decision() {
        for status in 0..=999u16 {
            let dispatch = classify(status, None, 0).dispatch();
            let expected = match status {
                200..=299 => Dispatch::Delivered,
                404 | 410 => Dispatch::Gone,
                _ => Dispatch::Retry,
            };
            assert_eq!(dispatch, expected, "{status}");
        }
    }

    #[test]
    fn outcomes_tell_a_wait_from_a_refusal() {
        assert_eq!(classify(201, None, 0), Outcome::Delivered);
        assert_eq!(classify(410, None, 0), Outcome::Gone);
        assert_eq!(classify(404, Some("30"), 0), Outcome::Gone);
        assert_eq!(classify(429, None, 0), Outcome::RetryLater { after: None });
        assert_eq!(classify(408, None, 0), Outcome::RetryLater { after: None });
        assert_eq!(classify(502, None, 0), Outcome::RetryLater { after: None });
        assert_eq!(classify(400, None, 0), Outcome::Rejected { status: 400 });
        assert_eq!(classify(401, None, 0), Outcome::Rejected { status: 401 });
        assert_eq!(classify(403, None, 0), Outcome::Rejected { status: 403 });
        assert_eq!(classify(413, None, 0), Outcome::Rejected { status: 413 });
        assert_eq!(classify(302, None, 0), Outcome::Rejected { status: 302 });
    }

    #[test]
    fn retry_after_in_seconds_rides_along() {
        assert_eq!(
            classify(429, Some("120"), 0),
            Outcome::RetryLater {
                after: Some(Duration::from_secs(120))
            }
        );
        assert_eq!(
            classify(503, Some(" 7 "), 0),
            Outcome::RetryLater {
                after: Some(Duration::from_secs(7))
            }
        );
        assert_eq!(
            classify(503, Some("0"), 0),
            Outcome::RetryLater {
                after: Some(Duration::ZERO)
            }
        );
    }

    #[test]
    fn retry_after_as_a_date_counts_from_now() {
        // 1994-11-06T08:49:37Z is 784111777; the date is the RFC 9110 example.
        let date = "Sun, 06 Nov 1994 08:49:37 GMT";
        assert_eq!(parse_http_date(date), Some(784_111_777));
        assert_eq!(
            parse_retry_after(date, 784_111_777 - 90),
            Some(Duration::from_secs(90))
        );
        assert_eq!(
            parse_retry_after(date, 784_111_777 + 5),
            Some(Duration::ZERO)
        );
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            parse_http_date("Tue, 29 Feb 2028 12:00:00 GMT"),
            Some(1_835_438_400)
        );
    }

    #[test]
    fn nonsense_retry_after_is_ignored() {
        for value in [
            "",
            " ",
            "-5",
            "1.5",
            "1e3",
            "soon",
            "+5",
            "0x10",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
            "Sun, 06 Nov 1994 08:49:37 UTC",
            "Sun, 06 Nov 1994 25:49:37 GMT",
            "Sun, 31 Nov 1994 08:49:37 GMT",
            "Sun, 29 Feb 2027 08:49:37 GMT",
            "Sun, 06 Nov 94 08:49:37 GMT",
            "Xyz, 06 Nov 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:37 GMT extra",
            "Sun, 06 Nov 1994 08:49 GMT",
        ] {
            assert_eq!(parse_retry_after(value, 0), None, "{value:?}");
        }
    }

    #[test]
    fn a_huge_number_of_seconds_does_not_overflow() {
        assert_eq!(
            parse_retry_after("99999999999999999999999999", 0),
            Some(Duration::from_secs(u64::MAX))
        );
    }
}
