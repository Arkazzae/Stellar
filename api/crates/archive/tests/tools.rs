//! Archives against other people's software: `unzip`, `zipinfo`, Python's
//! zipfile and the `zip` command. A tool that is not installed is reported
//! and its test passes, since there is nothing to compare with.

mod common;

use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use common::{Rng, Scratch, sample_entries};
use stellarr_archive::{EntryOptions, Error, Level, Method, ZipArchive, ZipWriter};

fn tool(name: &str) -> bool {
    let found = Command::new(name).arg("-h").output().is_ok();
    if !found {
        eprintln!("{name} is not available: part of the interoperability tests skipped");
    }
    found
}

fn run(command: &mut Command) -> String {
    let output = command.output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{command:?} failed:\n{text}");
    text
}

fn python(args: &[&str]) -> Option<String> {
    if !tool("python3") {
        return None;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/interop.py");
    Some(run(Command::new("python3")
        .arg("-I")
        .arg(script)
        .args(args)))
}

/// The files the archives are made from: empty, small, text, noise, runs,
/// and one in a folder.
fn write_inputs(dir: &Path) {
    let inputs = dir.join("inputs");
    fs::create_dir_all(inputs.join("sub/dir")).unwrap();
    let mut rng = Rng::new(3);
    fs::write(inputs.join("empty"), b"").unwrap();
    fs::write(inputs.join("small.txt"), b"hello\n").unwrap();
    fs::write(inputs.join("text.txt"), rng.text(70_000)).unwrap();
    fs::write(inputs.join("noise.bin"), rng.bytes(100_000)).unwrap();
    fs::write(inputs.join("zeros.bin"), vec![0u8; 300_000]).unwrap();
    fs::write(inputs.join("sub/dir/nested.txt"), rng.text(3000)).unwrap();
}

/// Archives of those files in several shapes, written by this crate.
fn write_mine(dir: &Path) {
    let inputs = dir.join("inputs");
    let names = [
        "empty",
        "small.txt",
        "text.txt",
        "noise.bin",
        "zeros.bin",
        "sub/dir/nested.txt",
    ];
    for (label, force, level, stored) in [
        ("default", false, Level::Default, false),
        ("zip64", true, Level::Default, false),
        ("fast", false, Level::Fast, false),
        ("best", false, Level::Best, false),
        ("stored", false, Level::Default, true),
        ("stored64", true, Level::Default, true),
    ] {
        let file = fs::File::create(dir.join(format!("mine-{label}.zip"))).unwrap();
        let mut zip = ZipWriter::new(std::io::BufWriter::new(file)).force_zip64(force);
        for name in names {
            let options = if stored {
                EntryOptions::stored()
            } else {
                EntryOptions::deflated(level)
            };
            let mut source = fs::File::open(inputs.join(name)).unwrap();
            zip.add_reader(name, options.modified(1_700_000_000), &mut source)
                .unwrap();
        }
        zip.add_bytes("folder/", EntryOptions::stored().unix_mode(0o040_755), b"")
            .unwrap();
        zip.finish().unwrap();
    }
    // The sample of the other tests, with names no tool should mangle.
    let mut zip = ZipWriter::new(Vec::new());
    for (name, data, deflated) in sample_entries() {
        let options = if deflated {
            EntryOptions::deflated(Level::Default)
        } else {
            EntryOptions::stored()
        };
        zip.add_bytes(&name, options, &data).unwrap();
    }
    fs::write(dir.join("mine-sample.zip"), zip.finish().unwrap()).unwrap();

    // Past what the 16-bit count of the end record holds.
    let mut zip = ZipWriter::new(Vec::new());
    for i in 0..70_000u32 {
        zip.add_bytes(&format!("many/{i:05}"), EntryOptions::stored(), b"")
            .unwrap();
    }
    fs::write(dir.join("mine-many.zip"), zip.finish().unwrap()).unwrap();
}

#[test]
fn what_this_writer_writes_unzip_zipinfo_and_python_accept() {
    let scratch = Scratch::new("mine");
    write_inputs(&scratch.0);
    write_mine(&scratch.0);

    let mut archives: Vec<_> = fs::read_dir(&scratch.0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "zip"))
        .collect();
    archives.sort();
    assert_eq!(archives.len(), 8);

    if tool("unzip") {
        for archive in &archives {
            let report = run(Command::new("unzip").arg("-t").arg(archive));
            assert!(
                report.contains("No errors detected"),
                "{}: {report}",
                archive.display()
            );
        }
    }
    if tool("zipinfo") {
        for archive in &archives {
            let listing = run(Command::new("zipinfo").arg(archive));
            assert!(
                listing.contains("files"),
                "{}: {listing}",
                archive.display()
            );
            let long = run(Command::new("zipinfo").arg("-v").arg(archive));
            assert!(!long.is_empty());
        }
        let many = run(Command::new("zipinfo")
            .arg("-h")
            .arg(scratch.0.join("mine-many.zip")));
        assert!(many.contains("70000"), "{many}");
    }
    if let Some(report) = python(&["check", scratch.0.to_str().unwrap()]) {
        eprintln!("{}", report.trim());
    }
}

