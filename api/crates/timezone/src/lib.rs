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

    /// The zone of that name in the database at `directory`.
    pub fn find_in(directory: &Path, id: &str) -> Result<TimeZone, Error> {
        if id == "UTC" {
            return Ok(TimeZone::utc());
        }
        if !is_plain_name(id) {
            return Err(Error::NotFound);
        }
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
    /// database (an empty one means UTC), otherwise the file `/etc/localtime`
    /// is or points to, otherwise UTC. The name is the one the link spells,
    /// or `Local` for a copy.
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
            if let Ok(zone) = TimeZone::find_in(directory, name) {
                return zone;
            }
        }
        if let Ok(bytes) = read_zone_file(localtime) {
            let id = fs::read_link(localtime)
                .ok()
                .and_then(|target| {
                    let text = target.to_string_lossy().into_owned();
                    text.split_once("zoneinfo/")
                        .map(|(_, name)| name.to_owned())
                })
                .unwrap_or_else(|| "Local".into());
            if let Ok(zone) = TimeZone::from_bytes(&id, &bytes) {
                return zone;
            }
        }
        TimeZone::utc()
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
