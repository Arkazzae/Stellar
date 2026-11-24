//! Canonical composition against the standard's own test file.

use stellarr_unicode::nfc;

fn text(field: &str) -> String {
    field
        .split(' ')
        .map(|code| {
            char::from_u32(u32::from_str_radix(code, 16).expect("a code point"))
                .expect("a character")
        })
        .collect()
}

/// Every line of `NormalizationTest.txt` of version 15.1.0: its NFC columns
/// (the second is the NFC of the first, the second and the third; the fourth
/// is the NFC of itself and of the fifth), and every character the first part
/// of the file does not list is its own NFC. Needs the file, which is not part
/// of the repository: see the crate's documentation.
///
/// ```sh
/// UCD_FOLDER=<folder> cargo test -p stellarr-unicode --test normalization -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs NormalizationTest.txt of Unicode 15.1.0; see the comment above"]
fn normalization_test_file() {
    let folder = std::env::var("UCD_FOLDER").expect("UCD_FOLDER names the folder of the files");
    let file = std::fs::read_to_string(std::path::Path::new(&folder).join("NormalizationTest.txt"))
        .expect("the file");
    let mut lines = 0usize;
    let mut listed = std::collections::BTreeSet::new();
    let mut part_one = false;
    for line in file.lines() {
        if line.starts_with("@Part1") {
            part_one = true;
        } else if line.starts_with("@Part") {
            part_one = false;
        }
        let data = line.split('#').next().unwrap_or("").trim();
        if data.is_empty() || data.starts_with('@') {
            continue;
        }
        let columns: Vec<String> = data.split(';').take(5).map(text).collect();
        assert_eq!(columns.len(), 5, "{line}");
        let [c1, c2, c3, c4, c5] = &columns[..] else {
            unreachable!()
        };
        for source in [c1, c2, c3] {
            assert_eq!(&nfc(source), c2, "{line}");
        }
        for source in [c4, c5] {
            assert_eq!(&nfc(source), c4, "{line}");
        }
        if part_one && c1.chars().count() == 1 {
            listed.insert(c1.chars().next().unwrap());
        }
        lines += 1;
    }
    // A character the file does not list stays as it is.
    let mut unlisted = 0usize;
    for code in 0..=0x10_FFFFu32 {
        let Some(c) = char::from_u32(code) else {
            continue;
        };
        if !listed.contains(&c) {
            let one = c.to_string();
            assert_eq!(nfc(&one), one, "U+{code:04X}");
            unlisted += 1;
        }
    }
    eprintln!(
        "{lines} lines of the file agree, and {unlisted} characters it does not list are their own NFC"
    );
    assert!(lines > 18_000, "only {lines} lines");
}
