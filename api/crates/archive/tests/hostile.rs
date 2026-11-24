//! Archives that are damaged or lying, on purpose: the cases a backup
//! restore has to refuse, and a good many it has never been told about.

mod common;

use std::io::{Cursor, Read};

use common::{Rng, central_record, data_start, open};
use stellarr_archive::{EntryOptions, Error, Level, Limits, ZipArchive, ZipWriter};

/// A small archive of the kinds of entry a backup holds.
fn small() -> Vec<u8> {
    let mut rng = Rng::new(21);
    let mut zip = ZipWriter::new(Vec::new());
    zip.add_bytes(
        "database/stellarr.db",
        EntryOptions::deflated(Level::Default),
        &rng.text(30_000),
    )
    .unwrap();
    zip.add_bytes(
        "objects/images/a.png",
        EntryOptions::stored(),
        &rng.bytes(3000),
    )
    .unwrap();
    zip.add_bytes(
        "objects/audio/b.webm",
        EntryOptions::stored(),
        b"voice voice voice",
    )
    .unwrap();
    zip.add_bytes(
        "keys/key-1.xml",
        EntryOptions::deflated(Level::Default),
        b"<key />",
    )
    .unwrap();
    zip.add_bytes(
        "manifest.json",
        EntryOptions::deflated(Level::Default),
        b"{\"format\":\"stellarr-backup\"}",
    )
    .unwrap();
    zip.finish().unwrap()
}

fn verdict(bytes: &[u8]) -> Result<(), Error> {
    ZipArchive::new(Cursor::new(bytes))?.verify_all()
}

fn patch(bytes: &mut [u8], at: usize, with: &[u8]) {
    bytes[at..at + with.len()].copy_from_slice(with);
}

#[test]
fn the_untouched_archive_passes() {
    verdict(&small()).unwrap();
}

#[test]
fn a_changed_byte_in_a_stored_entry_fails_its_checksum() {
    let mut bytes = small();
    let at = data_start(&bytes, "objects/images/a.png") + 1500;
    bytes[at] ^= 0x01;
    match verdict(&bytes) {
        Err(Error::Checksum(name)) => assert_eq!(name, "objects/images/a.png"),
        other => panic!("{other:?}"),
    }
    // Reading the neighbours is unaffected.
    let mut zip = open(&bytes);
    assert!(zip.read_to_vec(2, 100).is_ok());
}

#[test]
fn a_changed_byte_in_a_deflated_entry_is_caught_wherever_it_falls() {
    let clean = small();
    let start = data_start(&clean, "database/stellarr.db");
    let size = {
        let zip = open(&clean);
        zip.entries()[0].compressed_size() as usize
    };
    let mut caught = 0;
    for offset in (0..size).step_by(37) {
        let mut bytes = clean.clone();
        bytes[start + offset] ^= 0x40;
        match verdict(&bytes) {
            Err(Error::Checksum(_) | Error::Corrupt(_)) => caught += 1,
            Err(other) => panic!("{other:?}"),
            Ok(()) => panic!("a change at {offset} went unnoticed"),
        }
    }
    assert!(caught > 10);
}

#[test]
fn a_wrong_crc_in_the_directory_is_caught() {
    for name in ["objects/audio/b.webm", "keys/key-1.xml"] {
        let mut bytes = small();
        let record = central_record(&bytes, name);
        bytes[record + 16] ^= 0xff;
        assert!(
            matches!(verdict(&bytes), Err(Error::Checksum(n)) if n == name),
            "{name}"
        );
    }
}

#[test]
fn a_wrong_size_in_the_directory_is_caught() {
    // Deflated: the recorded size is lower or higher than the real one.
    for delta in [-1i64, 1, 1000] {
        let mut bytes = small();
        let record = central_record(&bytes, "keys/key-1.xml");
        let size = u32::from_le_bytes(bytes[record + 24..record + 28].try_into().unwrap());
        patch(
            &mut bytes,
            record + 24,
            &((i64::from(size) + delta) as u32).to_le_bytes(),
        );
        assert!(
            matches!(verdict(&bytes), Err(Error::Corrupt(_))),
            "delta {delta}"
        );
    }
    // Stored: both sizes grow, so the entry would run into the next one.
    let mut bytes = small();
    let record = central_record(&bytes, "objects/audio/b.webm");
    for field in [20, 24] {
        patch(&mut bytes, record + field, &9999u32.to_le_bytes());
    }
    assert!(matches!(
        ZipArchive::new(Cursor::new(&bytes[..])).err(),
        Some(Error::Corrupt(_))
    ));
    // Stored: the two sizes disagree.
    let mut bytes = small();
    let record = central_record(&bytes, "objects/audio/b.webm");
    patch(&mut bytes, record + 24, &3u32.to_le_bytes());
    assert!(matches!(
        ZipArchive::new(Cursor::new(&bytes[..])).err(),
        Some(Error::Corrupt(_))
    ));
}

