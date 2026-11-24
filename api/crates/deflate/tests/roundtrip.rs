//! The encoder against the decoder, both written here, over many inputs.

mod common;

use std::io::{Read, Write};

use stellarr_deflate::{Deflater, Inflate, Level, deflate_to_vec, inflate_to_vec};

#[test]
fn every_input_survives_every_level() {
    for (name, data) in common::corpus(1) {
        for level in [Level::Fast, Level::Default, Level::Best] {
            let packed = deflate_to_vec(&data, level);
            let back = inflate_to_vec(&packed, u64::MAX)
                .unwrap_or_else(|e| panic!("{name} {level:?}: {e}"));
            assert!(back == data, "{name} {level:?}: round trip differs");
        }
    }
}

#[test]
fn chunked_writes_and_sync_flushes_do_not_change_the_data() {
    let mut rng = common::Rng::new(7);
    for (name, data) in common::corpus(1).into_iter().filter(|(_, d)| d.len() > 100) {
        let mut deflater = Deflater::new(Vec::new(), Level::Default);
        let mut at = 0;
        let mut flushed_at = Vec::new();
        while at < data.len() {
            let take = (1 + rng.below(9000) as usize).min(data.len() - at);
            deflater.write_all(&data[at..at + take]).unwrap();
            at += take;
            if rng.below(7) == 0 {
                deflater.flush().unwrap();
                flushed_at.push((at, deflater.get_ref().len()));
            }
        }
        let packed = deflater.finish().unwrap();
        // After a sync flush, what was written so far must decode on its own.
        for &(upto, written) in flushed_at.iter().take(3) {
            let mut decoder = Inflate::new(&packed[..written]);
            let mut out = vec![0u8; upto];
            decoder
                .read_exact(&mut out)
                .unwrap_or_else(|e| panic!("{name}: prefix at {upto}: {e}"));
            assert!(out == data[..upto], "{name}: prefix at {upto}");
        }
        assert!(inflate_to_vec(&packed, u64::MAX).unwrap() == data, "{name}");
    }
}

#[test]
fn structured_data_shrinks_and_noise_barely_grows() {
    let zeros = deflate_to_vec(&vec![0u8; 1_000_000], Level::Default);
    assert!(zeros.len() < 2000, "{}", zeros.len());
    let text = common::corpus(1)
        .into_iter()
        .find(|(n, _)| n == "words-400000")
        .unwrap()
        .1;
    for level in [Level::Fast, Level::Default, Level::Best] {
        let packed = deflate_to_vec(&text, level);
        assert!(
            packed.len() * 3 < text.len(),
            "{level:?}: {} of {}",
            packed.len(),
            text.len()
        );
    }
    let noise = common::Rng::new(1).bytes(200_000);
    let packed = deflate_to_vec(&noise, Level::Default);
    // Stored blocks cost five bytes per 65,000.
    assert!(packed.len() <= noise.len() + 64, "{}", packed.len());
}

#[test]
fn an_empty_stream_is_the_two_standard_bytes() {
    assert_eq!(deflate_to_vec(b"", Level::Default), [0x03, 0x00]);
}

#[test]
fn the_limit_is_exact() {
    let data = vec![7u8; 10_000];
    let packed = deflate_to_vec(&data, Level::Default);
    assert_eq!(inflate_to_vec(&packed, 10_000).unwrap().len(), 10_000);
    assert!(inflate_to_vec(&packed, 9_999).is_err());
}

#[test]
fn a_slow_source_still_delivers_what_it_has() {
    // Decoded bytes are handed out before the decoder waits for more input.
    struct Dribble<'a>(&'a [u8], usize);
    impl Read for Dribble<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.1.min(self.0.len()).min(buf.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }
    let data = common::corpus(1)
        .into_iter()
        .find(|(n, _)| n == "records-70000")
        .unwrap()
        .1;
    let packed = deflate_to_vec(&data, Level::Default);
    for step in [1usize, 3, 100] {
        let mut out = Vec::new();
        Inflate::new(Dribble(&packed, step))
            .read_to_end(&mut out)
            .unwrap();
        assert!(out == data, "step {step}");
    }
}

#[test]
fn the_encoder_uses_all_three_block_types() {
    let corpus = common::corpus(1);
    let first_block_type = |name: &str| {
        let data = &corpus.iter().find(|(n, _)| n == name).unwrap().1;
        (deflate_to_vec(data, Level::Default)[0] >> 1) & 3
    };
    assert_eq!(first_block_type("random-200000"), 0, "stored");
    assert_eq!(first_block_type("hello"), 1, "fixed");
    assert_eq!(first_block_type("words-70000"), 2, "dynamic");
    // The input that needs code lengths limited to 15 is a dynamic block too.
    assert_eq!(first_block_type("fibonacci-26"), 2, "dynamic");
}

/// Prints how fast each level runs and how small it makes things; run with
/// `--release -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn throughput() {
    let corpus = common::corpus(4);
    for name in ["big-words-6000000", "big-mixed-2800000", "records-400000"] {
        let Some((_, data)) = corpus.iter().find(|(n, _)| n == name) else {
            continue;
        };
        for level in [Level::Fast, Level::Default, Level::Best] {
            let start = std::time::Instant::now();
            let packed = deflate_to_vec(data, level);
            let packing = start.elapsed();
            let start = std::time::Instant::now();
            let back = inflate_to_vec(&packed, u64::MAX).unwrap();
            let unpacking = start.elapsed();
            assert!(back == *data);
            let mb = data.len() as f64 / 1e6;
            eprintln!(
                "{name} {level:?}: {} -> {} bytes, pack {:.0} MB/s, unpack {:.0} MB/s",
                data.len(),
                packed.len(),
                mb / packing.as_secs_f64(),
                mb / unpacking.as_secs_f64()
            );
        }
    }
}
