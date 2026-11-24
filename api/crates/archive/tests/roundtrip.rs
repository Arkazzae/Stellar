//! What the writer writes, the reader reads back.

mod common;

use std::io::{Cursor, Read, Write};

use common::{open, sample_entries, write_sample};
use stellarr_archive::{EntryOptions, Error, Level, Method, ZipArchive, ZipWriter};

#[test]
fn entries_come_back_as_written_in_the_order_written() {
    for force in [false, true] {
        let bytes = write_sample(force);
        let mut zip = open(&bytes);
        let expected = sample_entries();
        assert_eq!(zip.entries().len(), expected.len());
        for (index, (name, data, deflated)) in expected.iter().enumerate() {
            let entry = &zip.entries()[index];
            assert_eq!(entry.name_bytes(), name.as_bytes(), "force={force}");
            assert_eq!(entry.name(), Some(name.as_str()));
            assert_eq!(entry.uncompressed_size(), data.len() as u64);
            assert_eq!(
                entry.method(),
                if *deflated {
                    Method::Deflated
                } else {
                    Method::Stored
                }
            );
            assert_eq!(entry.crc32(), stellarr_deflate::crc32(data));
            assert_eq!(entry.modified_unix(), 1_700_000_000);
            assert_eq!(entry.name_is_flagged_utf8(), !name.is_ascii());
            assert!(!entry.is_directory() && !entry.is_symlink());
            assert_eq!(
                zip.read_to_vec(index, u64::MAX).unwrap(),
                *data,
                "{name} force={force}"
            );
        }
        assert_eq!(zip.duplicate_name(), None);
        assert_eq!(zip.find(b"manifest.json"), Some(expected.len() - 1));
        assert_eq!(zip.find(b"nothing"), None);
        zip.verify_all().unwrap();
        assert_eq!(
            zip.total_uncompressed_size(),
            expected.iter().map(|e| e.1.len() as u64).sum::<u64>()
        );
    }
}

#[test]
fn deflating_shrinks_text_and_stored_keeps_the_bytes() {
    let bytes = write_sample(false);
    let zip = open(&bytes);
    let db = &zip.entries()[0];
    assert!(db.compressed_size() * 3 < db.uncompressed_size());
    let image = &zip.entries()[1];
    assert_eq!(image.compressed_size(), image.uncompressed_size());
}

#[test]
fn modes_folders_and_times_are_kept() {
    let mut zip = ZipWriter::new(Vec::new());
    zip.add_bytes(
        "secrets/key",
        EntryOptions::stored()
            .unix_mode(0o100_600)
            .modified(1_709_210_096),
        b"k",
    )
    .unwrap();
    zip.add_bytes("secrets/", EntryOptions::stored().unix_mode(0o040_700), b"")
        .unwrap();
    zip.add_bytes(
        "link",
        EntryOptions::stored().unix_mode(0o120_777),
        b"target",
    )
    .unwrap();
    zip.add_bytes(
        "plain",
        EntryOptions::stored().modified(1_709_210_097),
        b"p",
    )
    .unwrap();
    zip.add_bytes("undated", EntryOptions::stored(), b"p")
        .unwrap();
    zip.add_bytes("dos-folder/", EntryOptions::stored(), b"")
        .unwrap();
    let bytes = zip.finish().unwrap();
    let zip = open(&bytes);
    let entries = zip.entries();
    assert_eq!(entries[0].unix_mode(), Some(0o100_600));
    assert_eq!(entries[0].modified_unix(), 1_709_210_096);
    assert!(entries[1].is_directory());
    assert!(entries[2].is_symlink());
    assert_eq!(entries[3].unix_mode(), Some(0o100_644));
    assert_eq!(entries[3].modified_unix(), 1_709_210_097);
    assert_eq!(entries[4].modified_unix(), 315_532_800);
    assert!(entries[5].is_directory());
    assert_eq!(entries[5].unix_mode(), Some(0o040_755));
}

#[test]
fn names_are_checked_when_written() {
    let mut zip = ZipWriter::new(Vec::new());
    assert!(matches!(
        zip.start_entry("", EntryOptions::stored()),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        zip.start_entry("a\0b", EntryOptions::stored()),
        Err(Error::Invalid(_))
    ));
    let too_long = "x".repeat(65_536);
    assert!(matches!(
        zip.start_entry(&too_long, EntryOptions::stored()),
        Err(Error::Invalid(_))
    ));
    let longest = "y".repeat(65_535);
    zip.add_bytes(&longest, EntryOptions::stored(), b"ok")
        .unwrap();
    let bytes = zip.finish().unwrap();
    let mut zip = open(&bytes);
    assert_eq!(zip.entries()[0].name_bytes().len(), 65_535);
    assert_eq!(zip.read_to_vec(0, 10).unwrap(), b"ok");
}

