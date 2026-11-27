//! Time zones, read from the zone database the operating system keeps.
//!
//! [`TimeZone::find`] looks a zone up by its IANA name (`Europe/Warsaw`) in
//! the system's directory of zone files, which is `TZDIR` when that is set and
//! `/usr/share/zoneinfo` otherwise. The file is a TZif file (RFC 8536, every
//! version, the footer rule included). [`TimeZone::at`] says which offset
//! and abbreviation are in force at a moment given in UTC. `UTC` needs no
//! file. [`TimeZone::local`] is the zone of the machine.
//!
//! A name is untrusted input, so only the plain names of the database are
//! looked up: nothing that could leave the zone directory is ever opened.
//! Names are matched without regard to case, as the platform's own lookup
//! does, and the zone is called by the casing of the file.

mod civil;
mod posix;
mod tzif;

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use posix::LocalType;
use tzif::Tzif;

/// The longest zone name the database has is under 40 characters.
const MAX_NAME: usize = 128;
/// The largest zone file in the database is about 4 KiB.
const MAX_FILE: u64 = 1 << 20;
/// The most entries of a zone directory that are looked through.
const MAX_ENTRIES: usize = 4096;
const DEFAULT_DIRECTORY: &str = "/usr/share/zoneinfo";

/// Why a zone could not be had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No zone of that name is in the database.
    NotFound,
    /// There is a file of that name, but it is not a usable zone file.
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotFound => f.write_str("the time zone is not in the zone database"),
            Error::Invalid(why) => write!(f, "the time zone file is not valid: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// The offset and abbreviation in force at a moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offset<'a> {
    /// Seconds to add to UTC to get the local time.
    pub seconds: i32,
    /// `CET`, `CEST`, `+0545`, ...
    pub abbreviation: &'a str,
    /// Whether it is daylight saving time.
    pub daylight: bool,
}

/// A moment as the clocks of a zone show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime<'a> {
    pub year: i64,
    /// 1 to 12.
    pub month: u32,
    /// 1 to 31.
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// 0 is Sunday.
    pub weekday: u32,
    pub offset: Offset<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Utc,
    File(Box<Tzif>),
}

/// A zone: its name and what it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeZone {
    id: String,
    source: Source,
}

impl TimeZone {
    /// UTC, which needs no file.
    pub fn utc() -> TimeZone {
        TimeZone {
            id: "UTC".into(),
            source: Source::Utc,
        }
    }

    /// The zone of that name in the system's database. `UTC` is always there.
    pub fn find(id: &str) -> Result<TimeZone, Error> {
        TimeZone::find_in(&zone_directory(), id)
    }

    /// The zone of that name in the database at `directory`. The case of the
    /// name does not matter; the zone's id is the name as the database
    /// spells it. `UTC` in any case is [`TimeZone::utc`].
    pub fn find_in(directory: &Path, id: &str) -> Result<TimeZone, Error> {
        if id.eq_ignore_ascii_case("UTC") {
            return Ok(TimeZone::utc());
        }
        if !is_plain_name(id) {
            return Err(Error::NotFound);
        }
        let spelled = spell_as_database(directory, id).ok_or(Error::NotFound)?;
        TimeZone::read_named(directory, &spelled)
    }

    /// Like [`TimeZone::find_in`], but the name must be spelled exactly as
    /// the database spells it.
    fn find_exact_in(directory: &Path, id: &str) -> Result<TimeZone, Error> {
        if !is_plain_name(id) || spell_as_database(directory, id).as_deref() != Some(id) {
            return Err(Error::NotFound);
        }
        TimeZone::read_named(directory, id)
    }

    fn read_named(directory: &Path, id: &str) -> Result<TimeZone, Error> {
        let bytes = read_zone_file(&directory.join(id))?;
        TimeZone::from_bytes(id, &bytes)
    }

    /// A zone from the bytes of a TZif file, named `id`.
    pub fn from_bytes(id: &str, bytes: &[u8]) -> Result<TimeZone, Error> {
        let zone = Tzif::parse(bytes).map_err(|why| Error::Invalid(why.0.to_owned()))?;
        Ok(TimeZone {
            id: id.to_owned(),
            source: Source::File(Box::new(zone)),
        })
    }

    /// The zone of the machine: the `TZ` variable when it names a zone of the
    /// database exactly (an empty one means UTC; a path to a zone file is
    /// read), otherwise the file `/etc/localtime` is or points to, otherwise
    /// UTC. The name is the one the link spells, or `Local` for a copy.
    pub fn local() -> TimeZone {
        TimeZone::local_with(
            std::env::var("TZ").ok().as_deref(),
            &zone_directory(),
            Path::new("/etc/localtime"),
        )
    }

