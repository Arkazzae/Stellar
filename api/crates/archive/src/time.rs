//! Zip's two ways of keeping a time: the DOS date and time of the headers
//! (two-second steps, 1980 to 2107, no zone) and the Unix seconds of the
//! extended-timestamp field. Both are read and written as UTC.

const UNIX_1980: i64 = 315_532_800;
/// 2107-12-31 23:59:58, the last time a DOS date can hold.
const UNIX_2107: i64 = 4_354_819_198;

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
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

/// `(time, date)` as the headers keep them. Times before 1980 or after 2107
/// are held at the nearest end.
pub(crate) fn dos_from_unix(seconds: i64) -> (u16, u16) {
    let seconds = seconds.clamp(UNIX_1980, UNIX_2107);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let in_day = seconds.rem_euclid(86_400);
    let time = ((in_day / 3600) << 11) | (((in_day % 3600) / 60) << 5) | ((in_day % 60) / 2);
    let date = ((year - 1980) << 9) | (month << 5) | day;
    (time as u16, date as u16)
}

/// The Unix seconds a header's DOS time and date stand for. Fields that are
/// not a real date are read as the start of 1980.
pub(crate) fn unix_from_dos(time: u16, date: u16) -> i64 {
    let year = 1980 + i64::from(date >> 9);
    let month = i64::from((date >> 5) & 15);
    let day = i64::from(date & 31);
    let (hour, minute, second) = (
        i64::from(time >> 11),
        i64::from((time >> 5) & 63),
        i64::from(time & 31) * 2,
    );
    if !(1..=12).contains(&month) || day == 0 || hour > 23 || minute > 59 || second > 59 {
        return UNIX_1980;
    }
    days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_on_even_seconds() {
        for seconds in [
            UNIX_1980,
            951_782_400,
            1_700_000_000,
            1_700_000_002,
            4_102_444_800,
            UNIX_2107,
        ] {
            let (time, date) = dos_from_unix(seconds);
            assert_eq!(
                unix_from_dos(time, date),
                seconds - seconds % 2,
                "{seconds}"
            );
        }
    }

    #[test]
    fn out_of_range_is_held_at_the_ends() {
        assert_eq!(dos_from_unix(0), dos_from_unix(UNIX_1980));
        assert_eq!(dos_from_unix(i64::MAX), dos_from_unix(UNIX_2107));
        assert_eq!(dos_from_unix(i64::MIN), dos_from_unix(UNIX_1980));
    }

    #[test]
    fn known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
        // 2024-02-29 12:34:56 UTC
        let (time, date) = dos_from_unix(1_709_210_096);
        assert_eq!((date >> 9) + 1980, 2024);
        assert_eq!((date >> 5) & 15, 2);
        assert_eq!(date & 31, 29);
        assert_eq!(time >> 11, 12);
    }

    #[test]
    fn garbage_fields_do_not_panic() {
        for time in [0u16, 0xffff, 0x7bef] {
            for date in [0u16, 0xffff, 0x0021, 0x7d21] {
                let _ = unix_from_dos(time, date);
            }
        }
    }
}