#[test]
fn an_entry_left_open_stops_the_archive_from_being_finished() {
    let mut zip = ZipWriter::new(Vec::new());
    {
        let mut entry = zip.start_entry("a", EntryOptions::stored()).unwrap();
        entry.write_all(b"half").unwrap();
    }
    assert!(matches!(
        zip.start_entry("b", EntryOptions::stored()),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(zip.finish(), Err(Error::Invalid(_))));
}

#[test]
fn many_entries_need_the_zip64_end_record() {
    let mut zip = ZipWriter::new(Vec::new());
    for i in 0..70_000u32 {
        zip.add_bytes(
            &format!("many/{i:05}"),
            EntryOptions::stored(),
            if i % 7 == 0 { b"x" } else { b"" },
        )
        .unwrap();
    }
    let bytes = zip.finish().unwrap();
    assert!(
        bytes.windows(4).any(|w| w == [0x50, 0x4b, 0x06, 0x06]),
        "no ZIP64 end record"
    );
    let mut zip = open(&bytes);
    assert_eq!(zip.entries().len(), 70_000);
    assert_eq!(zip.entries()[69_999].name(), Some("many/69999"));
    assert_eq!(zip.read_to_vec(7, 10).unwrap(), b"x");
    zip.verify_all().unwrap();
}

#[test]
fn a_small_archive_has_no_zip64_end_record_unless_forced() {
    let plain = write_sample(false);
    assert!(!plain.windows(4).any(|w| w == [0x50, 0x4b, 0x06, 0x06]));
    let forced = write_sample(true);
    assert!(forced.windows(4).any(|w| w == [0x50, 0x4b, 0x06, 0x06]));
}

#[test]
fn a_large_entry_streams_in_pieces() {
    let mut rng = common::Rng::new(5);
    let piece = rng.text(100_000);
    let noise = rng.bytes(100_000);
    let mut zip = ZipWriter::new(Vec::new());
    let mut entry = zip
        .start_entry("big", EntryOptions::deflated(Level::Fast))
        .unwrap();
    for i in 0..30 {
        entry
            .write_all(if i % 3 == 0 { &noise } else { &piece })
            .unwrap();
        entry.write_all(&[i as u8]).unwrap();
    }
    let info = entry.finish().unwrap();
    assert_eq!(info.uncompressed_size, 30 * 100_001);
    let bytes = zip.finish().unwrap();

    let mut zip = open(&bytes);
    let mut reader = zip.open(0).unwrap();
    let mut seen = 0u64;
    let mut buf = [0u8; 7000];
    loop {
        let n = reader.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        seen += n as u64;
    }
    assert_eq!(seen, info.uncompressed_size);
}

#[test]
fn a_reader_as_the_source_of_an_entry() {
    let data = common::Rng::new(1).text(50_000);
    let mut zip = ZipWriter::new(Vec::new());
    let info = zip
        .add_reader(
            "from-reader",
            EntryOptions::deflated(Level::Default),
            &mut Cursor::new(&data),
        )
        .unwrap();
    assert_eq!(info.uncompressed_size, 50_000);
    let bytes = zip.finish().unwrap();
    assert_eq!(open(&bytes).read_to_vec(0, 100_000).unwrap(), data);
}

#[test]
fn an_archive_with_nothing_in_it_is_valid() {
    let bytes = ZipWriter::new(Vec::new()).finish().unwrap();
    assert_eq!(bytes.len(), 22);
    let mut zip = open(&bytes);
    assert!(zip.entries().is_empty());
    zip.verify_all().unwrap();
}

#[test]
fn reading_in_two_steps_returns_every_byte() {
    let bytes = write_sample(false);
    let mut zip = open(&bytes);
    let mut reader = zip.open(0).unwrap();
    let mut first = [0u8; 10];
    reader.read_exact(&mut first).unwrap();
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).unwrap();
    assert_eq!(rest.len(), 200_000 - 10);
}

#[test]
fn opening_an_entry_that_is_not_there_is_an_error() {
    let bytes = write_sample(false);
    let mut zip = ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    assert!(matches!(zip.open(99).err(), Some(Error::Invalid(_))));
}

#[test]
fn the_overhead_bound_holds() {
    for name_len in [1usize, 20, 300] {
        for force in [false, true] {
            let name = "n".repeat(name_len);
            let mut zip = ZipWriter::new(Vec::new()).force_zip64(force);
            zip.add_bytes(
                &name,
                EntryOptions::stored().modified(1_700_000_000),
                b"data",
            )
            .unwrap();
            let size = zip.finish().unwrap().len() as u64;
            assert!(
                size <= 4
                    + stellarr_archive::entry_overhead(name_len)
                    + stellarr_archive::ARCHIVE_OVERHEAD,
                "{name_len} {size}"
            );
        }
    }
}
