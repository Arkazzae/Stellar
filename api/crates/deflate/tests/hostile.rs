//! Streams that are wrong, on purpose. Every one has to come back as an
//! error, none may panic or hang, and a bomb has to stop at the cap.

mod common;

use std::io::Read;

use common::{BitWriter, Rng};
use stellarr_deflate::{
    Error, Inflate, Level, deflate_to_vec, gunzip_to_vec, inflate_to_vec, zlib_to_vec,
};

fn refused(stream: &[u8]) -> Error {
    match inflate_to_vec(stream, 1 << 24) {
        Ok(out) => panic!("accepted a bad stream, decoded {} bytes", out.len()),
        Err(error) => error,
    }
}

/// A dynamic block header with the given code-length code lengths (in the
/// stream's order) and then the given literal/length symbols, each written
/// with `bits` bits as a plain number.
fn dynamic_header(hlit: u32, hdist: u32, hclen: u32, cl_lengths: &[u32]) -> BitWriter {
    let mut w = BitWriter::default();
    w.bits(1, 1)
        .bits(2, 2)
        .bits(hlit, 5)
        .bits(hdist, 5)
        .bits(hclen, 4);
    for &l in cl_lengths {
        w.bits(l, 3);
    }
    w
}

#[test]
fn reserved_block_type() {
    assert_eq!(refused(&[0b0000_0111]), Error::InvalidBlockType);
}

#[test]
fn stored_length_must_match_its_complement() {
    assert_eq!(
        refused(&[1, 5, 0, 0, 0, b'h', b'e', b'l', b'l', b'o']),
        Error::StoredLengthMismatch
    );
    // The well-formed one passes.
    assert_eq!(
        inflate_to_vec(&[1, 5, 0, 0xfa, 0xff, b'h', b'e', b'l', b'l', b'o'], 10).unwrap(),
        b"hello"
    );
}

#[test]
fn a_stored_block_cut_short() {
    assert_eq!(
        refused(&[1, 5, 0, 0xfa, 0xff, b'h', b'e']),
        Error::Truncated
    );
}

#[test]
fn a_distance_beyond_the_start_of_the_output() {
    // Fixed block: literal 'a', then length 3 at distance 5.
    let mut w = BitWriter::default();
    w.bits(1, 1).bits(1, 2);
    w.code(0x30 + u32::from(b'a'), 8); // literal 'a'
    w.code(1, 7); // length symbol 257: length 3
    w.code(4, 5); // distance symbol 4: 5..6, one extra bit
    w.bits(0, 1);
    w.code(0, 7); // end of block
    assert_eq!(refused(&w.finish()), Error::DistanceTooFar);
}

#[test]
fn a_distance_that_would_be_fine_is_accepted() {
    let mut w = BitWriter::default();
    w.bits(1, 1).bits(1, 2);
    w.code(0x30 + u32::from(b'a'), 8);
    w.code(1, 7); // length 3
    w.code(0, 5); // distance 1
    w.code(0, 7);
    assert_eq!(inflate_to_vec(&w.finish(), 10).unwrap(), b"aaaa");
}

#[test]
fn symbols_the_format_reserves() {
    // Length symbol 286, and distance symbol 30, in a fixed block.
    let mut w = BitWriter::default();
    w.bits(1, 1).bits(1, 2);
    w.code(0xc0 + 6, 8); // symbol 286
    assert_eq!(refused(&w.finish()), Error::InvalidSymbol);

    let mut w = BitWriter::default();
    w.bits(1, 1).bits(1, 2);
    w.code(0x30 + u32::from(b'a'), 8);
    w.code(1, 7);
    w.code(30, 5); // distance symbol 30
    assert_eq!(refused(&w.finish()), Error::InvalidSymbol);
}

#[test]
fn too_many_codes_announced() {
    // HLIT of 30 means 287 length codes; the format has 286.
    let mut w = dynamic_header(30, 0, 0, &[0, 0, 0, 0]);
    assert_eq!(refused(&w.finish()), Error::TooManySymbols);
    // HDIST of 30 means 31 distance codes; the format has 30.
    let mut w = dynamic_header(0, 30, 0, &[0, 0, 0, 0]);
    assert_eq!(refused(&w.finish()), Error::TooManySymbols);
}

#[test]
fn an_oversubscribed_code_length_code() {
    // Three symbols of length 1 cannot all have a code.
    let mut w = dynamic_header(0, 0, 0, &[1, 1, 1, 0]);
    assert_eq!(refused(&w.finish()), Error::OversubscribedCode);
}

