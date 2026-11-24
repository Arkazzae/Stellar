//! Answers that are small on the wire and enormous decoded. This file is its
//! own test binary so that the memory of the process can be measured with
//! nothing else in it:
//!
//!     /usr/bin/time -l target/debug/deps/bombs-<hash> <test name>
//!
//! The streams are made by `fixtures/encodings.py`.

mod support;

use std::time::{Duration, Instant};

use stellarr_outbound::{AddressPolicy, Client, Request};
use support::{Server, fixture, handler};

fn asset(name: &str) -> Vec<u8> {
    fixture(&format!("encodings/{name}"))
}

async fn bomb(file: &'static str, coding: &'static str) -> Server {
    let body = asset(file);
    Server::plain(handler(move |mut conn| {
        let body = body.clone();
        async move {
            conn.respond(
                "200 OK",
                &[("Content-Type", "text/html"), ("Content-Encoding", coding)],
                &body,
            )
            .await;
        }
    }))
    .await
}

fn local(url: &str, ceiling: u64) -> Request {
    Request::get(url)
        .unwrap()
        .policy(AddressPolicy::Anywhere)
        .timeout(Duration::from_secs(60))
        .max_response_bytes(ceiling)
}

#[tokio::test]
async fn four_gibibytes_from_a_few_kilobytes_collected_stop_at_the_ceiling() {
    let wire = asset("bomb-4g.gz.gz").len();
    assert!(wire < 8192, "{wire}");
    let server = bomb("bomb-4g.gz.gz", "gzip, gzip").await;
    let clock = Instant::now();
    let error = Client::new()
        .send_collected(local(&server.url("/"), 1 << 20))
        .await
        .unwrap_err();
    assert!(error.is_too_large(), "{error}");
    eprintln!(
        "refused after {:?}; {wire} bytes on the wire",
        clock.elapsed()
    );
    assert!(clock.elapsed() < Duration::from_secs(20));
}

#[tokio::test]
async fn four_gibibytes_streamed_stop_at_a_ceiling_of_many_mebibytes() {
    let server = bomb("bomb-4g.gz.gz", "gzip, gzip").await;
    let response = Client::new()
        .send(local(&server.url("/"), 256 << 20))
        .await
        .unwrap();
    let mut body = response.body;
    let mut total = 0u64;
    let error = loop {
        match body.chunk().await {
            Ok(Some(piece)) => total += piece.len() as u64,
            Ok(None) => panic!("the bomb ended"),
            Err(error) => break error,
        }
    };
    assert!(error.is_too_large(), "{error}");
    assert!(total <= 256 << 20, "{total}");
    // Close to the ceiling, so it really was read up to it.
    assert!(total > 200 << 20, "{total}");
}

#[tokio::test]
async fn eight_mebibytes_from_one_layer_collected_stop_at_the_ceiling() {
    let server = bomb("bomb-8m.gz", "gzip").await;
    let error = Client::new()
        .send_collected(local(&server.url("/"), 1 << 20))
        .await
        .unwrap_err();
    assert!(error.is_too_large(), "{error}");
}

#[tokio::test]
async fn a_bomb_read_only_as_far_as_the_caller_wants_costs_only_that() {
    let server = bomb("bomb-4g.gz.gz", "gzip, gzip").await;
    let response = Client::new()
        .send(local(&server.url("/"), 1 << 20))
        .await
        .unwrap();
    let mut body = response.body;
    let (bytes, more) = body.read_up_to(4096).await.unwrap();
    assert_eq!(bytes.len(), 4096);
    assert!(more);
}
