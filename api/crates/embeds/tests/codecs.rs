//! The single-byte tables and the entity table against Python's own codecs,
//! all 256 bytes of each. `tests/codecs/codecs.py` made the committed
//! expectations and can make them again; one test here runs it live.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use stellarr_embeds::charset::SINGLE_BYTE_SETS;
use stellarr_embeds::decode_entities;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/codecs")
}

fn tables_from(text: &str) -> BTreeMap<String, Vec<u32>> {
    text.lines()
        .map(|line| {
            let mut parts = line.split(' ');
            let name = parts.next().unwrap().to_owned();
            (
                name,
                parts.map(|p| u32::from_str_radix(p, 16).unwrap()).collect(),
            )
        })
        .collect()
}

fn check_tables(expected: &BTreeMap<String, Vec<u32>>) {
    assert_eq!(expected.len(), SINGLE_BYTE_SETS.len());
    for (name, table) in SINGLE_BYTE_SETS {
        let want = &expected[name];
        assert_eq!(want.len(), 256, "{name}");
        for byte in 0..256 {
            assert_eq!(
                u32::from(table[byte]),
                want[byte],
                "{name} byte {byte:#04x}"
            );
        }
    }
}

fn check_entities(text: &str) {
    let mut count = 0;
    for line in text.lines() {
        let (name, code) = line.split_once(' ').unwrap();
        let want = char::from_u32(u32::from_str_radix(code, 16).unwrap()).unwrap();
        assert_eq!(
            decode_entities(&format!("&{name};")),
            want.to_string(),
            "&{name};"
        );
        count += 1;
    }
    assert_eq!(count, 253);
}

fn python(command: &str) -> Option<String> {
    let output = Command::new("python3")
        .arg("-I")
        .arg(dir().join("codecs.py"))
        .arg(command)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).unwrap())
}

#[test]
fn every_table_matches_the_committed_python_output() {
    check_tables(&tables_from(
        &std::fs::read_to_string(dir().join("expected.txt")).unwrap(),
    ));
}

#[test]
fn every_table_matches_python_run_now() {
    match python("expected") {
        Some(text) => check_tables(&tables_from(&text)),
        None => eprintln!("python3 is not here; only the committed output was checked"),
    }
}

#[test]
fn the_committed_expectations_are_what_python_says_now() {
    let Some(text) = python("expected") else {
        return;
    };
    assert_eq!(
        text,
        std::fs::read_to_string(dir().join("expected.txt")).unwrap()
    );
    let entities = python("entities").unwrap();
    assert_eq!(
        entities,
        std::fs::read_to_string(dir().join("entities.txt")).unwrap()
    );
}

#[test]
fn every_named_reference_decodes_as_python_says() {
    check_entities(&std::fs::read_to_string(dir().join("entities.txt")).unwrap());
}