    /// [`TimeZone::local`] with the variable, directory and file given.
    pub fn local_with(variable: Option<&str>, directory: &Path, localtime: &Path) -> TimeZone {
        if let Some(variable) = variable {
            if variable.is_empty() {
                return TimeZone::utc();
            }
            let name = variable.strip_prefix(':').unwrap_or(variable);
            // The variable is the operator's own, so a path in it is trusted.
            let found = if name.starts_with('/') {
                zone_of_file(Path::new(name))
            } else {
                TimeZone::find_exact_in(directory, name).ok()
            };
            if let Some(zone) = found {
                return zone;
            }
        }
        zone_of_file(localtime).unwrap_or_else(TimeZone::utc)
    }

    /// The name the zone was found by.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The offset and abbreviation in force at `unix_seconds`.
    pub fn at(&self, unix_seconds: i64) -> Offset<'_> {
        match &self.source {
            Source::Utc => Offset {
                seconds: 0,
                abbreviation: "UTC",
                daylight: false,
            },
            Source::File(zone) => {
                let local: &LocalType = zone.at(unix_seconds);
                Offset {
                    seconds: local.offset,
                    abbreviation: &local.abbreviation,
                    daylight: local.dst,
                }
            }
        }
    }

    /// The clocks of the zone at `unix_seconds`.
    pub fn local_time(&self, unix_seconds: i64) -> LocalTime<'_> {
        let offset = self.at(unix_seconds);
        let shown = unix_seconds.saturating_add(i64::from(offset.seconds));
        let days = shown.div_euclid(86_400);
        let rest = shown.rem_euclid(86_400);
        let (year, month, day) = civil::civil_from_days(days);
        LocalTime {
            year,
            month,
            day,
            hour: (rest / 3600) as u32,
            minute: (rest % 3600 / 60) as u32,
            second: (rest % 60) as u32,
            weekday: civil::weekday_of_days(days),
            offset,
        }
    }

    /// The moment at which the zone's clocks show this date and time. A time
    /// the clocks skip, in the hour lost when they go forward, is taken as
    /// that much later: 02:30 on the night they go from 02:00 to 03:00 is
    /// 03:30. A time they show twice, in the hour repeated when they go back,
    /// is taken the first time.
    pub fn from_local(&self, year: i64, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> i64 {
        let local = civil::days_from_civil(year, month, day) * 86_400
            + i64::from(hour) * 3600
            + i64::from(minute) * 60
            + i64::from(second);
        // A day either side is far from any one transition, so the offsets
        // there are the two that can be in force at `local`.
        let before = i64::from(self.at(local - 86_400).seconds);
        let after = i64::from(self.at(local + 86_400).seconds);
        let shows = |moment: i64| moment + i64::from(self.at(moment).seconds) == local;
        [local - before, local - after]
            .into_iter()
            .filter(|moment| shows(*moment))
            .min()
            .unwrap_or(local - before)
    }

    /// The moment whose clock time in this zone is that of `start`, on the
    /// date `months` months and then `days` days after `start`'s date there.
    /// A day the month does not have is its last day, so the 31st of January
    /// a month on is the 28th or 29th of February, and two months on the 31st
    /// of March again.
    pub fn shifted(&self, start: i64, months: i64, days: i64) -> i64 {
        let shown = self.local_time(start);
        let index = shown.year * 12 + i64::from(shown.month) - 1 + months;
        let (year, month) = (index.div_euclid(12), (index.rem_euclid(12) + 1) as u32);
        let day = shown.day.min(civil::days_in_month(year, month));
        let (year, month, day) = civil::civil_from_days(civil::days_from_civil(year, month, day) + days);
        self.from_local(year, month, day, shown.hour, shown.minute, shown.second)
    }
}

/// The zone in a file, named by the part of the path (or of the link it is)
/// after `zoneinfo/`, or `Local` when there is none.
fn zone_of_file(path: &Path) -> Option<TimeZone> {
    let bytes = read_zone_file(path).ok()?;
    let id = fs::read_link(path)
        .ok()
        .and_then(|target| name_after_zoneinfo(&target.to_string_lossy()))
        .or_else(|| name_after_zoneinfo(&path.to_string_lossy()))
        .unwrap_or_else(|| "Local".into());
    TimeZone::from_bytes(&id, &bytes).ok()
}

fn name_after_zoneinfo(path: &str) -> Option<String> {
    let (_, name) = path.split_once("zoneinfo/")?;
    let name = name.trim_start_matches('/');
    (!name.is_empty()).then(|| name.to_owned())
}

