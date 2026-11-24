//! Reading the dates a request carries in its conditions, as the first
//! server's header parser did: the three spellings HTTP has had, with a day
//! of the week that has to be the right one, whatever the letter case of the
//! names and however much white space lies between the parts.

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const LONG_DAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    Word(&'a str),
    Number(&'a str),
    Mark(char),
}

/// The words, numbers and marks (`,` `:` `-` `+`) of the text; anything else
/// is not a date.
fn tokens(text: &str) -> Option<Vec<Token<'_>>> {
    let mut found = Vec::new();
    let mut rest = text.trim_matches(|c: char| c == ' ' || c == '\t');
    while let Some(first) = rest.chars().next() {
        let length = if first.is_ascii_alphabetic() {
            rest.bytes().take_while(u8::is_ascii_alphabetic).count()
        } else if first.is_ascii_digit() {
            rest.bytes().take_while(u8::is_ascii_digit).count()
        } else if matches!(first, ',' | ':' | '-' | '+') {
            1
        } else if first == ' ' || first == '\t' {
            rest = rest.trim_start_matches([' ', '\t']);
            continue;
        } else {
            return None;
        };
        let (token, after) = rest.split_at(length);
        found.push(if first.is_ascii_alphabetic() {
            Token::Word(token)
        } else if first.is_ascii_digit() {
            Token::Number(token)
        } else {
            Token::Mark(first)
        });
        rest = after;
    }
    Some(found)
}

/// Seconds since 1970-01-01T00:00:00Z.
pub fn parse(text: &str) -> Option<i64> {
    use Token::{Mark, Number, Word};
    let tokens = tokens(text)?;
    let number = |token: &Token, least: usize, most: usize| match token {
        Number(digits) if (least..=most).contains(&digits.len()) => digits.parse::<i64>().ok(),
        _ => None,
    };
    let clock = |at: &[Token]| -> Option<(i64, i64, i64)> {
        match at {
            [hour, Mark(':'), minute, Mark(':'), second] => {
                Some((number(hour, 1, 2)?, number(minute, 1, 2)?, number(second, 1, 2)?))
            }
            _ => None,
        }
    };
    match tokens.as_slice() {
        // `Fri Nov 20 10:11:12 2026`, the form C's asctime writes.
        [Word(day), Word(month), date, clock_and_year @ ..] if clock_and_year.len() == 6 => {
            let (time, year) = clock_and_year.split_at(5);
            let (hour, minute, second) = clock(time)?;
            moment(
                Some((day, false)),
                number(&year[0], 4, 4)?,
                month_of(month)?,
                number(date, 1, 2)?,
                (hour, minute, second),
                0,
            )
        }
        // `Friday, 20-Nov-26 10:11:12 GMT`, the form HTTP once had for older
        // clients, whose year has two digits.
        [Word(day), Mark(','), date, Mark('-'), Word(month), Mark('-'), year, rest @ ..] => {
            let (time, zone) = rest.split_at(rest.len().min(5));
            let offset = match zone {
                [] => 0,
                [Word("GMT")] => 0,
                _ => return None,
            };
            moment(
                Some((day, true)),
                two_digit_year(number(year, 2, 2)?),
                month_of(month)?,
                number(date, 1, 2)?,
                clock(time)?,
                offset,
            )
        }
        // `Fri, 20 Nov 2026 10:11:12 GMT`, with or without the day name and
        // the zone, and with a year of two digits.
        _ => {
            let (day, rest) = match tokens.as_slice() {
                [Word(day), Mark(','), rest @ ..] => (Some((day, false)), rest),
                rest => (None, rest),
            };
            let [date, Word(month), year, rest @ ..] = rest else {
                return None;
            };
            if rest.len() < 5 {
                return None;
            }
            let (time, zone) = rest.split_at(5);
            let year = match year {
                Number(digits) if digits.len() == 4 => number(year, 4, 4)?,
                Number(_) => two_digit_year(number(year, 2, 2)?),
                _ => return None,
            };
            let offset = match zone {
                [] | [Word("GMT")] => 0,
                [Mark(sign @ ('+' | '-')), Number(digits)] if digits.len() == 4 => {
                    offset_of(sign, digits[..2].parse().ok()?, digits[2..].parse().ok()?)?
                }
                [Mark(sign @ ('+' | '-')), Number(hours), Mark(':'), Number(minutes)]
                    if hours.len() == 2 && minutes.len() == 2 =>
                {
                    offset_of(sign, hours.parse().ok()?, minutes.parse().ok()?)?
                }
                _ => return None,
            };
            moment(
                day,
                year,
                month_of(month)?,
                number(date, 1, 2)?,
                clock(time)?,
                offset,
            )
        }
    }
}

fn month_of(word: &str) -> Option<i64> {
    let lower = word.to_ascii_lowercase();
    Some(MONTHS.iter().position(|name| *name == lower)? as i64 + 1)
}

/// Years of two digits are 1950 to 2049, as the calendar the first server
/// read them with has it.
fn two_digit_year(year: i64) -> i64 {
    if year < 50 { 2000 + year } else { 1900 + year }
}

fn offset_of(sign: &char, hours: i64, minutes: i64) -> Option<i64> {
    let seconds = hours * 3600 + minutes * 60;
    (hours <= 14 && minutes < 60).then_some(if *sign == '-' { -seconds } else { seconds })
}

