//! The one date format Signature Version 4 needs, from seconds since 1970.
//!
//! The caller passes the time in, so signing is a pure function and a test can
//! fix the clock.

/// A point in time, whole seconds since the Unix epoch, UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp(u64);

/// Seconds up to the last second of year 9999, so the date has four digits.
const LAST_SECOND: u64 = 253_402_300_799;

impl Timestamp {
    /// `None` for a time past the year 9999, which the format cannot spell.
    pub fn from_unix(seconds: u64) -> Option<Self> {
        (seconds <= LAST_SECOND).then_some(Self(seconds))
    }

    /// `20130524T000000Z`, the `x-amz-date` form.
    pub fn amz_date(self) -> String {
        let (year, month, day) = civil_from_days(self.0 / 86_400);
        let rest = self.0 % 86_400;
        format!(
            "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
            rest / 3600,
            rest % 3600 / 60,
            rest % 60
        )
    }

    /// `20130524`, the date part of the credential scope.
    pub fn date(self) -> String {
        self.amz_date()[..8].to_owned()
    }
}

/// Days since 1970-01-01 to a proleptic Gregorian date.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
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
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spell(seconds: u64) -> String {
        Timestamp::from_unix(seconds).unwrap().amz_date()
    }

    // The right-hand sides come from Python's calendar.timegm.
    #[test]
    fn known_instants() {
        assert_eq!(spell(0), "19700101T000000Z");
        assert_eq!(spell(1_369_353_600), "20130524T000000Z");
        assert_eq!(spell(1_440_938_160), "20150830T123600Z");
        assert_eq!(spell(951_868_799), "20000229T235959Z");
        assert_eq!(spell(4_107_542_400), "21000301T000000Z");
        assert_eq!(spell(LAST_SECOND), "99991231T235959Z");
    }

    #[test]
    fn the_date_is_the_front_of_the_stamp() {
        assert_eq!(
            Timestamp::from_unix(1_369_353_600).unwrap().date(),
            "20130524"
        );
    }

    #[test]
    fn a_time_past_year_9999_is_refused() {
        assert_eq!(Timestamp::from_unix(LAST_SECOND + 1), None);
        assert_eq!(Timestamp::from_unix(u64::MAX), None);
    }
}