/// The name as the database spells it: each segment is looked for among the
/// entries of its directory, ignoring ASCII case. Only the names the
/// directory lists are joined, so what comes out is a path inside it.
fn spell_as_database(directory: &Path, id: &str) -> Option<String> {
    let mut path = directory.to_path_buf();
    let mut spelled = Vec::new();
    for segment in id.split('/') {
        let names: Vec<String> = fs::read_dir(&path)
            .ok()?
            .take(MAX_ENTRIES)
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let entry = names
            .iter()
            .find(|name| *name == segment)
            .or_else(|| names.iter().find(|name| name.eq_ignore_ascii_case(segment)))?
            .clone();
        path.push(&entry);
        spelled.push(entry);
    }
    Some(spelled.join("/"))
}

fn zone_directory() -> PathBuf {
    match std::env::var_os("TZDIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(DEFAULT_DIRECTORY),
    }
}

/// The names the database uses: segments of letters, digits, `_`, `-`, `+`
/// and inner dots, separated by single slashes. No `..`, no empty segment, no
/// leading slash, no backslash: nothing that reaches outside the directory.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.split('/').all(|segment| {
            !segment.is_empty()
                && !segment.starts_with('.')
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+' | b'.'))
        })
}

fn read_zone_file(path: &Path) -> Result<Vec<u8>, Error> {
    let file = fs::File::open(path).map_err(|_| Error::NotFound)?;
    let meta = file.metadata().map_err(|_| Error::NotFound)?;
    if !meta.is_file() {
        return Err(Error::NotFound);
    }
    if meta.len() > MAX_FILE {
        return Err(Error::Invalid("the file is too large".into()));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::NotFound)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::tzif::build::{Spec, file};
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("stellarr-timezone-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Europe")).unwrap();
        dir
    }

    fn warsaw() -> Vec<u8> {
        file(&Spec {
            transitions: &[],
            types: &[(3600, false, "CET")],
            footer: Some("CET-1CEST,M3.5.0,M10.5.0/3"),
            version: b'2',
        })
    }

    #[test]
    fn utc_needs_no_file() {
        let zone = TimeZone::find_in(Path::new("/nonexistent"), "UTC").unwrap();

        assert_eq!(zone.id(), "UTC");
        assert_eq!(zone.at(1_700_000_000).seconds, 0);
        assert_eq!(zone.at(0).abbreviation, "UTC");
    }

    #[test]
    fn a_zone_is_found_by_its_name_and_tells_the_local_time() {
        let dir = directory("find");
        fs::write(dir.join("Europe/Warsaw"), warsaw()).unwrap();

        let zone = TimeZone::find_in(&dir, "Europe/Warsaw").unwrap();
        // 2026-11-20 13:05:00 UTC
        let local = zone.local_time(1_795_179_900);

        assert_eq!(zone.id(), "Europe/Warsaw");
        assert_eq!(
            (local.year, local.month, local.day, local.hour, local.minute),
            (2026, 11, 20, 14, 5)
        );
        assert_eq!(local.weekday, 5);
        assert_eq!(local.offset.abbreviation, "CET");
        let summer = zone.at(1_782_864_000);
        assert_eq!((summer.seconds, summer.daylight), (7200, true));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_case_of_a_name_does_not_matter_and_the_database_spells_the_id() {
        let dir = directory("case");
        fs::write(dir.join("Europe/Warsaw"), warsaw()).unwrap();

        for name in ["europe/warsaw", "EUROPE/WARSAW", "Europe/warsaw"] {
            let zone = TimeZone::find_in(&dir, name).unwrap();
            assert_eq!(zone.id(), "Europe/Warsaw", "{name}");
            assert_eq!(zone.at(1_795_179_900).seconds, 3600);
        }
        for name in ["utc", "Utc", "UTC", "uTc"] {
            assert_eq!(TimeZone::find_in(&dir, name).unwrap(), TimeZone::utc());
        }
        assert_eq!(
            TimeZone::find_in(&dir, "europe/nowhere"),
            Err(Error::NotFound)
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_name_that_could_leave_the_directory_is_not_looked_up() {
        let dir = directory("hostile");
        fs::write(dir.join("Europe/Warsaw"), warsaw()).unwrap();
        // A real zone file outside the directory that the names point at.
        let outside = dir.parent().unwrap().join("stellarr-timezone-outside");
        fs::write(&outside, warsaw()).unwrap();
        let hostile = [
            "",
            ".",
            "..",
            "../stellarr-timezone-outside",
            "Europe/../../stellarr-timezone-outside",
            "/etc/localtime",
            outside.to_str().unwrap(),
            "Europe//Warsaw",
            "Europe/Warsaw/",
            "/Europe/Warsaw",
            "Europe\\Warsaw",
            "Europe/Warsaw\0",
            "Europe/.hidden",
            " Europe/Warsaw",
            "Europe",
            "~root",
            "Europe/Wąrsaw",
            "EUROPE/../../stellarr-timezone-outside",
            "..\\x",
        ];
        for name in hostile {
            assert_eq!(
                TimeZone::find_in(&dir, name),
                Err(Error::NotFound),
                "{name:?}"
            );
        }
        let long = "A".repeat(100_000);
        assert_eq!(TimeZone::find_in(&dir, &long), Err(Error::NotFound));
        let deep = "A/".repeat(5000) + "B";
        assert_eq!(TimeZone::find_in(&dir, &deep), Err(Error::NotFound));
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_file(outside);
    }

    #[test]
    fn a_link_inside_the_directory_is_followed_as_the_database_wants() {
        let dir = directory("link");
        fs::write(dir.join("Europe/Warsaw"), warsaw()).unwrap();
        std::os::unix::fs::symlink("Europe/Warsaw", dir.join("Poland")).unwrap();

        let zone = TimeZone::find_in(&dir, "Poland").unwrap();

        assert_eq!(zone.id(), "Poland");
        assert_eq!(zone.at(1_795_179_900).seconds, 3600);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_is_not_a_zone_is_invalid_and_a_missing_one_is_not_found() {
        let dir = directory("invalid");
        fs::write(dir.join("zone.tab"), "# a table\n").unwrap();
        let mut cut = warsaw();
        cut.truncate(60);
        fs::write(dir.join("Cut"), cut).unwrap();
        fs::write(dir.join("Empty"), b"").unwrap();

        assert!(matches!(
            TimeZone::find_in(&dir, "zone.tab"),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            TimeZone::find_in(&dir, "Cut"),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            TimeZone::find_in(&dir, "Empty"),
            Err(Error::Invalid(_))
        ));
        assert_eq!(
            TimeZone::find_in(&dir, "Nowhere/Land"),
            Err(Error::NotFound)
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_local_zone_follows_the_variable_then_the_link_then_utc() {
        let dir = directory("local");
        fs::write(dir.join("Europe/Warsaw"), warsaw()).unwrap();
        let link = dir.join("localtime");
        let copy = dir.join("localtime-copy");
        std::os::unix::fs::symlink(dir.join("Europe/Warsaw"), &link).unwrap();
        fs::write(&copy, warsaw()).unwrap();
        let nothing = dir.join("absent");

        let from_variable = TimeZone::local_with(Some("Europe/Warsaw"), &dir, &nothing);
        assert_eq!(from_variable.id(), "Europe/Warsaw");
        let colon = TimeZone::local_with(Some(":Europe/Warsaw"), &dir, &nothing);
        assert_eq!(colon.id(), "Europe/Warsaw");
        assert_eq!(TimeZone::local_with(Some(""), &dir, &link), TimeZone::utc());
        // The directory here is not named zoneinfo, so a link has no name to
        // take and the zone is called Local, as for a copy of a file.
        assert_eq!(TimeZone::local_with(None, &dir, &link).id(), "Local");
        // The variable names a zone exactly, and may be a path to a file.
        assert_eq!(
            TimeZone::local_with(Some("europe/warsaw"), &dir, &nothing),
            TimeZone::utc()
        );
        let by_path = dir.join("usr/share/zoneinfo/Europe/Warsaw");
        fs::create_dir_all(by_path.parent().unwrap()).unwrap();
        fs::write(&by_path, warsaw()).unwrap();
        let from_path = TimeZone::local_with(Some(by_path.to_str().unwrap()), &dir, &nothing);
        assert_eq!(from_path.id(), "Europe/Warsaw");
        assert_eq!(TimeZone::local_with(None, &dir, &copy).id(), "Local");
        assert_eq!(
            TimeZone::local_with(Some("No/Such"), &dir, &copy).id(),
            "Local"
        );
        assert_eq!(
            TimeZone::local_with(Some("No/Such"), &dir, &nothing),
            TimeZone::utc()
        );
        assert_eq!(TimeZone::local_with(None, &dir, &nothing), TimeZone::utc());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_link_into_a_directory_named_zoneinfo_gives_the_name_after_it() {
        let dir = directory("named");
        let database = dir.join("usr/share/zoneinfo/Europe");
        fs::create_dir_all(&database).unwrap();
        fs::write(database.join("Warsaw"), warsaw()).unwrap();
        let link = dir.join("localtime");
        std::os::unix::fs::symlink(database.join("Warsaw"), &link).unwrap();

        let zone = TimeZone::local_with(None, &dir, &link);

        assert_eq!(zone.id(), "Europe/Warsaw");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn local_time_before_the_epoch_and_far_ahead_is_a_valid_date() {
        let zone = TimeZone::utc();
        let before = zone.local_time(-1);
        assert_eq!(
            (
                before.year,
                before.month,
                before.day,
                before.hour,
                before.second
            ),
            (1969, 12, 31, 23, 59)
        );
        let far = zone.local_time(i64::MAX);
        assert!(far.month >= 1 && far.month <= 12);
        let low = zone.local_time(i64::MIN);
        assert!(low.day >= 1);
    }
}
