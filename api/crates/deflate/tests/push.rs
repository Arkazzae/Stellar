//! The push decoder against the pull decoders: the same bytes, or the same
//! error, however the input is cut into pieces.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::Rng;
use stellarr_deflate::{
    Adler32, Crc32, Error, Format, Level, PushDecoder, Step, deflate_to_vec, gunzip_to_vec,
    inflate_to_vec, zlib_to_vec,
};

/// Pushes `pieces` in order, reading into a buffer of `room` bytes, and
/// returns what came out or why it failed.
fn run(format: Format, limit: u64, pieces: &[&[u8]], room: usize) -> Result<Vec<u8>, Error> {
    let mut decoder = PushDecoder::new(format).with_limit(limit);
    let mut out = Vec::new();
    let mut buffer = vec![0u8; room];
    for piece in pieces {
        decoder.push(piece);
        loop {
            match decoder.read(&mut buffer)? {
                Step::Data(n) => out.extend_from_slice(&buffer[..n]),
                Step::NeedInput => break,
                Step::End => return Ok(out),
            }
        }
    }
    decoder.finish();
    loop {
        match decoder.read(&mut buffer)? {
            Step::Data(n) => out.extend_from_slice(&buffer[..n]),
            Step::NeedInput => panic!("asked for input after the end of it was said"),
            Step::End => return Ok(out),
        }
    }
}

fn whole(format: Format, stream: &[u8]) -> Result<Vec<u8>, Error> {
    run(format, u64::MAX, &[stream], 4096)
}

fn drip(format: Format, stream: &[u8]) -> Result<Vec<u8>, Error> {
    let pieces: Vec<&[u8]> = stream.chunks(1).collect();
    run(format, u64::MAX, &pieces, 777)
}

fn pull(format: Format, stream: &[u8]) -> Result<Vec<u8>, Error> {
    match format {
        Format::Raw => inflate_to_vec(stream, u64::MAX),
        Format::Zlib => zlib_to_vec(stream, u64::MAX),
        Format::Gzip => gunzip_to_vec(stream, u64::MAX),
        Format::ZlibOrRaw => unreachable!(),
    }
}

fn zlib_wrap(data: &[u8], level: Level) -> Vec<u8> {
    let mut out = vec![0x78, 0x9c];
    out.extend_from_slice(&deflate_to_vec(data, level));
    let mut adler = Adler32::new();
    adler.update(data);
    out.extend_from_slice(&adler.value().to_be_bytes());
    out
}