#[test]
fn an_incomplete_code_length_code() {
    // A single code of length 2 leaves most bit patterns unassigned.
    let mut w = dynamic_header(0, 0, 0, &[2, 0, 0, 0]);
    assert_eq!(refused(&w.finish()), Error::IncompleteCode);
    // No codes at all.
    let mut w = dynamic_header(0, 0, 0, &[0, 0, 0, 0]);
    assert_eq!(refused(&w.finish()), Error::BadCodeLengths);
}

/// A dynamic block whose code-length code gives symbols 0 and 8 one bit each
/// (code 0 = length 0, code 1 = length 8), then lets the test spell out the
/// literal/length and distance lengths with those two codes.
fn two_symbol_header(lengths: &[u8], hlit: u32, hdist: u32) -> BitWriter {
    // Order is 16, 17, 18, 0, 8: so lengths for 0 and 8 are the 4th and 5th.
    let mut w = dynamic_header(hlit, hdist, 1, &[0, 0, 0, 1, 1]);
    for &l in lengths {
        w.code(u32::from(l == 8), 1);
    }
    w
}

#[test]
fn an_oversubscribed_literal_table() {
    // 257 literal codes of length 8 are too many for an 8-bit code; the
    // distance table is one code of length 8.
    let mut lengths = vec![8u8; 257];
    lengths.push(0);
    let mut w = two_symbol_header(&lengths, 0, 0);
    assert_eq!(refused(&w.finish()), Error::OversubscribedCode);
}

#[test]
fn an_incomplete_literal_table() {
    // Two codes of length 8 where there is room for 256.
    let mut lengths = vec![0u8; 258];
    lengths[0] = 8;
    lengths[256] = 8;
    let mut w = two_symbol_header(&lengths, 0, 0);
    assert_eq!(refused(&w.finish()), Error::IncompleteCode);
}

#[test]
fn a_literal_table_without_an_end_of_block_code() {
    // 256 literal codes of length 8: complete, but no symbol 256.
    let mut lengths = vec![8u8; 256];
    lengths.push(0);
    lengths.push(0);
    let mut w = two_symbol_header(&lengths, 0, 0);
    assert_eq!(refused(&w.finish()), Error::BadCodeLengths);
}

#[test]
fn repeat_codes_that_repeat_nothing_or_too_much() {
    // The code-length code: symbol 16 and symbol 0 of length 1 each.
    let mut w = dynamic_header(0, 0, 0, &[1, 0, 0, 1]);
    w.code(0, 1); // symbol 16 (code 0 of the two sorted: 0 then 16 -> 0 is first)
    // Whichever of the two symbols the bit means, a repeat at the start or a
    // run past the end is refused, and nothing here may decode.
    w.bits(0, 2);
    assert!(inflate_to_vec(&w.finish(), 100).is_err());

    // A zero run of 138 with only 258 lengths announced, repeated past the end.
    let mut w = dynamic_header(0, 0, 0, &[0, 0, 1, 1]);
    for _ in 0..4 {
        w.code(0, 1); // symbol 18 or 0, depending on the order of codes
        w.bits(127, 7);
    }
    assert!(inflate_to_vec(&w.finish(), 100).is_err());
}

#[test]
fn every_strict_prefix_of_a_valid_stream_is_an_error() {
    let corpus = common::corpus(1);
    for name in [
        "hello",
        "words-5000",
        "random-4096",
        "zeros-1000",
        "skewed-17",
    ] {
        let data = &corpus.iter().find(|(n, _)| n == name).unwrap().1;
        let stream = deflate_to_vec(data, Level::Default);
        for cut in 0..stream.len() {
            match inflate_to_vec(&stream[..cut], 1 << 24) {
                Err(_) => {}
                Ok(_) => panic!(
                    "{name}: a prefix of {cut} of {} bytes was accepted",
                    stream.len()
                ),
            }
        }
    }
}

#[test]
fn damaged_streams_never_panic() {
    let corpus = common::corpus(1);
    let mut rng = Rng::new(99);
    let mut caught = 0;
    for name in [
        "words-5000",
        "records-5000",
        "mixed-5000",
        "skewed-17",
        "hello",
    ] {
        let data = &corpus.iter().find(|(n, _)| n == name).unwrap().1;
        for level in [Level::Fast, Level::Best] {
            let stream = deflate_to_vec(data, level);
            for _ in 0..1500 {
                let mut damaged = stream.clone();
                for _ in 0..1 + rng.below(3) {
                    let at = rng.below(damaged.len() as u64) as usize;
                    damaged[at] ^= 1 << rng.below(8);
                }
                if inflate_to_vec(&damaged, 1 << 22).is_err() {
                    caught += 1;
                }
            }
        }
    }
    assert!(
        caught > 1000,
        "only {caught} of 15000 damaged streams were noticed"
    );
}