#[test]
fn what_zipfile_and_the_zip_command_write_this_reader_reads() {
    let scratch = Scratch::new("theirs");
    write_inputs(&scratch.0);
    if !tool("zip") || python(&["make", scratch.0.to_str().unwrap()]).is_none() {
        return;
    }
    let inputs = scratch.0.join("inputs");

    let mut read = 0;
    for label in [
        "py-stored",
        "py-deflated",
        "py-zip64",
        "py-stream",
        "py-comment",
        "cli-deflated",
        "cli-stored",
        "cli-best",
        "cli-zip64",
        "cli-pipe",
    ] {
        let path = scratch.0.join(format!("{label}.zip"));
        let mut zip = ZipArchive::new(fs::File::open(&path).unwrap())
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(zip.duplicate_name(), None, "{label}");
        for index in 0..zip.entries().len() {
            let entry = zip.entries()[index].clone();
            let name = entry.name().unwrap().to_owned();
            let bytes = zip
                .read_to_vec(index, 1 << 20)
                .unwrap_or_else(|e| panic!("{label}: {name}: {e}"));
            assert_eq!(
                bytes.len() as u64,
                entry.uncompressed_size(),
                "{label}: {name}"
            );
            // Entries that came from a file must be that file; the rest are known by name.
            let source = inputs.join(name.trim_end_matches('/').trim_start_matches("inputs/"));
            if source.is_file() && !entry.is_directory() {
                assert_eq!(bytes, fs::read(&source).unwrap(), "{label}: {name}");
            }
            if label == "cli-pipe" {
                assert_eq!(name, "-");
                assert_eq!(bytes, b"hello\n");
            }
            read += 1;
        }
        if label == "cli-stored" || label == "py-stored" {
            assert!(
                zip.entries().iter().all(|e| e.method() == Method::Stored),
                "{label}"
            );
        }
        if label == "py-comment" {
            assert!(
                zip.find("unicode/\u{fc}\u{f1}\u{ee}.txt".as_bytes())
                    .is_some()
            );
        }
    }
    assert!(read > 50);

    // More entries than a 16-bit count holds, written by Python.
    let mut many = ZipArchive::new(fs::File::open(scratch.0.join("py-many.zip")).unwrap()).unwrap();
    assert_eq!(many.entries().len(), 70_000);
    many.verify_all().unwrap();

    // Exactly 65,535: the count field is full, and no ZIP64 record follows.
    let mut full =
        ZipArchive::new(fs::File::open(scratch.0.join("py-65535.zip")).unwrap()).unwrap();
    assert_eq!(full.entries().len(), 65_535);
    full.verify_all().unwrap();

    // Things this reader refuses, and says so cleanly.
    for label in ["py-bzip2", "py-lzma"] {
        let result =
            ZipArchive::new(fs::File::open(scratch.0.join(format!("{label}.zip"))).unwrap());
        assert!(
            matches!(result.err(), Some(Error::Unsupported(_))),
            "{label}"
        );
    }
    let encrypted = ZipArchive::new(fs::File::open(scratch.0.join("cli-encrypted.zip")).unwrap());
    assert!(matches!(encrypted.err(), Some(Error::Unsupported(_))));

    // A symbolic link is reported as one, with its target as the data.
    let mut link =
        ZipArchive::new(fs::File::open(scratch.0.join("cli-symlink.zip")).unwrap()).unwrap();
    let index = link
        .entries()
        .iter()
        .position(|e| e.is_symlink())
        .expect("no symlink entry");
    assert_eq!(link.read_to_vec(index, 100).unwrap(), b"../../etc/passwd");
}