fn gzip_wrap_with(header: &[u8], data: &[u8], level: Level) -> Vec<u8> {
    let mut out = header.to_vec();
    out.extend_from_slice(&deflate_to_vec(data, level));
    let mut crc = Crc32::new();
    crc.update(data);
    out.extend_from_slice(&crc.value().to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

const PLAIN_HEADER: [u8; 10] = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];

fn gzip_wrap(data: &[u8], level: Level) -> Vec<u8> {
    gzip_wrap_with(&PLAIN_HEADER, data, level)
}

fn sample(name: &str) -> Vec<u8> {
    common::corpus(1)
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap()
        .1
}

fn small_cases() -> Vec<(&'static str, Vec<u8>)> {
    [
        "empty",
        "one-byte",
        "hello",
        "words-5000",
        "skewed-17",
        "random-1000",
        "zeros-1000",
    ]
    .into_iter()
    .map(|name| (name, sample(name)))
    .collect()
}

#[test]
fn every_two_way_split_gives_what_the_whole_gives() {
    for (name, data) in small_cases() {
        for level in [Level::Fast, Level::Best] {
            let streams = [
                (Format::Raw, deflate_to_vec(&data, level)),
                (Format::Zlib, zlib_wrap(&data, level)),
                (Format::ZlibOrRaw, zlib_wrap(&data, level)),
                (Format::ZlibOrRaw, deflate_to_vec(&data, level)),
                (Format::Gzip, gzip_wrap(&data, level)),
            ];
            for (format, stream) in &streams {
                let step = if stream.len() > 400 { 13 } else { 1 };
                for cut in (0..=stream.len()).step_by(step) {
                    let got = run(*format, u64::MAX, &[&stream[..cut], &stream[cut..]], 300)
                        .unwrap_or_else(|e| panic!("{name} {format:?} cut {cut}: {e}"));
                    assert!(got == data, "{name} {format:?} cut {cut}: bytes differ");
                }
            }
        }
    }
}

#[test]
fn a_drip_of_single_bytes_gives_the_same_for_every_shape_of_data() {
    let corpus = common::corpus(1);
    let mut checked = 0;
    for (name, data) in corpus.iter().filter(|(_, d)| d.len() <= 70_000) {
        let streams = [
            (Format::Raw, deflate_to_vec(data, Level::Default)),
            (Format::Zlib, zlib_wrap(data, Level::Default)),
            (Format::Gzip, gzip_wrap(data, Level::Fast)),
        ];
        for (format, stream) in &streams {
            let got = drip(*format, stream).unwrap_or_else(|e| panic!("{name} {format:?}: {e}"));
            assert!(got == *data, "{name} {format:?}: bytes differ");
            checked += 1;
        }
    }
    assert!(checked > 100);
}

#[test]
fn big_inputs_in_odd_pieces_and_small_output_buffers() {
    let mut rng = Rng::new(7);
    for name in ["big-mixed-700000", "big-words-1500000"] {
        let data = sample(name);
        let stream = gzip_wrap(&data, Level::Default);
        let mut pieces: Vec<&[u8]> = Vec::new();
        let mut at = 0;
        while at < stream.len() {
            let n = (1 + rng.below(40_000) as usize).min(stream.len() - at);
            pieces.push(&stream[at..at + n]);
            at += n;
        }
        for room in [1usize, 3, 4096, 200_000] {
            // One byte at a time is slow; it only gets the smaller input.
            if room == 1 && data.len() > 800_000 {
                continue;
            }
            let got = run(Format::Gzip, u64::MAX, &pieces, room).unwrap();
            assert!(got == data, "{name} room {room}");
        }
    }
}

fn python(args: &[&str]) -> Option<()> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/interop.py");
    match Command::new("python3")
        .arg("-I")
        .arg(&script)
        .args(args)
        .output()
    {
        Ok(output) => {
            assert!(output.status.success(), "python failed");
            Some(())
        }
        Err(_) => {
            eprintln!("python3 is not available: interoperability test skipped");
            None
        }
    }
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn what_zlib_and_gzip_write_is_read_when_pushed_in_pieces() {
    let dir = std::env::temp_dir().join(format!("stellarr-deflate-push-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let scratch = Scratch(dir);
    let corpus: Vec<_> = common::corpus(1)
        .into_iter()
        .filter(|(_, d)| d.len() <= 100_000)
        .collect();
    for (name, data) in &corpus {
        fs::write(scratch.0.join(format!("{name}.raw")), data).unwrap();
    }
    if python(&["compress", scratch.0.to_str().unwrap()]).is_none() {
        return;
    }
    let mut rng = Rng::new(11);
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
            let formats: &[Format] = if kind.ends_with(".def") {
                &[Format::Raw, Format::ZlibOrRaw]
            } else if kind == "zlib" {
                &[Format::Zlib, Format::ZlibOrRaw]
            } else {
                &[Format::Gzip]
            };
            for format in formats {
                let mut pieces: Vec<&[u8]> = Vec::new();
                let mut at = 0;
                while at < stream.len() {
                    let n = (1 + rng.below(2000) as usize).min(stream.len() - at);
                    pieces.push(&stream[at..at + n]);
                    at += n;
                }
                let got = run(*format, u64::MAX, &pieces, 1 + rng.below(5000) as usize)
                    .unwrap_or_else(|e| panic!("{file} {format:?}: {e}"));
                assert!(got == *data, "{file} {format:?}: bytes differ");
                streams += 1;
            }
        }
    }
    eprintln!("{streams} streams written by zlib, read pushed in pieces");
    assert!(streams > corpus.len() * 15);
}

#[test]
fn damaged_and_cut_streams_fail_the_way_the_pull_decoders_do() {
    let mut rng = Rng::new(31);
    let mut caught = 0;
    for (name, data) in small_cases() {
        let streams = [
            (Format::Raw, deflate_to_vec(&data, Level::Default)),
            (Format::Zlib, zlib_wrap(&data, Level::Default)),
            (Format::Gzip, gzip_wrap(&data, Level::Default)),
        ];
        for (format, stream) in &streams {
            // Every strict prefix.
            let step = if stream.len() > 400 { 11 } else { 1 };
            for cut in (0..stream.len()).step_by(step) {
                let expected = pull(*format, &stream[..cut]);
                let a = run(*format, u64::MAX, &[&stream[..cut]], 512);
                let b = drip(*format, &stream[..cut]);
                assert_eq!(a, expected, "{name} {format:?} prefix {cut}");
                assert_eq!(b, expected, "{name} {format:?} drip of prefix {cut}");
                assert!(a.is_err(), "{name} {format:?}: a prefix was accepted");
            }
            // Flipped bits.
            for _ in 0..150 {
                let mut damaged = stream.clone();
                let at = rng.below(damaged.len() as u64) as usize;
                damaged[at] ^= 1 << rng.below(8);
                let expected = pull(*format, &damaged);
                let cut = rng.below(damaged.len() as u64 + 1) as usize;
                let got = run(*format, u64::MAX, &[&damaged[..cut], &damaged[cut..]], 700);
                assert_eq!(got, expected, "{name} {format:?} flip at {at}, cut {cut}");
                if expected.is_err() {
                    caught += 1;
                }
            }
        }
    }
    assert!(caught > 500, "only {caught} damaged streams were noticed");
}

#[test]
fn noise_never_panics_or_hangs_whatever_the_pieces() {
    let mut rng = Rng::new(404);
    for _ in 0..5000 {
        let n = rng.below(200) as usize;
        let noise = rng.bytes(n);
        for format in [Format::Raw, Format::Zlib, Format::Gzip, Format::ZlibOrRaw] {
            let _ = run(format, 1 << 20, &[&noise[..n / 2], &noise[n / 2..]], 64);
        }
    }
}

#[test]
fn zlib_or_raw_is_decided_by_the_first_two_bytes_wherever_they_are_cut() {
    let data = sample("words-5000");
    let raw = deflate_to_vec(&data, Level::Default);
    let zlib = zlib_wrap(&data, Level::Best);
    for stream in [&raw, &zlib] {
        let got = run(
            Format::ZlibOrRaw,
            u64::MAX,
            &[&stream[..1], &stream[1..2], &stream[2..]],
            1000,
        )
        .unwrap();
        assert!(got == data);
    }
    // One byte of input is never enough, whichever way it would have gone.
    assert_eq!(
        run(Format::ZlibOrRaw, u64::MAX, &[&zlib[..1]], 10),
        Err(Error::Truncated)
    );
    assert_eq!(
        run(Format::ZlibOrRaw, u64::MAX, &[], 10),
        Err(Error::Truncated)
    );
    // A zlib header with a preset dictionary is zlib, and refused as that.
    let with_dictionary = [0x78u8, 0xbb, 0, 0, 0, 0, 0];
    assert!(matches!(
        whole(Format::ZlibOrRaw, &with_dictionary),
        Err(Error::BadHeader(_))
    ));
}

#[test]
fn every_optional_gzip_header_field_is_read_wherever_the_input_is_cut() {
    let data = sample("hello");
    // FHCRC | FEXTRA | FNAME | FCOMMENT, in the order the format gives them.
    let mut header = vec![0x1f, 0x8b, 8, 2 | 4 | 8 | 16, 1, 2, 3, 4, 0, 3];
    header.extend_from_slice(&[5, 0, b'a', b'b', b'c', b'd', b'e']);
    header.extend_from_slice(b"name.txt\0");
    header.extend_from_slice(b"a comment\0");
    let mut crc = Crc32::new();
    crc.update(&header);
    header.extend_from_slice(&((crc.value() & 0xffff) as u16).to_le_bytes());
    let stream = gzip_wrap_with(&header, &data, Level::Default);
    assert_eq!(gunzip_to_vec(&stream, u64::MAX).unwrap(), data);
    for cut in 0..=stream.len() {
        let got = run(
            Format::Gzip,
            u64::MAX,
            &[&stream[..cut], &stream[cut..]],
            50,
        )
        .unwrap();
        assert_eq!(got, data, "cut {cut}");
    }
    // A wrong header checksum is refused.
    let mut wrong = stream.clone();
    let at = header.len() - 1;
    wrong[at] ^= 1;
    assert!(matches!(
        whole(Format::Gzip, &wrong),
        Err(Error::BadHeader(_))
    ));
    // An empty extra field, and a name that is long: no limit on a header
    // that nothing is kept of.
    let mut long = vec![0x1f, 0x8b, 8, 4 | 8, 0, 0, 0, 0, 0, 3, 0, 0];
    long.extend(std::iter::repeat_n(b'x', 300_000));
    long.push(0);
    let stream = gzip_wrap_with(&long, &data, Level::Default);
    assert_eq!(whole(Format::Gzip, &stream).unwrap(), data);
    assert_eq!(drip(Format::Gzip, &stream).unwrap(), data);
}

#[test]
fn what_follows_the_stream_is_decided_the_same_way_as_by_the_pull_readers() {
    let data = sample("hello");
    // Raw and zlib: anything after the end is refused.
    for (format, mut stream) in [
        (Format::Raw, deflate_to_vec(&data, Level::Default)),
        (Format::Zlib, zlib_wrap(&data, Level::Default)),
        (Format::ZlibOrRaw, zlib_wrap(&data, Level::Default)),
    ] {
        stream.push(b'\n');
        assert_eq!(
            whole(format, &stream),
            Err(Error::TrailingData),
            "{format:?}"
        );
        assert_eq!(
            drip(format, &stream),
            Err(Error::TrailingData),
            "{format:?}"
        );
    }
    // Gzip: another member is more data, anything else is trailing junk.
    let one = gzip_wrap(&data, Level::Default);
    let mut two = one.clone();
    two.extend_from_slice(&gzip_wrap(b" again", Level::Fast));
    let mut expected = data.clone();
    expected.extend_from_slice(b" again");
    assert_eq!(whole(Format::Gzip, &two).unwrap(), expected);
    assert_eq!(drip(Format::Gzip, &two).unwrap(), expected);
    let mut junk = one.clone();
    junk.extend_from_slice(b"\n\n");
    assert_eq!(whole(Format::Gzip, &junk), Err(Error::TrailingData));
    assert_eq!(drip(Format::Gzip, &junk), Err(Error::TrailingData));
    // A second member that is cut short is truncated, not junk.
    let mut cut = one.clone();
    cut.extend_from_slice(&PLAIN_HEADER[..6]);
    assert_eq!(whole(Format::Gzip, &cut), Err(Error::Truncated));
}

#[test]
fn the_limit_holds_for_every_format_and_every_cut() {
    let data = vec![0u8; 3_000_000];
    let streams = [
        (Format::Raw, deflate_to_vec(&data, Level::Default)),
        (Format::Zlib, zlib_wrap(&data, Level::Default)),
        (Format::Gzip, gzip_wrap(&data, Level::Default)),
    ];
    for (format, stream) in &streams {
        for limit in [0u64, 1, 1000, 65_536, 2_999_999] {
            let mut decoder = PushDecoder::new(*format).with_limit(limit);
            let mut produced = 0u64;
            let mut buffer = vec![0u8; 10_000];
            let mut result = Ok(());
            'feed: for piece in stream.chunks(500) {
                decoder.push(piece);
                loop {
                    match decoder.read(&mut buffer) {
                        Ok(Step::Data(n)) => produced += n as u64,
                        Ok(Step::NeedInput) => break,
                        Ok(Step::End) => break 'feed,
                        Err(error) => {
                            result = Err(error);
                            break 'feed;
                        }
                    }
                }
            }
            if result.is_ok() {
                // The last symbols wait for the end of the input to be said.
                decoder.finish();
                loop {
                    match decoder.read(&mut buffer) {
                        Ok(Step::Data(n)) => produced += n as u64,
                        Ok(Step::NeedInput | Step::End) => break,
                        Err(error) => {
                            result = Err(error);
                            break;
                        }
                    }
                }
            }
            assert_eq!(result, Err(Error::OutputLimit), "{format:?} limit {limit}");
            assert_eq!(produced, limit, "{format:?}: output stops at the limit");
        }
        // Exactly the size is allowed.
        let got = run(*format, 3_000_000, &[stream], 65_536).unwrap();
        assert_eq!(got.len(), 3_000_000);
    }
}

#[test]
fn an_error_is_repeated_and_nothing_follows_the_end() {
    let mut decoder = PushDecoder::new(Format::Gzip);
    decoder.push(b"not gzip at all");
    let mut buffer = [0u8; 16];
    let first = decoder.read(&mut buffer).unwrap_err();
    assert_eq!(decoder.read(&mut buffer), Err(first));
    decoder.push(b"more");
    assert_eq!(decoder.read(&mut buffer), Err(first));

    let mut decoder = PushDecoder::new(Format::Zlib);
    decoder.push(&zlib_wrap(b"x", Level::Default));
    decoder.finish();
    assert_eq!(decoder.read(&mut buffer), Ok(Step::Data(1)));
    assert_eq!(decoder.read(&mut buffer), Ok(Step::End));
    assert_eq!(decoder.read(&mut buffer), Ok(Step::End));
}