#[test]
fn every_truncation_of_the_file_is_refused() {
    let bytes = small();
    for cut in 0..bytes.len() {
        assert!(
            verdict(&bytes[..cut]).is_err(),
            "a file cut to {cut} of {} bytes passed",
            bytes.len()
        );
    }
}

#[test]
fn bytes_lost_from_the_middle_are_refused() {
    let bytes = small();
    let start = data_start(&bytes, "objects/images/a.png");
    for (from, length) in [
        (start, 1),
        (start + 100, 50),
        (start + 2900, 100),
        (10, 1),
        (bytes.len() - 30, 5),
    ] {
        let mut damaged = bytes.clone();
        damaged.drain(from..from + length);
        assert!(
            verdict(&damaged).is_err(),
            "removing {length} at {from} passed"
        );
    }
}

#[test]
fn an_entry_the_directory_places_beyond_the_data_is_refused() {
    let mut bytes = small();
    let record = central_record(&bytes, "keys/key-1.xml");
    patch(&mut bytes, record + 42, &0x00ff_ffffu32.to_le_bytes());
    assert!(verdict(&bytes).is_err());
}

#[test]
fn entries_that_overlap_are_refused() {
    let mut bytes = small();
    let first = u32::from_le_bytes(
        bytes[central_record(&bytes, "database/stellarr.db") + 42..][..4]
            .try_into()
            .unwrap(),
    );
    let record = central_record(&bytes, "objects/images/a.png");
    patch(&mut bytes, record + 42, &first.to_le_bytes());
    assert!(matches!(
        ZipArchive::new(Cursor::new(&bytes[..])).err(),
        Some(Error::Corrupt(_))
    ));
}

#[test]
fn a_local_header_that_names_another_entry_is_refused() {
    let mut bytes = small();
    let record = central_record(&bytes, "objects/audio/b.webm");
    let offset = u32::from_le_bytes(bytes[record + 42..record + 46].try_into().unwrap()) as usize;
    bytes[offset + 30 + 2] ^= 0x20;
    let mut zip = ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    let index = zip.find(b"objects/audio/b.webm").unwrap();
    assert!(matches!(zip.open(index).err(), Some(Error::Corrupt(_))));
    // And one that is no header at all.
    let mut bytes = small();
    bytes[offset] = 0;
    let mut zip = ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    assert!(matches!(zip.open(index).err(), Some(Error::Corrupt(_))));
}

#[test]
fn duplicate_and_missing_entries_are_reported_to_the_caller() {
    let mut zip = ZipWriter::new(Vec::new());
    zip.add_bytes("objects/x", EntryOptions::stored(), b"first")
        .unwrap();
    zip.add_bytes("objects/y", EntryOptions::stored(), b"y")
        .unwrap();
    zip.add_bytes("objects/x", EntryOptions::stored(), b"second")
        .unwrap();
    let bytes = zip.finish().unwrap();
    let mut zip = open(&bytes);
    assert_eq!(zip.duplicate_name(), Some(&b"objects/x"[..]));
    assert_eq!(zip.find(b"objects/x"), Some(0));
    assert_eq!(zip.read_to_vec(2, 100).unwrap(), b"second");
    assert_eq!(zip.find(b"manifest.json"), None);
}

#[test]
fn names_come_back_exactly_as_stored() {
    let names = [
        "objects/../../escape",
        "objects/.incoming/half-written",
        "keys/nested/key.xml",
        "instance/etc/passwd",
        "/absolute",
        "C:\\windows\\x",
        "back\\slash",
        "a//b",
        "./dot",
        "trailing/",
        "",
    ];
    let mut zip = ZipWriter::new(Vec::new());
    for name in names.iter().filter(|n| !n.is_empty()) {
        zip.add_bytes(name, EntryOptions::stored(), b"x").unwrap();
    }
    let bytes = zip.finish().unwrap();
    let zip = open(&bytes);
    let stored: Vec<&[u8]> = zip.entries().iter().map(|e| e.name_bytes()).collect();
    let expected: Vec<&[u8]> = names
        .iter()
        .filter(|n| !n.is_empty())
        .map(|n| n.as_bytes())
        .collect();
    assert_eq!(stored, expected);

    // A NUL and bytes that are not text, which the writer refuses, put in by hand.
    let mut zip = ZipWriter::new(Vec::new());
    zip.add_bytes("aXb", EntryOptions::stored(), b"x").unwrap();
    zip.add_bytes("cYd", EntryOptions::stored(), b"y").unwrap();
    let mut bytes = zip.finish().unwrap();
    let at_a: Vec<usize> = (0..bytes.len() - 3)
        .filter(|&at| bytes[at..at + 3] == *b"aXb")
        .collect();
    let at_c: Vec<usize> = (0..bytes.len() - 3)
        .filter(|&at| bytes[at..at + 3] == *b"cYd")
        .collect();
    assert_eq!(
        (at_a.len(), at_c.len()),
        (2, 2),
        "a name is in the local header and the directory"
    );
    for at in at_a {
        bytes[at + 1] = 0;
    }
    for at in at_c {
        bytes[at + 1] = 0xff;
    }
    let zip = open(&bytes);
    assert_eq!(zip.entries()[0].name_bytes(), b"a\0b");
    assert_eq!(zip.entries()[0].name(), Some("a\0b"));
    assert_eq!(zip.entries()[1].name_bytes(), b"c\xffd");
    assert_eq!(zip.entries()[1].name(), None);
}

