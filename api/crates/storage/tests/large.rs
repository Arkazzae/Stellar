//! Large objects through the stores, with the process's memory watched.
//!
//! These tests move hundreds of megabytes, so they run only when asked:
//!
//! ```text
//! cargo build -p stellarr-storage --example fake_s3
//! cargo test -p stellarr-storage --test large -- --ignored --nocapture
//! ```
//!
//! (The first line builds the server the bucket test starts; a run of
//! `--test large` alone does not build examples, a plain `cargo test` does.)
//!
//! The size is `STELLARR_TEST_LARGE_MIB` (300 by default). The bucket test
//! talks to the fake server as a process of its own (so its memory is not
//! counted in ours), or, with `STELLARR_TEST_S3_ENDPOINT` set, to a real store.

mod support;

use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Instant;

use aws_lc_rs::digest;
use stellarr_outbound::Client;
use stellarr_storage::{
    BucketStore, BucketTuning, ByteRange, FolderStore, ObjectStorage, StorageOptions,
    StorageProvider,
};
use support::TempDir;
use tokio::io::{AsyncRead, ReadBuf};

const MIB: u64 = 1024 * 1024;

fn size() -> u64 {
    std::env::var("STELLARR_TEST_LARGE_MIB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300)
        * MIB
}

/// The resident set of this process, in bytes, as the system's `ps` reports it.
fn resident() -> u64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        * 1024
}

/// Polls the resident set and keeps the highest.
struct Watch {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watch {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(resident()));
        let (s, p) = (stop.clone(), peak.clone());
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                p.fetch_max(resident(), Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });
        Self {
            stop,
            peak,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
        self.peak.fetch_max(resident(), Ordering::Relaxed);
        self.peak.load(Ordering::Relaxed)
    }
}

/// A sequence of bytes that depends only on the seed and the position, so any
/// slice of it can be made again without making what comes before; hashed as
/// it is produced.
struct Generated {
    remaining: u64,
    position: u64,
    seed: u64,
    hash: digest::Context,
}

fn bytes_at(seed: u64, from: u64, out: &mut [u8]) {
    for (n, byte) in out.iter_mut().enumerate() {
        let position = from + n as u64;
        let mut z = seed.wrapping_add((position / 8).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        *byte = z.to_le_bytes()[(position % 8) as usize];
    }
}

impl Generated {
    fn new(length: u64, seed: u64) -> Self {
        Self {
            remaining: length,
            position: 0,
            seed,
            hash: digest::Context::new(&digest::SHA256),
        }
    }

    fn finish(self) -> Vec<u8> {
        self.hash.finish().as_ref().to_vec()
    }
}

impl AsyncRead for Generated {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let n = (buf.remaining() as u64).min(self.remaining).min(256 * 1024) as usize;
        let mut out = vec![0u8; n];
        bytes_at(self.seed, self.position, &mut out);
        self.hash.update(&out);
        self.remaining -= n as u64;
        self.position += n as u64;
        buf.put_slice(&out);
        Poll::Ready(Ok(()))
    }
}

fn hex(bytes: &[u8]) -> String {
    stellarr_s3::internals::hex(bytes)
}