#[test]
fn random_noise_never_panics_or_hangs() {
    let mut rng = Rng::new(4242);
    for _ in 0..20_000 {
        let n = rng.below(200) as usize;
        let noise = rng.bytes(n);
        let _ = inflate_to_vec(&noise, 1 << 20);
    }
}

#[test]
fn a_bomb_stops_at_the_cap() {
    // A gigabyte of zeros is a few kilobytes of stream.
    let zeros = vec![0u8; 64 << 20];
    let stream = deflate_to_vec(&zeros, Level::Fast);
    assert!(stream.len() < 100_000, "{}", stream.len());
    assert_eq!(inflate_to_vec(&stream, 1 << 20), Err(Error::OutputLimit));

    // The reader gives what is within the cap, then the error, and never more.
    let mut decoder = Inflate::new(&stream[..]).with_limit(1_000_000);
    let mut got = 0u64;
    let mut buf = [0u8; 4096];
    let error = loop {
        match decoder.read(&mut buf) {
            Ok(0) => panic!("the bomb ended"),
            Ok(n) => got += n as u64,
            Err(error) => break error,
        }
    };
    assert_eq!(got, 1_000_000);
    assert_eq!(Error::from_io(&error), Some(Error::OutputLimit));
    // And it stays an error.
    assert!(decoder.read(&mut buf).is_err());
}

#[test]
fn a_stored_bomb_is_refused_before_it_is_copied() {
    let mut stream = vec![1u8, 0xff, 0xff, 0, 0];
    stream.extend(vec![9u8; 65_535]);
    assert_eq!(inflate_to_vec(&stream, 1000), Err(Error::OutputLimit));
}

#[test]
fn gzip_and_zlib_headers_and_trailers_are_checked() {
    let data = b"hello hello hello hello hello".to_vec();
    let body = deflate_to_vec(&data, Level::Default);

    let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    gz.extend_from_slice(&body);
    gz.extend_from_slice(&stellarr_deflate::crc32(&data).to_le_bytes());
    gz.extend_from_slice(&(data.len() as u32).to_le_bytes());
    assert_eq!(gunzip_to_vec(&gz, 1000).unwrap(), data);

    let mut bad = gz.clone();
    let last = bad.len() - 8;
    bad[last] ^= 1;
    assert_eq!(gunzip_to_vec(&bad, 1000), Err(Error::ChecksumMismatch));
    let mut bad = gz.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(gunzip_to_vec(&bad, 1000), Err(Error::LengthMismatch));
    let mut bad = gz.clone();
    bad[0] = 0x1e;
    assert!(matches!(
        gunzip_to_vec(&bad, 1000),
        Err(Error::BadHeader(_))
    ));
    let mut bad = gz.clone();
    bad[3] = 0x20;
    assert!(matches!(
        gunzip_to_vec(&bad, 1000),
        Err(Error::BadHeader(_))
    ));
    assert_eq!(
        gunzip_to_vec(&gz[..gz.len() - 3], 1000),
        Err(Error::Truncated)
    );
    assert_eq!(gunzip_to_vec(&[], 1000), Err(Error::Truncated));
    let mut junk = gz.clone();
    junk.extend_from_slice(b"junk");
    assert!(gunzip_to_vec(&junk, 1000).is_err());
    assert_eq!(gunzip_to_vec(&gz, 10), Err(Error::OutputLimit));

    // Two members in a row are one stream; the cap covers both together.
    let mut two = gz.clone();
    two.extend_from_slice(&gz);
    assert_eq!(gunzip_to_vec(&two, 1000).unwrap().len(), 2 * data.len());
    assert_eq!(
        gunzip_to_vec(&two, data.len() as u64 + 1),
        Err(Error::OutputLimit)
    );

    let mut zl = vec![0x78, 0x9c];
    zl.extend_from_slice(&body);
    let mut adler = stellarr_deflate::Adler32::new();
    adler.update(&data);
    zl.extend_from_slice(&adler.value().to_be_bytes());
    assert_eq!(zlib_to_vec(&zl, 1000).unwrap(), data);
    let mut bad = zl.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(zlib_to_vec(&bad, 1000), Err(Error::ChecksumMismatch));
    let mut bad = zl.clone();
    bad[1] = 0x9d;
    assert!(matches!(zlib_to_vec(&bad, 1000), Err(Error::BadHeader(_))));
    let mut dict = zl.clone();
    dict[0] = 0x78;
    dict[1] = 0xbb; // FDICT set, check bits right
    assert!(matches!(zlib_to_vec(&dict, 1000), Err(Error::BadHeader(_))));
    let mut junk = zl.clone();
    junk.push(0);
    assert_eq!(zlib_to_vec(&junk, 1000), Err(Error::TrailingData));
    assert_eq!(
        zlib_to_vec(&zl[..zl.len() - 2], 1000),
        Err(Error::Truncated)
    );
}