#[test]
fn encryption_other_methods_and_several_parts_are_refused_cleanly() {
    let clean = small();
    let record = central_record(&clean, "keys/key-1.xml");

    let mut bytes = clean.clone();
    bytes[record + 8] |= 0x01;
    assert!(matches!(
        ZipArchive::new(Cursor::new(&bytes[..])).err(),
        Some(Error::Unsupported(_))
    ));
    let mut bytes = clean.clone();
    bytes[record + 8] |= 0x40;
    assert!(matches!(
        ZipArchive::new(Cursor::new(&bytes[..])).err(),
        Some(Error::Unsupported(_))
    ));
    for method in [1u16, 9, 12, 14, 93, 99] {
        let mut bytes = clean.clone();
        patch(&mut bytes, record + 10, &method.to_le_bytes());
        assert!(
            matches!(
                ZipArchive::new(Cursor::new(&bytes[..])).err(),
                Some(Error::Unsupported(_))
            ),
            "{method}"
        );
    }

    let end = clean.len() - 22;
    let mut bytes = clean.clone();
    patch(&mut bytes, end + 4, &1u16.to_le_bytes());
    assert!(matches!(
        ZipArchive::new(Cursor::new(&bytes[..])).err(),
        Some(Error::Unsupported(_))
    ));
    let mut bytes = clean.clone();
    patch(&mut bytes, end + 8, &9u16.to_le_bytes());
    assert!(ZipArchive::new(Cursor::new(&bytes[..])).is_err());
    let mut bytes = clean.clone();
    patch(&mut bytes, record + 34, &1u16.to_le_bytes());
    assert!(matches!(
        ZipArchive::new(Cursor::new(&bytes[..])).err(),
        Some(Error::Unsupported(_))
    ));
}

#[test]
fn a_damaged_zip64_end_is_refused() {
    let bytes = common::write_sample(true);
    let locator = bytes.len() - 22 - 20;
    assert_eq!(bytes[locator..locator + 4], [0x50, 0x4b, 0x06, 0x07]);
    // Marker values with no ZIP64 record behind them.
    let mut damaged = bytes.clone();
    let end = bytes.len() - 22;
    patch(&mut damaged, end + 10, &0xffffu16.to_le_bytes());
    patch(&mut damaged, end + 8, &0xffffu16.to_le_bytes());
    assert!(
        ZipArchive::new(Cursor::new(&damaged[..])).is_ok(),
        "the record is there and agrees"
    );
    let mut damaged = bytes.clone();
    damaged[locator] = 0;
    patch(&mut damaged, end + 10, &0xffffu16.to_le_bytes());
    patch(&mut damaged, end + 8, &0xffffu16.to_le_bytes());
    assert!(matches!(
        ZipArchive::new(Cursor::new(&damaged[..])).err(),
        Some(Error::Corrupt(_))
    ));
    // A locator that points somewhere else.
    for target in [0u64, 5, u64::MAX, bytes.len() as u64] {
        let mut damaged = bytes.clone();
        patch(&mut damaged, locator + 8, &target.to_le_bytes());
        patch(&mut damaged, end + 12, &0xffff_ffffu32.to_le_bytes());
        assert!(
            ZipArchive::new(Cursor::new(&damaged[..])).is_err(),
            "{target}"
        );
    }
    // A ZIP64 record that claims more entries than the directory has.
    let record = locator - 56;
    for entries in [0u64, 1, 11, u64::MAX, 1 << 40] {
        let mut damaged = bytes.clone();
        patch(&mut damaged, record + 24, &entries.to_le_bytes());
        patch(&mut damaged, record + 32, &entries.to_le_bytes());
        patch(&mut damaged, end + 12, &0xffff_ffffu32.to_le_bytes());
        assert!(
            ZipArchive::new(Cursor::new(&damaged[..])).is_err(),
            "{entries}"
        );
    }
}