async fn transfer(store: &dyn ObjectStorage, label: &str, length: u64) {
    let key = "large/object.bin";
    let baseline = resident();
    let watch = Watch::start();

    let started = Instant::now();
    let mut source = Generated::new(length, 7);
    store.put(key, &mut source).await.unwrap();
    let wrote = started.elapsed();
    eprintln!(
        "{label}: after the write the resident set is {:.0} MiB",
        resident() as f64 / MIB as f64
    );
    let expected = hex(&source.finish());

    let listed = store.list().collect().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].size, length);

    let started = Instant::now();
    let mut object = store.get(key, None).await.unwrap();
    assert_eq!((object.length, object.total_length), (length, length));
    let mut hash = digest::Context::new(&digest::SHA256);
    let mut read = 0u64;
    let mut largest = 0usize;
    while let Some(piece) = object.body.chunk().await.unwrap() {
        hash.update(&piece);
        read += piece.len() as u64;
        largest = largest.max(piece.len());
    }
    let read_in = started.elapsed();
    assert_eq!(read, length);
    assert_eq!(
        hex(hash.finish().as_ref()),
        expected,
        "the object came back byte for byte"
    );

    // A slice from the middle, checked against the same sequence.
    let from = length / 2 + 12_345;
    let slice = 1_000_003u64;
    let ranged = store
        .get(
            key,
            Some(ByteRange {
                from,
                to: Some(from + slice - 1),
            }),
        )
        .await
        .unwrap();
    assert_eq!(ranged.length, slice);
    assert_eq!(ranged.total_length, length);
    let got = ranged.body.collect(slice).await.unwrap();
    let mut want = vec![0u8; slice as usize];
    bytes_at(7, from, &mut want);
    assert!(
        got.as_ref() == want.as_slice(),
        "the slice is the bytes of that part of the object"
    );

    store.delete(key).await.unwrap();
    assert!(store.list().collect().await.unwrap().is_empty());

    let peak = watch.finish();
    let mib = |b: u64| b as f64 / MIB as f64;
    eprintln!(
        "{label}: {:.0} MiB written in {:.1} s ({:.0} MiB/s), read in {:.1} s ({:.0} MiB/s), largest piece read {} KiB",
        mib(length),
        wrote.as_secs_f64(),
        mib(length) / wrote.as_secs_f64(),
        read_in.as_secs_f64(),
        mib(length) / read_in.as_secs_f64(),
        largest / 1024
    );
    eprintln!(
        "{label}: peak resident {:.0} MiB, {:.0} MiB above the {:.0} MiB before",
        mib(peak),
        mib(peak.saturating_sub(baseline)),
        mib(baseline)
    );
    // The bound is an upper limit for a design that holds one part (8 MiB) or
    // one read buffer, with generous room for the allocator and the test; a
    // design that held the object would be hundreds of MiB above.
    assert!(
        peak.saturating_sub(baseline) < 100 * MIB,
        "memory grew by {:.0} MiB for a {:.0} MiB object",
        mib(peak.saturating_sub(baseline)),
        mib(length)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "moves hundreds of megabytes; run with --ignored"]
async fn the_folder_store_streams_a_large_object() {
    let dir = TempDir::new("large");
    let store = FolderStore::new(dir.path().join("files"));
    transfer(&store, "folder", size()).await;
    assert!(
        std::fs::read_dir(dir.path().join("files/.incoming"))
            .unwrap()
            .next()
            .is_none()
    );
}

struct ChildFake(Child);

impl Drop for ChildFake {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_fake() -> (ChildFake, String) {
    use std::io::{BufRead, BufReader};
    let exe = std::env::current_exe().unwrap();
    let examples = exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/fake_s3");
    let mut child = Command::new(&examples)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| {
            panic!(
                "cannot start {}: {e}; run `cargo build -p stellarr-storage --example fake_s3` first",
                examples.display()
            )
        });
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let endpoint = line
        .trim()
        .strip_prefix("LISTENING ")
        .expect("the fake says where it listens")
        .to_owned();
    (ChildFake(child), endpoint)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "moves hundreds of megabytes; run with --ignored"]
async fn the_bucket_store_streams_a_large_object_through_the_fake() {
    let (_server, endpoint) = spawn_fake();
    let options = StorageOptions {
        provider: StorageProvider::S3,
        endpoint,
        access_key: support::fake_s3::ACCESS_KEY.into(),
        secret_key: support::fake_s3::SECRET_KEY.into(),
        bucket: "large".into(),
    };
    let store = BucketStore::with_tuning(&options, Client::new(), BucketTuning::default()).unwrap();
    store.prepare().await.unwrap();
    transfer(&store, "bucket (fake, own process)", size()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a real store: see tests/servers.sh"]
async fn the_bucket_store_streams_a_large_object_through_a_real_store() {
    let store = support::server::store_from_env("large").await;
    transfer(&store, "bucket (real store)", size()).await;
}
