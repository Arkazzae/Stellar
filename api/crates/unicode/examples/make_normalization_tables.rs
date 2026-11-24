//! Writes `src/normalization_tables.rs` from the Unicode Character Database.
//!
//! ```sh
//! cargo run -p stellarr-unicode --example make_normalization_tables -- <folder> > src/normalization_tables.rs
//! ```
//!
//! The folder holds `UnicodeData.txt` and `CompositionExclusions.txt` of
//! version 15.1.0 (see the crate's documentation for where they are). Each
//! file's SHA-256 is checked against the one written here before it is read:
//! a file of another version, or altered, stops the program.

use std::fmt::Write as _;
use std::path::Path;

use aws_lc_rs::digest::{SHA256, digest};

const VERSION: &str = "15.1.0";
const FILES: [(&str, &str); 2] = [
    (
        "UnicodeData.txt",
        "2fc713e6a31a87c4850a37fe2caffa4218180fadb5de86b43a143ddb4581fb86",
    ),
    (
        "CompositionExclusions.txt",
        "59d2d9e3dfdf0a999cf9dae11d594f053631222679a2f5710315ea07f7fe82af",
    ),
];

fn read(folder: &Path, name: &str, expected: &str) -> Result<String, String> {
    let bytes = std::fs::read(folder.join(name)).map_err(|e| format!("{name}: {e}"))?;
    let found: String = digest(&SHA256, &bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if found != expected {
        return Err(format!(
            "{name} is not the file of version {VERSION}: its SHA-256 is {found}, expected {expected}"
        ));
    }
    String::from_utf8(bytes).map_err(|e| format!("{name}: {e}"))
}

/// Rows written as the tables are: a line holds as many as fit in 100 columns.
fn table(output: &mut String, name: &str, kind: &str, rows: &[String]) {
    let _ = writeln!(output, "#[rustfmt::skip]");
    let _ = writeln!(output, "pub(crate) static {name}: &[{kind}] = &[");
    let mut line = String::from("   ");
    for row in rows {
        if line.len() + row.len() + 2 > 100 {
            let _ = writeln!(output, "{}", line.trim_end());
            line = String::from("   ");
        }
        let _ = write!(line, " {row},");
    }
    if line.len() > 3 {
        let _ = writeln!(output, "{line}");
    }
    let _ = writeln!(output, "];\n");
}

fn run(folder: &Path) -> Result<String, String> {
    let data = read(folder, FILES[0].0, FILES[0].1)?;
    let exclusions = read(folder, FILES[1].0, FILES[1].1)?;

    let mut class = vec![0u8; 0x11_0000];
    // One-level canonical mappings: a character, then one or two.
    let mut mappings: Vec<(u32, u32, u32)> = Vec::new();
    for line in data.lines() {
        let fields: Vec<&str> = line.split(';').collect();
        let [code, name, _, ccc, _, decomposition, ..] = fields[..] else {
            continue;
        };
        let code = u32::from_str_radix(code, 16).map_err(|e| format!("{line}: {e}"))?;
        // Ranges (CJK, Hangul, private use) have no class and no mapping, and
        // Hangul is composed by arithmetic.
        if name.ends_with(", First>") || name.ends_with(", Last>") {
            continue;
        }
        class[code as usize] = ccc.parse().map_err(|e| format!("{line}: {e}"))?;
        if !decomposition.is_empty() && !decomposition.starts_with('<') {
            let parts: Vec<u32> = decomposition
                .split(' ')
                .map(|part| u32::from_str_radix(part, 16))
                .collect::<Result<_, _>>()
                .map_err(|e| format!("{line}: {e}"))?;
            match parts[..] {
                [one] => mappings.push((code, one, 0)),
                [first, second] => mappings.push((code, first, second)),
                _ => {
                    return Err(format!(
                        "{line}: a canonical mapping of {} characters",
                        parts.len()
                    ));
                }
            }
        }
    }
    // What does not compose back: the file's list, the singletons, and every
    // character that decomposes into something starting with a mark, or is a
    // mark that decomposes.
    let mut excluded: Vec<u32> = exclusions
        .lines()
        .filter_map(|line| {
            let code = line.split('#').next()?.trim();
            u32::from_str_radix(code, 16).ok()
        })
        .collect();
    for &(code, first, second) in &mappings {
        if second == 0 || class[first as usize] != 0 || class[code as usize] != 0 {
            excluded.push(code);
        }
    }

    let mut ranges: Vec<(u32, u32, u8)> = Vec::new();
    for (code, &value) in class.iter().enumerate() {
        let code = code as u32;
        if value == 0 {
            continue;
        }
        match ranges.last_mut() {
            Some(last) if last.2 == value && last.1 + 1 == code => last.1 = code,
            _ => ranges.push((code, code, value)),
        }
    }
    let mut compositions: Vec<(u32, u32, u32)> = mappings
        .iter()
        .filter(|&&(code, _, second)| second != 0 && !excluded.contains(&code))
        .map(|&(code, first, second)| (first, second, code))
        .collect();
    compositions.sort_by_key(|&(first, second, _)| (first, second));

    let mut output = String::new();
    let _ = writeln!(
        output,
        "//! Generated by `examples/make_normalization_tables.rs` from the Unicode Character\n//! Database, version {VERSION} (`UnicodeData.txt` and `CompositionExclusions.txt`). Do not\n//! edit: see the crate's documentation for how to make it again.\n"
    );
    table(
        &mut output,
        "COMBINING_CLASS",
        "(u32, u32, u8)",
        &ranges
            .iter()
            .map(|(a, b, c)| format!("(0x{a:X}, 0x{b:X}, {c})"))
            .collect::<Vec<_>>(),
    );
    table(
        &mut output,
        "DECOMPOSITION",
        "(u32, u32, u32)",
        &mappings
            .iter()
            .map(|(a, b, c)| format!("(0x{a:X}, 0x{b:X}, 0x{c:X})"))
            .collect::<Vec<_>>(),
    );
    table(
        &mut output,
        "COMPOSITION",
        "(u32, u32, u32)",
        &compositions
            .iter()
            .map(|(a, b, c)| format!("(0x{a:X}, 0x{b:X}, 0x{c:X})"))
            .collect::<Vec<_>>(),
    );
    Ok(output.trim_end().to_string() + "\n")
}

fn main() {
    let Some(folder) = std::env::args().nth(1) else {
        eprintln!(
            "usage: make_normalization_tables <folder of the Unicode Character Database files>"
        );
        std::process::exit(2);
    };
    match run(Path::new(&folder)) {
        Ok(text) => print!("{text}"),
        Err(error) => {
            eprintln!("make_normalization_tables: {error}");
            std::process::exit(1);
        }
    }
}
