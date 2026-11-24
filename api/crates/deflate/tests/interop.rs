//! Checks against Python's zlib: its streams through this decoder, and this
//! encoder's streams through its decoder. The inputs come from the generator
//! in `common`. When `python3` is not on the machine the test says so and
//! passes, since there is nothing to compare with.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use stellarr_deflate::{Level, deflate_to_vec, gunzip_to_vec, inflate_to_vec, zlib_to_vec};

fn python(args: &[&str]) -> Option<String> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/interop.py");
    let output = match Command::new("python3")
        .arg("-I")
        .arg(&script)
        .args(args)
        .output()
    {
        Ok(output) => output,
        Err(_) => {
            eprintln!("python3 is not available: interoperability test skipped");
            return None;
        }
    };
    assert!(
        output.status.success(),
        "python failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("stellarr-deflate-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn what_zlib_writes_this_decoder_reads_at_every_level_and_strategy() {
    let scratch = Scratch::new("from-python");
    let corpus = common::corpus(1);
    for (name, data) in &corpus {
        fs::write(scratch.0.join(format!("{name}.raw")), data).unwrap();
    }
    if python(&["compress", scratch.0.to_str().unwrap()]).is_none() {
        return;
    }

    let mut streams = 0;
    for (name, data) in &corpus {
        let prefix = format!("{name}.py.");
        for entry in fs::read_dir(&scratch.0).unwrap() {
            let path = entry.unwrap().path();
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let Some(kind) = file.strip_prefix(&prefix) else {
                continue;
            };
            let stream = fs::read(&path).unwrap();
            let decoded = if kind.ends_with(".def") {
                inflate_to_vec(&stream, u64::MAX)
            } else if kind == "zlib" {
                zlib_to_vec(&stream, u64::MAX)
            } else {
                gunzip_to_vec(&stream, u64::MAX)
            };
            let decoded = decoded.unwrap_or_else(|e| panic!("{file}: {e}"));
            assert!(decoded == *data, "{file}: decoded bytes differ");
            streams += 1;
        }
    }
    eprintln!("decoded {streams} streams written by zlib");
    assert!(streams > corpus.len() * 15);
}

#[test]
fn what_this_encoder_writes_zlib_reads() {
    let scratch = Scratch::new("to-python");
    let corpus = common::corpus(1);
    for (name, data) in &corpus {
        fs::write(scratch.0.join(format!("{name}.raw")), data).unwrap();
        for (label, level) in [
            ("fast", Level::Fast),
            ("default", Level::Default),
            ("best", Level::Best),
        ] {
            let stream = deflate_to_vec(data, level);
            fs::write(scratch.0.join(format!("{name}.mine.{label}.def")), &stream).unwrap();
            // The same stream in a zlib wrapper, which Python's decompress reads whole.
            let mut wrapped = vec![0x78, 0x9c];
            wrapped.extend_from_slice(&stream);
            let mut adler = stellarr_deflate::Adler32::new();
            adler.update(data);
            wrapped.extend_from_slice(&adler.value().to_be_bytes());
            fs::write(scratch.0.join(format!("{name}.mine.{label}.zz")), wrapped).unwrap();
        }
    }
    let Some(report) = python(&["verify", scratch.0.to_str().unwrap()]) else {
        return;
    };
    eprintln!("{}", report.trim());
}
