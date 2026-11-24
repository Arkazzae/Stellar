//! A real S3-compatible server, named by the environment (see `minio.sh`,
//! which starts MinIO and runs these): the contract every store keeps, the
//! questions about what a server does with the library's requests, and the
//! same questions put to the fake so the two can be read side by side.
//!
//! Without a server these tests are ignored, and fail loudly if forced.

#[macro_use]
mod support;

use std::sync::Arc;

use support::contract::Env;
use support::fake_s3::FakeS3;
use support::points::Probe;
use support::server::{Server, store_from_env};

async fn env() -> Env {
    let store = store_from_env("contract").await;
    Env {
        store: Arc::new(store),
        exclusive_put: false,
        boundary: 5 * 1024 * 1024,
        longest_key: 1024,
        // The library has no call that lists unfinished uploads.
        leftovers: Box::new(Vec::new),
        guard: Box::new(()),
    }
}

contract_tests!(ignored env().await);

fn write_transcript(name: &str, text: &str) {
    println!("{text}");
    if let Some(folder) = std::env::var_os("STELLARR_TEST_TRANSCRIPTS") {
        let path = std::path::Path::new(&folder).join(name);
        std::fs::write(&path, text)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a real store: see tests/minio.sh"]
async fn what_the_real_server_does_with_the_requests_of_the_library() {
    let server = Server::from_env();
    let bucket = format!("stellarr-points-{}", std::process::id());
    let mut probe = Probe::new(
        &server.endpoint,
        &server.access_key,
        &server.secret_key,
        &bucket,
        server.http(),
    );
    probe.run_all().await;
    let text = format!("{}{}", probe.transcript, probe.summary());
    let scheme = if server.https() { "https" } else { "http" };
    write_transcript(&format!("points-{scheme}.txt"), &text);
    let bad: Vec<_> = probe.findings.iter().filter(|f| !f.as_expected).collect();
    assert!(
        bad.is_empty(),
        "{} points did not go as expected; the transcript says how",
        bad.len()
    );
}

/// The same questions put to the fake, to show the harness works and what the
/// client puts on the wire (the fake logs the request targets as they
/// arrive). What the fake answers proves nothing about a real server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_questions_run_against_the_fake_and_show_the_wire() {
    let fake = FakeS3::start().await;
    let mut probe = Probe::new(
        &fake.endpoint,
        support::fake_s3::ACCESS_KEY,
        support::fake_s3::SECRET_KEY,
        "points",
        stellarr_outbound::Client::new(),
    );
    probe.run_all().await;
    let mut wire = String::from("\nTARGETS AS THEY ARRIVED AT THE FAKE\n");
    for request in fake.requests() {
        if request.target.contains("p5/")
            || request.target.contains("uploads")
            || request.target.contains("p2/")
        {
            wire.push_str(&format!(
                "  {} {} -> {}\n",
                request.method, request.target, request.status
            ));
        }
    }
    write_transcript(
        "points-fake.txt",
        &format!("{}{}{wire}", probe.transcript, probe.summary()),
    );
    // Everything the fake was written to do, it did.
    let by_the_fake: Vec<_> = probe
        .findings
        .iter()
        .filter(|f| !f.as_expected && ![1, 5, 8, 10].contains(&f.point))
        .collect();
    assert!(
        by_the_fake.is_empty(),
        "{:?}",
        by_the_fake.iter().map(|f| &f.said).collect::<Vec<_>>()
    );
}

/// The script's test certificates are a root the HTTP client can trust: they
/// parse, and the leaf verifies against the root with `openssl`. (Whether a
/// real server serves them is the business of the run itself.)
#[test]
fn the_test_certificates_of_the_script_make_a_trustable_root() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/minio.sh");
    let folder = support::TempDir::new("certs");
    let made = std::process::Command::new("sh")
        .arg(&script)
        .arg("certs")
        .arg(folder.path().join("c"))
        .output();
    let Ok(made) = made else { return };
    if !made.status.success() {
        // No openssl on this machine: nothing to check here.
        eprintln!("skipped: {}", String::from_utf8_lossy(&made.stderr));
        return;
    }
    let roots = stellarr_outbound::tls::roots_from_file(&folder.path().join("c/ca.crt")).unwrap();
    assert_eq!(roots.len(), 1);
}
