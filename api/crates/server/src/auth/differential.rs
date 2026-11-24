//! Runs the same lines through this code and through a program built from the
//! first server's own source, so that the two answers can be diffed.
//!
//! Not part of the ordinary run. `tests/totp-independent.py cases N SEED`
//! writes the lines; this reads them from the file named by
//! `STELLARR_DIFFERENTIAL_CASES` and writes one answer per line to the file
//! named by `STELLARR_DIFFERENTIAL_OUT`:
//!
//! ```sh
//! STELLARR_DIFFERENTIAL_CASES=cases.txt STELLARR_DIFFERENTIAL_OUT=rust.txt \
//!   cargo test -p stellarr-api --lib differential -- --ignored
//! ```
//!
//! A line is an operation and its arguments, apart by tabs. Text is written as
//! hex of UTF-16BE, so that any character can be given.

use std::fmt::Write as _;

use super::{recovery_codes, tokens, totp};
use crate::encoding::hex_upper;
use crate::values::Timestamp;

fn text(hex: &str) -> Option<String> {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|pair| u16::from(pair[0]) << 8 | u16::from(pair[1]))
        .collect();
    String::from_utf16(&units).ok()
}

fn answer(line: &str) -> String {
    let fields: Vec<&str> = line.split('\t').collect();
    let argument = |index: usize| text(fields[index]);
    let moment = |index: usize| Timestamp::from_unix_seconds(fields[index].parse().unwrap());
    match fields[0] {
        "verify" => match totp::verify(
            &argument(1).unwrap(),
            &argument(2).unwrap(),
            moment(3).unwrap(),
        ) {
            Some(step) => format!("step {step}"),
            None => "no".to_string(),
        },
        "code" => match totp::code_at(&argument(1).unwrap(), moment(2).unwrap()) {
            Some(code) => code,
            None => "exception ArgumentException".to_string(),
        },
        "uri" => totp::otpauth_uri(
            &argument(1).unwrap(),
            &argument(2).unwrap(),
            &argument(3).unwrap(),
        ),
        "b32dec" => match totp::base32_decode(&argument(1).unwrap()) {
            Some(bytes) => hex_upper(&bytes),
            None => "error".to_string(),
        },
        "b32enc" => {
            let bytes: Vec<u8> = (0..fields[1].len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&fields[1][i..i + 2], 16).unwrap())
                .collect();
            totp::base32_encode(&bytes)
        }
        "hash" => tokens::hash(&argument(1).unwrap()),
        "normalize" => hex_upper(recovery_codes::normalize(&argument(1).unwrap()).as_bytes()),
        "guidparse" => match super::mfa::parse_method_id(&argument(1).unwrap()) {
            Some(id) => id.to_string(),
            None => "no".to_string(),
        },
        "trim" => {
            let mut out = String::new();
            for unit in argument(1).unwrap().trim().encode_utf16() {
                write!(out, "{unit:04X}").unwrap();
            }
            out
        }
        other => format!("unknown {other}"),
    }
}

#[test]
#[ignore = "needs the cases file and the first server's answers to be compared with"]
fn answer_the_cases_of_a_file() {
    let cases = std::env::var("STELLARR_DIFFERENTIAL_CASES").expect("a cases file");
    let out = std::env::var("STELLARR_DIFFERENTIAL_OUT").expect("a file to write");
    let mut answers = String::new();
    for line in std::fs::read_to_string(cases).unwrap().lines() {
        answers.push_str(&answer(line));
        answers.push('\n');
    }
    std::fs::write(out, answers).unwrap();
}
