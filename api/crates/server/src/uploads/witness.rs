//! The parts of an upload that decide by rules of the first server's runtime,
//! asked the same questions it was asked.
//!
//! Not part of the ordinary run: it needs a corpus of inputs and what the
//! first server's own code answered for each, made by a small program that
//! calls the same functions (`double.TryParse`, `int.TryParse`,
//! `bool.TryParse`, `Convert.TryFromBase64String`, `Path.GetFileName` and the
//! range parser of the download route). A line of the corpus is the hex of an
//! input's UTF-8; a line of the answers is what that input makes of each.
//!
//! ```sh
//! UPLOADS_WITNESS_CORPUS=corpus.hex UPLOADS_WITNESS_ANSWERS=expected.txt \
//!   acargo test -p stellarr-api --lib uploads::witness -- --ignored
//! ```

use stellarr_storage::validation::spec_for;

use super::inspect::{is_placeholder, safe_file_name};
use super::numbers;
use crate::assets::range;

fn unhex(line: &str) -> String {
    let bytes: Vec<u8> = (0..line.len() / 2)
        .map(|at| u8::from_str_radix(&line[at * 2..at * 2 + 2], 16).unwrap())
        .collect();
    String::from_utf8(bytes).unwrap()
}

fn hex(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02X}")).collect()
}

/// What this server makes of an input, in the shape of the first server's line.
fn answer(input: &str) -> String {
    let decimal = match numbers::decimal(input) {
        None => "x".to_string(),
        Some(value) if value.is_nan() => "nan".to_string(),
        Some(value) => format!("{:x}", value.to_bits() as i64),
    };
    let integer = numbers::integer(input).map_or("x".to_string(), |value| value.to_string());
    let flag = match numbers::flag(input) {
        Some(true) => "t",
        Some(false) => "f",
        None => "x",
    };
    let placeholder = if is_placeholder(input) { "1" } else { "0" };
    let pdf = spec_for("application/pdf").unwrap();
    let name = hex(&safe_file_name(input, &pdf));
    let span = match range::parse(input) {
        Some(found) => format!("{}-{}", found.from, found.to.map_or(String::new(), |to| to.to_string())),
        None => "x".to_string(),
    };
    format!("{decimal} {integer} {flag} {placeholder} {name} {span}")
}

#[test]
#[ignore = "needs the corpus and the first server's answers; see the module comment"]
fn the_first_servers_runtime_agrees_on_every_input() {
    let corpus = std::fs::read_to_string(std::env::var("UPLOADS_WITNESS_CORPUS").unwrap()).unwrap();
    let answers = std::fs::read_to_string(std::env::var("UPLOADS_WITNESS_ANSWERS").unwrap()).unwrap();
    let mut differing = Vec::new();
    for (line, expected) in corpus.lines().zip(answers.lines()) {
        let input = unhex(line);
        let got = answer(&input);
        if got != expected {
            differing.push(format!("{input:?}\n  first: {expected}\n  here:  {got}"));
        }
    }
    let shown: Vec<_> = differing.iter().take(40).cloned().collect();
    assert!(differing.is_empty(), "{} differ:\n{}", differing.len(), shown.join("\n"));
}
