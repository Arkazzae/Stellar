//! Reading the moments a manifest and a request carry.

use crate::values::Timestamp;

const TICKS_PER_SECOND: i64 = 10_000_000;
/// The most an offset from UTC may be: fourteen hours.
const MAX_OFFSET_SECONDS: i64 = 14 * 3600;
/// The longest fraction of a second that is accepted, though only seven
/// digits of it count.
const MAX_FRACTION_DIGITS: usize = 16;

/// Accepted forms for a manifest's `publishedAt`:
/// `2026-11-20`, `2026-11-20T10:11`, `2026-11-20T10:11:12`, with a fraction of
/// up to sixteen digits, and with `Z` or an offset of `+hh` or `+hh:mm`. A
/// moment without a zone is UTC. The first moment of year 1 stands for "no
/// moment" and is refused.
pub fn parse_published_at(text: &str) -> Option<Timestamp> {
    let bytes = text.as_bytes();
    let digits = |from: usize, count: usize| -> Option<u32> {
        let part = bytes.get(from..from + count)?;
        part.iter().all(u8::is_ascii_digit).then(|| {
            part.iter()
                .fold(0, |acc, digit| acc * 10 + u32::from(digit - b'0'))
        })
    };
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let (year, month, day) = (digits(0, 4)?, digits(5, 2)?, digits(8, 2)?);
    let (mut hour, mut minute, mut second) = (0, 0, 0);
    let mut fraction = String::new();
    let mut offset_seconds = 0;
    if bytes.len() > 10 {
        if bytes[10] != b'T' || bytes.get(13) != Some(&b':') {
            return None;
        }
        hour = digits(11, 2)?;
        minute = digits(14, 2)?;
        let mut at = 16;
        if bytes.get(at) == Some(&b':') {
            second = digits(at + 1, 2)?;
            at += 3;
            if bytes.get(at) == Some(&b'.') {
                at += 1;
                let start = at;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
                let count = at - start;
                // A point with nothing after it is only fine where a zone follows.
                if count > MAX_FRACTION_DIGITS || (count == 0 && at == bytes.len()) {
                    return None;
                }
                fraction = text[start..at].chars().take(7).collect();
            }
        }
        match bytes.get(at) {
            None => {}
            Some(b'Z') => at += 1,
            Some(sign @ (b'+' | b'-')) => {
                let hours = digits(at + 1, 2)?;
                let minutes = if bytes.get(at + 3) == Some(&b':') {
                    at += 6;
                    digits(at - 2, 2)?
                } else {
                    at += 3;
                    0
                };
                if minutes > 59 {
                    return None;
                }
                let seconds = i64::from(hours) * 3600 + i64::from(minutes) * 60;
                if seconds > MAX_OFFSET_SECONDS {
                    return None;
                }
                offset_seconds = if *sign == b'-' { -seconds } else { seconds };
            }
            Some(_) => return None,
        }
        if at != bytes.len() {
            return None;
        }
    }
    // The calendar and the clock are checked by the reader of stored moments.
    let normalized =
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{fraction:0<7}");
    let local = Timestamp::parse(&normalized)?;
    let moment = Timestamp::from_ticks(local.ticks() - offset_seconds * TICKS_PER_SECOND)?;
    let year_one = Timestamp::parse("0001-01-01T00:00:00")?;
    (moment != year_one).then_some(moment)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Option<String> {
        parse_published_at(text).map(Timestamp::to_wire_text)
    }

    #[test]
    fn dates_times_and_supported_offsets_are_accepted() {
        for (given, expected) in [
            ("2026-11-20", "2026-11-20T00:00:00Z"),
            ("2026-11-20T10:11", "2026-11-20T10:11:00Z"),
            ("2026-11-20T10:11Z", "2026-11-20T10:11:00Z"),
            ("2026-11-20T10:11:12", "2026-11-20T10:11:12Z"),
            ("2026-11-20T10:11:12Z", "2026-11-20T10:11:12Z"),
            ("2026-11-20T10:11:12.1Z", "2026-11-20T10:11:12.1Z"),
            ("2026-11-20T10:11:12.12345678Z", "2026-11-20T10:11:12.1234567Z"),
            ("2026-11-20T10:11:12.1111111111111111Z", "2026-11-20T10:11:12.1111111Z"),
            ("2026-11-20T10:11:12.99999995Z", "2026-11-20T10:11:12.9999999Z"),
            ("2026-11-20T10:11:12.00000001Z", "2026-11-20T10:11:12Z"),
            ("2026-11-20T10:11:12.5", "2026-11-20T10:11:12.5Z"),
            ("2026-11-20T10:11:12.Z", "2026-11-20T10:11:12Z"),
            ("2026-11-20T10:11:12.+01", "2026-11-20T09:11:12Z"),
            ("2026-11-20T10:11:12+01:00", "2026-11-20T09:11:12Z"),
            ("2026-11-20T10:11:12+01", "2026-11-20T09:11:12Z"),
            ("2026-11-20T10:11:12-01:00", "2026-11-20T11:11:12Z"),
            ("2026-11-20T10:11:12-00:00", "2026-11-20T10:11:12Z"),
            ("2026-11-20T10:11:12+14:00", "2026-11-19T20:11:12Z"),
            ("2024-02-29T00:00:00Z", "2024-02-29T00:00:00Z"),
            ("9999-12-31T23:59:59.9999999Z", "9999-12-31T23:59:59.9999999Z"),
            ("9999-12-31T23:59:59+01:00", "9999-12-31T22:59:59Z"),
            ("0001-01-01T00:00:00-01:00", "0001-01-01T01:00:00Z"),
            ("0001-01-02T00:00:00+01:00", "0001-01-01T23:00:00Z"),
            ("0001-01-01T00:00:00.0000001Z", "0001-01-01T00:00:00.0000001Z"),
        ] {
            assert_eq!(read(given).as_deref(), Some(expected), "{given}");
        }
    }

    #[test]
    fn invalid_and_unsupported_timestamps_are_rejected() {
        for given in [
            "",
            "2026-11-20T10",
            "2026-11-20T10:1",
            "2026-11-2T10:11:12Z",
            "26-11-20T10:11:12Z",
            "2026-11-20 10:11:12",
            "2026-11-20t10:11:12z",
            "2026-11-20T10:11:12,5Z",
            "2026-11-20T24:00:00Z",
            "2026-11-20T10:60:00Z",
            "2026-11-20T10:11:60Z",
            "2026-02-30T10:11:12Z",
            "2023-02-29T00:00:00Z",
            "0000-01-01T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "0001-01-01T00:00:00Z",
            "0001-01-01T00:00:00+00:00",
            "0001-01-01T01:00:00+02:00",
            "9999-12-31T23:59:59-01:00",
            "2026-11-20T10:11:12-0100",
            "2026-11-20T10:11:12+1:00",
            "2026-11-20T10:11:12+01:0",
            "2026-11-20T10:11:12+01:60",
            "2026-11-20T10:11:12+14:01",
            "2026-11-20T10:11:12+15:00",
            "2026-11-20T10:11:12+25:00",
            "2026-11-20T10:11:12.",
            "2026-11-20T10:11.5Z",
            "2026-11-20T10:11:12ZZ",
            "2026-11-20T10:11:12 Z",
            "2026-11-20T10:11:12.12a",
            "2026-11-20T10:11:12.11111111111111111Z",
            "2026-11-20Z",
            "2026-11-20+01:00",
            "2026-11-20T",
            " 2026-11-20T10:11:12Z",
            "20261120T101112Z",
            "Fri, 20 Nov 2026 10:11:12 GMT",
            "2026-W47-5",
            "２026-11-20",
        ] {
            assert_eq!(read(given), None, "{given:?}");
        }
    }
}
