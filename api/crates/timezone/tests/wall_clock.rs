//! From a zone's clocks back to the moment: the hour skipped and the hour
//! repeated around a change of daylight saving time, and dates moved by weeks
//! and months that keep the time on the clock.

use std::path::Path;

use stellarr_timezone::TimeZone;

fn warsaw() -> TimeZone {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/zoneinfo");
    TimeZone::find_in(&directory, "Europe/Warsaw").unwrap()
}

/// A moment written in UTC.
fn utc(year: i64, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    TimeZone::utc().from_local(year, month, day, hour, minute, 0)
}

#[test]
fn a_clock_time_is_the_moment_its_zone_shows_it() {
    let zone = warsaw();
    assert_eq!(zone.from_local(2026, 11, 28, 20, 0, 0), utc(2026, 11, 28, 19, 0), "winter, an hour ahead");
    assert_eq!(zone.from_local(2026, 7, 4, 20, 0, 0), utc(2026, 7, 4, 18, 0), "summer, two hours ahead");
    // The clocks go from 02:00 to 03:00 on 28 March 2027: 02:30 is not shown,
    // and is taken as 03:30.
    assert_eq!(zone.from_local(2027, 3, 28, 2, 30, 0), utc(2027, 3, 28, 1, 30));
    // They go back from 03:00 to 02:00 on 25 October 2026: 02:30 is shown
    // twice, and the first is taken.
    assert_eq!(zone.from_local(2026, 10, 25, 2, 30, 0), utc(2026, 10, 25, 0, 30));
    for moment in (utc(2026, 1, 1, 0, 0)..utc(2027, 1, 1, 0, 0)).step_by(3_607 * 7) {
        let shown = zone.local_time(moment);
        let back = zone.from_local(shown.year, shown.month, shown.day, shown.hour, shown.minute, shown.second);
        // Only the repeated hour has a second moment, and the first is taken.
        assert!(back == moment || back == moment - 3600, "{moment}");
    }
}

#[test]
fn a_date_moved_by_weeks_or_months_keeps_its_time_on_the_clock() {
    let zone = warsaw();
    // Saturday 20:00 before the clocks go back, and the Saturday after.
    let saturday = zone.from_local(2026, 10, 24, 20, 0, 0);
    assert_eq!(saturday, utc(2026, 10, 24, 18, 0));
    assert_eq!(zone.shifted(saturday, 0, 7), utc(2026, 10, 31, 19, 0));
    assert_eq!(zone.shifted(saturday, 0, 14), utc(2026, 11, 7, 19, 0));
    // The 31st, month by month.
    let last = zone.from_local(2027, 1, 31, 20, 0, 0);
    assert_eq!(zone.shifted(last, 1, 0), utc(2027, 2, 28, 19, 0));
    assert_eq!(zone.shifted(last, 2, 0), utc(2027, 3, 31, 18, 0), "back to the 31st, now in summer time");
    assert_eq!(zone.shifted(last, 13, 0), utc(2028, 2, 29, 19, 0), "a leap year");
    // A time the clocks skip a week on.
    let early = zone.from_local(2027, 3, 21, 2, 30, 0);
    assert_eq!(zone.shifted(early, 0, 7), utc(2027, 3, 28, 1, 30));
}
