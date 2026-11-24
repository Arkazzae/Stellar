//! The rule a TZif file ends with: a POSIX time zone string such as
//! `CET-1CEST,M3.5.0,M10.5.0/3`, which says what the zone does in every
//! year after the last recorded transition.

use crate::civil::{civil_from_days, days_from_civil, days_in_month, is_leap, weekday_of_days};

/// One local time type: its offset from UTC, whether it is daylight saving
/// time, and its abbreviation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalType {
    pub offset: i32,
    pub dst: bool,
    pub abbreviation: String,
}

/// The day a transition happens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Day {
    /// `Mm.w.d`: day `d` (0 is Sunday) of week `w` (5 is the last) of month `m`.
    Month { month: u32, week: u32, weekday: u32 },
    /// `Jn`: day 1 to 365 of the year, February 29 never counted.
    Julian(u32),
    /// `n`: day 0 to 365 of the year, February 29 counted.
    Zero(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Change {
    day: Day,
    /// Seconds after midnight of local time, which may be negative or past a
    /// day (the extension of version 3 files).
    time: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rule {
    Fixed(LocalType),
    Daylight {
        standard: LocalType,
        daylight: LocalType,
        start: Change,
        end: Change,
    },
}

impl Rule {
    /// Reads the string; `None` when it is not a well-formed rule.
    pub(crate) fn parse(text: &str) -> Option<Rule> {
        let mut reader = Reader {
            rest: text.as_bytes(),
        };
        let standard_name = reader.name()?;
        let standard_offset = -reader.offset(24)?;
        let standard = LocalType {
            offset: standard_offset,
            dst: false,
            abbreviation: standard_name,
        };
        if reader.rest.is_empty() {
            return Some(Rule::Fixed(standard));
        }
        let daylight_name = reader.name()?;
        let daylight_offset = if matches!(reader.rest.first(), Some(b',') | None) {
            standard_offset.checked_add(3600)?
        } else {
            -reader.offset(24)?
        };
        let daylight = LocalType {
            offset: daylight_offset,
            dst: true,
            abbreviation: daylight_name,
        };
        // A string with daylight saving and no dates is left to the system's
        // default in other readers; here it is refused, as the file format
        // says the dates must be there.
        reader.expect(b',')?;
        let start = reader.change()?;
        reader.expect(b',')?;
        let end = reader.change()?;
        if !reader.rest.is_empty() {
            return None;
        }
        Some(Rule::Daylight {
            standard,
            daylight,
            start,
            end,
        })
    }

    /// The local time type in force at `moment`, seconds since the epoch.
    pub(crate) fn at(&self, moment: i64) -> &LocalType {
        let Rule::Daylight {
            standard,
            daylight,
            start,
            end,
        } = self
        else {
            return match self {
                Rule::Fixed(fixed) => fixed,
                Rule::Daylight { standard, .. } => standard,
            };
        };
        // Far outside any calendar a person uses; the clamp keeps the
        // arithmetic from overflowing.
        let moment = moment.clamp(-(1 << 42), 1 << 42);
        let (year, _, _) =
            civil_from_days((moment + i64::from(standard.offset)).div_euclid(86_400));
        let mut events: Vec<(i64, bool)> = Vec::with_capacity(6);
        for candidate in year - 1..=year + 1 {
            events.push((instant(candidate, start, standard.offset), true));
            events.push((instant(candidate, end, daylight.offset), false));
        }
        events.sort_unstable();
        match events.iter().rev().find(|(at, _)| *at <= moment) {
            Some((_, true)) => daylight,
            _ => standard,
        }
    }
}

/// The UTC moment of a change in `year`, given the offset in force before it.
fn instant(year: i64, change: &Change, offset_before: i32) -> i64 {
    let day = match change.day {
        Day::Month {
            month,
            week,
            weekday,
        } => {
            let first = days_from_civil(year, month, 1);
            let ahead = (weekday + 7 - weekday_of_days(first)) % 7;
            let mut day = first + i64::from(ahead) + i64::from(week - 1) * 7;
            let length = i64::from(days_in_month(year, month));
            while day >= first + length {
                day -= 7;
            }
            day
        }
        Day::Julian(n) => {
            let skip = i64::from(is_leap(year) && n >= 60);
            days_from_civil(year, 1, 1) + i64::from(n) - 1 + skip
        }
        Day::Zero(n) => days_from_civil(year, 1, 1) + i64::from(n),
    };
    day * 86_400 + i64::from(change.time) - i64::from(offset_before)
}

struct Reader<'a> {
    rest: &'a [u8],
}

impl Reader<'_> {
    fn expect(&mut self, byte: u8) -> Option<()> {
        if self.rest.first() == Some(&byte) {
            self.rest = &self.rest[1..];
            Some(())
        } else {
            None
        }
    }

    /// An abbreviation: three or more letters, or any run of letters,
    /// digits, `+` and `-` between `<` and `>`.
    fn name(&mut self) -> Option<String> {
        let (name, rest) = if self.rest.first() == Some(&b'<') {
            let close = self.rest.iter().position(|&b| b == b'>')?;
            let name = &self.rest[1..close];
            if !name
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'-')
            {
                return None;
            }
            (name, &self.rest[close + 1..])
        } else {
            let length = self
                .rest
                .iter()
                .take_while(|b| b.is_ascii_alphabetic())
                .count();
            (&self.rest[..length], &self.rest[length..])
        };
        if name.len() < 3 {
            return None;
        }
        self.rest = rest;
        Some(String::from_utf8_lossy(name).into_owned())
    }

    fn number(&mut self, max: u32) -> Option<u32> {
        let length = self.rest.iter().take_while(|b| b.is_ascii_digit()).count();
        if length == 0 || length > 3 {
            return None;
        }
        let value: u32 = std::str::from_utf8(&self.rest[..length])
            .ok()?
            .parse()
            .ok()?;
        if value > max {
            return None;
        }
        self.rest = &self.rest[length..];
        Some(value)
    }

    /// `[+-]hh[:mm[:ss]]` in seconds; the hours may go up to `max_hours`.
    fn offset(&mut self, max_hours: u32) -> Option<i32> {
        let sign = match self.rest.first() {
            Some(b'-') => {
                self.rest = &self.rest[1..];
                -1
            }
            Some(b'+') => {
                self.rest = &self.rest[1..];
                1
            }
            _ => 1,
        };
        let hours = self.number(max_hours)?;
        let mut seconds = hours * 3600;
        if self.rest.first() == Some(&b':') {
            self.rest = &self.rest[1..];
            seconds += self.number(59)? * 60;
            if self.rest.first() == Some(&b':') {
                self.rest = &self.rest[1..];
                seconds += self.number(59)?;
            }
        }
        Some(sign * i32::try_from(seconds).ok()?)
    }

    fn change(&mut self) -> Option<Change> {
        let day = match self.rest.first()? {
            b'M' => {
                self.rest = &self.rest[1..];
                let month = self.number(12).filter(|m| *m >= 1)?;
                self.expect(b'.')?;
                let week = self.number(5).filter(|w| *w >= 1)?;
                self.expect(b'.')?;
                let weekday = self.number(6)?;
                Day::Month {
                    month,
                    week,
                    weekday,
                }
            }
            b'J' => {
                self.rest = &self.rest[1..];
                Day::Julian(self.number(365).filter(|n| *n >= 1)?)
            }
            _ => Day::Zero(self.number(365)?),
        };
        let time = if self.rest.first() == Some(&b'/') {
            self.rest = &self.rest[1..];
            self.offset(167)?
        } else {
            7200
        };
        Some(Change { day, time })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rule: &str, year: i64, month: u32, day: u32, hour: i64) -> (i32, String) {
        let rule = Rule::parse(rule).unwrap();
        let moment = days_from_civil(year, month, day) * 86_400 + hour * 3600;
        let local = rule.at(moment);
        (local.offset, local.abbreviation.clone())
    }

    #[test]
    fn a_fixed_rule_never_changes() {
        assert_eq!(at("JST-9", 2030, 7, 1, 0), (32_400, "JST".into()));
    }

    #[test]
    fn the_european_rule_changes_at_one_oclock_utc() {
        let rule = "CET-1CEST,M3.5.0,M10.5.0/3";
        assert_eq!(at(rule, 2026, 3, 29, 0), (3600, "CET".into()));
        assert_eq!(at(rule, 2026, 3, 29, 1), (7200, "CEST".into()));
        assert_eq!(at(rule, 2026, 10, 25, 0), (7200, "CEST".into()));
        assert_eq!(at(rule, 2026, 10, 25, 1), (3600, "CET".into()));
    }

    #[test]
    fn a_southern_rule_spans_the_new_year() {
        let rule = "AEST-10AEDT,M10.1.0,M4.1.0/3";
        assert_eq!(at(rule, 2030, 1, 15, 0), (39_600, "AEDT".into()));
        assert_eq!(at(rule, 2030, 7, 15, 0), (36_000, "AEST".into()));
        assert_eq!(at(rule, 2030, 12, 31, 23), (39_600, "AEDT".into()));
    }

    #[test]
    fn a_quoted_name_and_an_explicit_daylight_offset_are_read() {
        let rule = Rule::parse("<-03>3<-02>2,M3.5.0/-2,M10.5.0/-1").unwrap();
        let Rule::Daylight {
            standard, daylight, ..
        } = rule
        else {
            panic!("a rule with daylight saving");
        };
        assert_eq!(standard.offset, -10_800);
        assert_eq!(standard.abbreviation, "-03");
        assert_eq!(daylight.offset, -7_200);
    }

    #[test]
    fn day_forms_other_than_month_week_day_are_read() {
        let rule = "EST5EDT,J60,300/1";
        assert_eq!(at(rule, 2027, 3, 1, 12), (-14_400, "EDT".into()));
        // In a leap year J60 is March 1, not February 29.
        assert_eq!(at(rule, 2028, 2, 29, 12), (-18_000, "EST".into()));
        assert_eq!(at(rule, 2028, 3, 1, 12), (-14_400, "EDT".into()));
    }

    #[test]
    fn a_malformed_rule_is_refused() {
        for text in [
            "",
            "A1",
            "CET",
            "CET-1CEST",
            "CET-1CEST,M3.5.0",
            "CET-1CEST,M13.1.0,M10.5.0",
            "CET-1CEST,M3.6.0,M10.5.0",
            "CET-1CEST,M3.5.7,M10.5.0",
            "CET-1CEST,J0,J5",
            "CET-1CEST,M3.5.0,M10.5.0,",
            "CET-25",
            "<CET-1",
            "CET-1:60",
            "CET-1CEST,M3.5.0/168,M10.5.0",
            "CET-9999999999",
        ] {
            assert_eq!(Rule::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn extreme_moments_do_not_overflow() {
        let rule = Rule::parse("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        for moment in [i64::MIN, i64::MAX, 0] {
            let _ = rule.at(moment);
        }
    }
}