#[test]
fn an_archive_survives_a_round_trip_through_unzip_extraction() {
    if !tool("unzip") {
        return;
    }
    let scratch = Scratch::new("extract");
    let mut zip = ZipWriter::new(Vec::new());
    for (name, data, deflated) in sample_entries() {
        let options = if deflated {
            EntryOptions::deflated(Level::Default)
        } else {
            EntryOptions::stored()
        };
        zip.add_bytes(&name, options, &data).unwrap();
    }
    let bytes = zip.finish().unwrap();
    let archive = scratch.0.join("a.zip");
    fs::write(&archive, &bytes).unwrap();
    let out = scratch.0.join("out");
    run(Command::new("unzip")
        .arg("-q")
        .arg(&archive)
        .arg("-d")
        .arg(&out));
    for (name, data, _) in sample_entries() {
        assert_eq!(fs::read(out.join(&name)).unwrap(), data, "{name}");
    }
    let mut zip = ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    zip.verify_all().unwrap();
}

/// About 330 MB through a temporary file, removed afterwards: a stored entry
/// of pseudo-random bytes, a deflated one of text, and a small one after
/// them. Checked by this reader, by `unzip -t` and by Python.
#[test]
#[ignore = "writes a few hundred megabytes to the temporary directory"]
fn a_few_hundred_megabytes_through_a_file() {
    use std::io::Write;
    let scratch = Scratch::new("large");
    let path = scratch.0.join("large.zip");
    let mut rng = Rng::new(9);
    let noise = rng.bytes(1 << 20);
    let text = rng.text(1 << 20);

    let mut zip = ZipWriter::new(std::io::BufWriter::new(fs::File::create(&path).unwrap()));
    let mut stored = zip
        .start_entry("objects/noise", EntryOptions::stored())
        .unwrap();
    for i in 0..200u32 {
        stored.write_all(&noise[(i as usize % 7) * 1000..]).unwrap();
    }
    let stored_info = stored.finish().unwrap();
    let mut deflated = zip
        .start_entry("database/text", EntryOptions::deflated(Level::Fast))
        .unwrap();
    for _ in 0..130 {
        deflated.write_all(&text).unwrap();
    }
    let deflated_info = deflated.finish().unwrap();
    zip.add_bytes(
        "manifest.json",
        EntryOptions::deflated(Level::Default),
        b"{}",
    )
    .unwrap();
    zip.finish().unwrap();
    let size = fs::metadata(&path).unwrap().len();
    assert!(size < 400 << 20, "{size}");

    let mut archive = ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
    assert_eq!(
        archive.entries()[0].uncompressed_size(),
        stored_info.uncompressed_size
    );
    assert_eq!(
        archive.entries()[1].uncompressed_size(),
        deflated_info.uncompressed_size
    );
    archive.verify_all().unwrap();

    if tool("unzip") {
        let report = run(Command::new("unzip").arg("-t").arg(&path));
        assert!(report.contains("No errors detected"), "{report}");
    }
    if tool("python3") {
        let check = "import sys, zipfile; z = zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None; print(len(z.namelist()))";
        run(Command::new("python3")
            .arg("-I")
            .arg("-c")
            .arg(check)
            .arg(&path));
    }
}