/// The seconds since the epoch of a moment `offset` seconds ahead of UTC. The
/// day name, abbreviated or in full as the form has it, has to be the day.
fn moment(
    day_name: Option<(&&str, bool)>,
    year: i64,
    month: i64,
    day: i64,
    (hour, minute, second): (i64, i64, i64),
    offset: i64,
) -> Option<i64> {
    let lengths = [
        31,
        if leap(year) { 29 } else { 28 },
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
    if !(1..=9999).contains(&year)
        || day < 1
        || day > lengths[(month - 1) as usize]
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if let Some((name, long)) = day_name {
        // 1970-01-01 was a Thursday.
        let weekday = (days + 3).rem_euclid(7) as usize;
        let names = if long { &LONG_DAYS } else { &DAYS };
        if name.to_ascii_lowercase() != names[weekday] {
            return None;
        }
    }
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let of_day = of_era * 365 + of_era / 4 - of_era / 100 + day_of_year;
    era * 146_097 + of_day - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fri, 20 Nov 2026 10:11:12 GMT, checked with Python's calendar.timegm.
    const MOMENT: i64 = 1_795_169_472;

    #[test]
    fn the_spellings_the_first_server_read_are_read() {
        for text in [
            "Fri, 20 Nov 2026 10:11:12 GMT",
            "fri, 20 nov 2026 10:11:12 GMT",
            "FRI, 20 NOV 2026 10:11:12 GMT",
            "Fri, 20 Nov 2026 10:11:12",
            "20 Nov 2026 10:11:12 GMT",
            "Fri, 20 Nov 26 10:11:12 GMT",
            "20 Nov 26 10:11:12 GMT",
            "Fri, 20 Nov 2026 10:11:12 +0000",
            "Fri, 20 Nov 2026 10:11:12 -0000",
            "Fri, 20 Nov 2026 10:11:12 +00:00",
            "Fri, 20 Nov 2026 11:11:12 +0100",
            "Fri, 20 Nov 2026 9:11:12 -01:00",
            "Friday, 20-Nov-26 10:11:12 GMT",
            "FRIDAY, 20-NOV-26 10:11:12 GMT",
            "Friday, 20-Nov-26 10:11:12",
            "Fri Nov 20 10:11:12 2026",
            "fri nov 20 10:11:12 2026",
            "Fri Nov  20 10:11:12 2026",
            "  Fri,  20 Nov 2026  10:11:12 GMT\t",
            "Fri,20 Nov 2026 10:11:12 GMT",
            "Fri , 20 Nov 2026 10:11:12 GMT",
        ] {
            assert_eq!(parse(text), Some(MOMENT), "{text}");
        }
        assert_eq!(parse("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(parse("Fri, 5 Nov 2026 1:2:3 GMT"), parse("Fri, 05 Nov 2026 01:02:03 GMT"));
        // Two digits are the years 1950 to 2049.
        assert_eq!(parse("Sat, 20 Nov 2049 00:00:00 GMT"), parse("Sat, 20 Nov 49 00:00:00 GMT"));
        assert_eq!(parse("Mon, 20 Nov 1950 00:00:00 GMT"), parse("Mon, 20 Nov 50 00:00:00 GMT"));
    }

    #[test]
    fn what_is_not_a_date_is_refused() {
        for text in [
            "",
            "yesterday",
            "Sat, 20 Nov 2026 10:11:12 GMT",
            "Fri, 31 Nov 2026 10:11:12 GMT",
            "Fri, 20 Nov 2026 24:11:12 GMT",
            "Fri, 20 Nov 2026 10:60:12 GMT",
            "Fri, 20 Nov 2026 10:11 GMT",
            "Fri, 20 Nov 2026 10:11:12.5 GMT",
            "Fri, 20 Nov 2026 10:11:12 gmt",
            "Fri, 20 Nov 2026 10:11:12 Gmt",
            "Fri, 20 Nov 2026 10:11:12 Z",
            "Fri, 20 Nov 2026 10:11:12 UTC",
            "Fri, 20 Nov 2026 10:11:12 PST",
            "Fri, 20 Nov 2026 10:11:12 +1",
            "Fri, 20 Nov 2026 10:11:12 +15:00",
            "Fri, 20 Nov 2026 10:11:12 +0000 (GMT)",
            "Fri, 20 Nov 2026 10:11:12 GMT;",
            "Fri, 20 Nov 2026 10:11:12 GMT extra",
            "Friday, 20 Nov 2026 10:11:12 GMT",
            "Friday, 20-Nov-2026 10:11:12 GMT",
            "Fri, 20-Nov-26 10:11:12 GMT",
            "Fri Nov 20 10:11:12 2026 GMT",
            "Fri, 020 Nov 2026 10:11:12 GMT",
            "Fri, 20 Nov 02026 10:11:12 GMT",
            "Fri, 20 November 2026 10:11:12 GMT",
            "Fri, 20 Nov 2026 010:11:12 GMT",
            "Fri, 20 Foo 2026 10:11:12 GMT",
            "Fri, 20 Nov 202 10:11:12 GMT",
            "2026-11-20T10:11:12Z",
            "1795169472",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }
}