#[test]
fn the_callers_limits_are_enforced() {
    let bytes = small();
    let with = |limits: Limits| ZipArchive::with_limits(Cursor::new(&bytes[..]), limits).err();
    let wide = Limits::default();
    assert!(matches!(
        with(Limits {
            max_entries: 4,
            ..wide
        }),
        Some(Error::TooLarge(_))
    ));
    assert!(
        with(Limits {
            max_entries: 5,
            ..wide
        })
        .is_none()
    );
    assert!(matches!(
        with(Limits {
            max_directory_bytes: 100,
            ..wide
        }),
        Some(Error::TooLarge(_))
    ));
    assert!(matches!(
        with(Limits {
            max_entry_bytes: 29_999,
            ..wide
        }),
        Some(Error::TooLarge(_))
    ));
    assert!(
        with(Limits {
            max_entry_bytes: 30_000,
            ..wide
        })
        .is_none()
    );
    assert!(matches!(
        with(Limits {
            max_total_bytes: 33_000,
            ..wide
        }),
        Some(Error::TooLarge(_))
    ));
    let mut zip = open(&bytes);
    assert!(matches!(
        zip.read_to_vec(0, 100).err(),
        Some(Error::TooLarge(_))
    ));
}

#[test]
fn a_bomb_cannot_unpack_past_what_it_declares() {
    // 64 MB of zeros in a few hundred kilobytes.
    let mut zip = ZipWriter::new(Vec::new());
    let mut entry = zip
        .start_entry("bomb", EntryOptions::deflated(Level::Fast))
        .unwrap();
    let zeros = vec![0u8; 1 << 20];
    for _ in 0..64 {
        std::io::Write::write_all(&mut entry, &zeros).unwrap();
    }
    entry.finish().unwrap();
    let clean = zip.finish().unwrap();
    assert!(clean.len() < 200_000, "{}", clean.len());

    // Honest, it is refused by a cap before anything is read.
    let capped = ZipArchive::with_limits(
        Cursor::new(&clean[..]),
        Limits {
            max_entry_bytes: 1 << 20,
            ..Limits::default()
        },
    );
    assert!(matches!(capped.err(), Some(Error::TooLarge(_))));

    // Lying about its size, it is stopped at the size it declared.
    let mut lying = clean.clone();
    let record = central_record(&lying, "bomb");
    patch(&mut lying, record + 24, &1000u32.to_le_bytes());
    let mut zip = ZipArchive::new(Cursor::new(&lying[..])).unwrap();
    let mut reader = zip.open(0).unwrap();
    let mut got = 0u64;
    let mut buf = [0u8; 4096];
    let error = loop {
        match reader.read(&mut buf) {
            Ok(0) => panic!("the bomb ended quietly"),
            Ok(n) => got += n as u64,
            Err(error) => break Error::from_io(error),
        }
    };
    assert_eq!(got, 1000);
    assert!(matches!(error, Error::Corrupt(_)), "{error:?}");

    // Declaring a size no stream of its length could have is caught at open.
    let mut impossible = clean.clone();
    let record = central_record(&impossible, "bomb");
    patch(&mut impossible, record + 24, &0xffff_fff0u32.to_le_bytes());
    assert!(matches!(
        ZipArchive::new(Cursor::new(&impossible[..])).err(),
        Some(Error::Corrupt(_))
    ));
}

#[test]
fn garbage_and_nonsense_files_are_not_archives() {
    assert!(ZipArchive::new(Cursor::new(Vec::<u8>::new())).is_err());
    assert!(ZipArchive::new(Cursor::new(vec![0u8; 21])).is_err());
    assert!(ZipArchive::new(Cursor::new(vec![0u8; 22])).is_err());
    assert!(ZipArchive::new(Cursor::new(b"SQLite format 3\0".repeat(100))).is_err());
    let mut rng = Rng::new(8);
    for _ in 0..300 {
        let n = rng.below(2000) as usize;
        assert!(ZipArchive::new(Cursor::new(rng.bytes(n))).is_err());
    }
}

#[test]
fn damage_anywhere_never_panics() {
    let clean = small();
    let mut rng = Rng::new(77);
    let mut refused = 0;
    for _ in 0..4000 {
        let mut bytes = clean.clone();
        for _ in 0..1 + rng.below(4) {
            let at = rng.below(bytes.len() as u64) as usize;
            bytes[at] = rng.next() as u8;
        }
        if verdict(&bytes).is_err() {
            refused += 1;
        }
    }
    assert!(refused > 3000, "{refused}");
}

#[test]
fn damage_to_the_directory_alone_never_panics() {
    let clean = small();
    let directory = central_record(&clean, "database/stellarr.db");
    let mut rng = Rng::new(78);
    for _ in 0..4000 {
        let mut bytes = clean.clone();
        let at = directory + rng.below((bytes.len() - directory) as u64) as usize;
        bytes[at] ^= 1 << rng.below(8);
        let _ = verdict(&bytes);
    }
}
