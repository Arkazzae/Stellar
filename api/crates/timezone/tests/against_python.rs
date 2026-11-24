//! The crate against Python's zoneinfo, which reads the same files with a
//! different implementation. `expected.tsv` and the zone files beside it come
//! from `generate_expected.py`, which says how to make them again.

use std::collections::HashMap;
use std::path::Path;

use stellarr_timezone::TimeZone;

fn zones() -> HashMap<String, TimeZone> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/zoneinfo");
    let expected =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/expected.tsv"))
            .unwrap();
    let mut zones = HashMap::new();
    for line in expected.lines().filter(|l| !l.starts_with('#')) {
        let name = line.split('\t').nth(1).unwrap();
        zones.entry(name.to_owned()).or_insert_with(|| {
            TimeZone::find_in(&directory, name).unwrap_or_else(|e| panic!("{name}: {e}"))
        });
    }
    zones
}

fn check(zone: &TimeZone, moment: i64, offset: i32, abbreviation: &str, context: &str) {
    let found = zone.at(moment);
    assert_eq!(
        (found.seconds, found.abbreviation),
        (offset, abbreviation),
        "{} at {moment} ({context})",
        zone.id()
    );
}

#[test]
fn every_transition_and_every_random_moment_matches_python() {
    let zones = zones();
    let expected =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/expected.tsv"))
            .unwrap();
    let (mut transitions, mut samples, mut beyond_2037) = (0, 0, 0);
    for line in expected.lines().filter(|l| !l.starts_with('#')) {
        let fields: Vec<&str> = line.split('\t').collect();
        let zone = &zones[fields[1]];
        let moment: i64 = fields[2].parse().unwrap();
        // 2038-01-01
        beyond_2037 += usize::from(moment >= 2_145_916_800);
        match fields[0] {
            "T" => {
                let (before, before_name) = (fields[3].parse().unwrap(), fields[4]);
                let (after, after_name) = (fields[5].parse().unwrap(), fields[6]);
                for back in 1..=3 {
                    check(
                        zone,
                        moment - back,
                        before,
                        before_name,
                        "before the transition",
                    );
                }
                for forward in 0..=2 {
                    check(
                        zone,
                        moment + forward,
                        after,
                        after_name,
                        "from the transition",
                    );
                }
                transitions += 1;
            }
            "S" => {
                check(
                    zone,
                    moment,
                    fields[3].parse().unwrap(),
                    fields[4],
                    "a random moment",
                );
                samples += 1;
            }
            other => panic!("unknown line kind {other}"),
        }
    }
    assert!(transitions > 5000, "{transitions} transitions");
    assert!(samples > 5000, "{samples} samples");
    // The files hold transitions up to 2037; after that the footer rule rules.
    assert!(beyond_2037 > 1000, "{beyond_2037} moments past 2037");
}

#[test]
fn the_list_covers_both_hemispheres_odd_offsets_and_changed_rules() {
    let zones = zones();
    let at_summer = 1_783_000_000; // July 2026
    let at_winter = 1_769_000_000; // January 2026
    let offset = |name: &str, moment| zones[name].at(moment).seconds;

    assert_eq!(offset("Europe/Warsaw", at_summer), 7200);
    assert_eq!(offset("Australia/Sydney", at_winter), 39_600);
    assert_eq!(offset("Asia/Kolkata", at_summer), 19_800);
    assert_eq!(offset("Asia/Kathmandu", at_summer), 20_700);
    assert_eq!(offset("Pacific/Chatham", at_winter), 49_500);
    assert_eq!(offset("Australia/Lord_Howe", at_winter), 39_600);
    assert_eq!(offset("America/St_Johns", at_winter), -12_600);
    assert_eq!(offset("Asia/Tokyo", at_summer), 32_400);
    // Moscow gave up daylight saving in 2011, and Samoa moved to +13 for good.
    assert_eq!(offset("Europe/Moscow", at_summer), 10_800);
    assert_eq!(offset("Pacific/Apia", at_summer), 46_800);
    assert_eq!(offset("Pacific/Kiritimati", at_summer), 50_400);
}
