//! Every message the crate builds, read by Python's own e-mail parser.
//!
//! The parser is the independent judge: it is not ours, so it does not share
//! our mistakes about what the encodings mean. The test is skipped, with a
//! note, on a machine without `python3`.

mod common;

use std::fs;
use std::process::Command;

#[test]
fn python_reads_every_kind_of_message_back_intact() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 is not installed: the independent parse was skipped");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("smtp-python-parse");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for sample in common::samples() {
        let encoded = sample.message.encode().unwrap();
        assert!(!encoded.needs_utf8);
        fs::write(dir.join(format!("{}.eml", sample.name)), &encoded.bytes).unwrap();
        common::write_expectations(&sample, &dir.join(format!("{}.expect", sample.name)));
    }
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/check_message.py");
    // -I keeps the interpreter from loading anything from the directories
    // it is run in.
    let output = Command::new("python3")
        .arg("-I")
        .arg(script)
        .arg(&dir)
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{report}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{report}");
}
