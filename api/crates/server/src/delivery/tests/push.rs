//! The sender against a push service of the test's own, over TLS on the
//! loopback.

use std::sync::Arc;
use std::time::{Duration, Instant};

use stellarr_outbound::{AddressPolicy, TableResolver};
use stellarr_webpush::{Dispatch, PushPayload};

use super::support::{Behaviour, FakePush, PUSH_HOST, World, certificates, open, wired_to};
use crate::delivery::sender::{Sender, Stored, Wiring};

fn payload() -> PushPayload {
    PushPayload {
        id: "0123456789abcdef0123456789abcdef".into(),
        title: "New mention".into(),
        body: "Someone mentioned you in a channel. Open Stellarr to read it.".into(),
        url: "https://chat.test/".into(),
        tag: "0123456789abcdef0123456789abcdef".into(),
    }
}

async fn send_to(behaviour: Behaviour) -> (Dispatch, FakePush, super::support::Browser) {
    let world = World::new(&[]).await;
    let fake = FakePush::start(behaviour).await;
    let browser = world.browser(world.ada, "send/one").await;
    let sender = wired_to(&world, &fake);
    let result = sender.send(&stored(&browser), &payload()).await;
    (result, fake, browser)
}

fn stored(browser: &super::support::Browser) -> Stored {
    Stored {
        id: browser.id,
        endpoint: browser.endpoint.clone(),
        p256dh: stellarr_webpush::base64url::encode(&browser.p256dh),
        auth: stellarr_webpush::base64url::encode(&browser.auth),
    }
}

#[tokio::test]
async fn the_push_service_gets_one_encrypted_signed_request_the_browser_can_read() {
    let (result, fake, browser) = send_to(Behaviour::Status(201)).await;

    assert_eq!(result, Dispatch::Delivered);
    let seen = fake.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/send/one");
    assert_eq!(seen[0].header("ttl"), Some("300"));
    assert_eq!(seen[0].header("content-encoding"), Some("aes128gcm"));
    assert_eq!(seen[0].header("content-type"), Some("application/octet-stream"));
    assert_eq!(seen[0].header("accept-encoding"), Some("identity"));
    assert_eq!(seen[0].header("user-agent"), Some("Stellarr/2.0"));
    assert!(seen[0].header("authorization").unwrap().starts_with("vapid t="));
    let (record_size, plain) = open(&browser, &seen[0].body);
    assert_eq!(record_size, 4096);
    assert_eq!(
        String::from_utf8(plain).unwrap(),
        "{\"id\":\"0123456789abcdef0123456789abcdef\",\"title\":\"New mention\",\"body\":\"Someone mentioned you in a channel. Open Stellarr to read it.\",\"url\":\"https://chat.test/\",\"tag\":\"0123456789abcdef0123456789abcdef\"}"
    );
}

#[tokio::test]
async fn every_answer_means_what_the_first_server_took_it_to_mean() {
    for (status, expected) in [
        (200, Dispatch::Delivered),
        (201, Dispatch::Delivered),
        (204, Dispatch::Delivered),
        (400, Dispatch::Retry),
        (401, Dispatch::Retry),
        (403, Dispatch::Retry),
        (404, Dispatch::Gone),
        (408, Dispatch::Retry),
        (410, Dispatch::Gone),
        (413, Dispatch::Retry),
        (429, Dispatch::Retry),
        (500, Dispatch::Retry),
        (503, Dispatch::Retry),
        (302, Dispatch::Retry),
    ] {
        let (result, _, _) = send_to(Behaviour::Status(status)).await;
        assert_eq!(result, expected, "{status}");
    }
}

#[tokio::test]
async fn a_retry_after_is_not_obeyed_as_the_first_server_did_not() {
    for header in ["30", "Wed, 21 Oct 2099 07:28:00 GMT"] {
        let (result, _, _) = send_to(Behaviour::StatusWith(429, "Retry-After", header)).await;
        assert_eq!(result, Dispatch::Retry);
    }
}

#[tokio::test]
async fn a_large_body_with_the_status_is_still_the_status() {
    let (result, _, _) = send_to(Behaviour::BigBody(8 * 1024 * 1024)).await;

    assert_eq!(result, Dispatch::Delivered);
}

#[tokio::test]
async fn an_answer_that_breaks_off_is_a_retry_in_its_head_and_the_status_in_its_body() {
    assert_eq!(send_to(Behaviour::CloseMidHead).await.0, Dispatch::Retry);
    assert_eq!(send_to(Behaviour::CloseMidBody).await.0, Dispatch::Delivered);
}

#[tokio::test]
async fn a_service_that_never_answers_is_given_ten_seconds() {
    let started = Instant::now();

    let (result, _, _) = send_to(Behaviour::Silent).await;

    assert_eq!(result, Dispatch::Retry);
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(9) && waited < Duration::from_secs(14), "{waited:?}");
}

#[tokio::test]
async fn a_redirect_is_not_followed_wherever_it_points() {
    let target = FakePush::start(Behaviour::Status(201)).await;
    for to in [
        format!("https://{PUSH_HOST}:{}/elsewhere", target.port),
        "https://127.0.0.1/inside".to_string(),
        "http://10.0.0.1/inside".to_string(),
    ] {
        let (result, fake, _) = send_to(Behaviour::Redirect(to)).await;

        assert_eq!(result, Dispatch::Retry);
        assert_eq!(fake.seen().len(), 1);
    }
    assert!(target.seen().is_empty());
}

#[tokio::test]
async fn the_sender_the_server_runs_never_reaches_the_loopback() {
    let world = World::new(&[]).await;
    let fake = FakePush::start(Behaviour::Status(201)).await;
    let browser = world.browser(world.ada, "send/one").await;
    // Everything of the test's wiring but the policy, which is the server's.
    let sender = Sender::wired(
        &world.state.settings().push,
        Wiring {
            roots: Some(certificates::roots()),
            resolver: Some(Arc::new(TableResolver(vec![(
                PUSH_HOST.to_string(),
                vec!["127.0.0.1".parse().unwrap()],
            )]))),
            policy: AddressPolicy::PublicOnly,
            port: Some(fake.port),
        },
    );

    let result = sender.send(&stored(&browser), &payload()).await;

    assert_eq!(result, Dispatch::Retry);
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn a_stored_endpoint_that_is_not_public_is_gone_without_a_request() {
    let world = World::new(&[]).await;
    let fake = FakePush::start(Behaviour::Status(201)).await;
    let browser = world.browser(world.ada, "x").await;
    let sender = wired_to(&world, &fake);
    for endpoint in [
        "https://127.0.0.1/send",
        "https://[::1]/send",
        "https://localhost/send",
        "http://push.example.test/send",
        "https://push.example.test:8443/send",
        "https://10.0.0.1/send",
    ] {
        let mut subscription = stored(&browser);
        subscription.endpoint = endpoint.to_string();

        assert_eq!(sender.send(&subscription, &payload()).await, Dispatch::Gone, "{endpoint}");
    }
    let mut broken = stored(&browser);
    broken.p256dh = "not-a-public-key".into();
    assert_eq!(sender.send(&broken, &payload()).await, Dispatch::Gone);
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn a_payload_that_is_too_large_is_tried_again_and_nothing_is_sent() {
    let world = World::new(&[]).await;
    let fake = FakePush::start(Behaviour::Status(201)).await;
    let browser = world.browser(world.ada, "x").await;
    let mut big = payload();
    big.body = "x".repeat(6000);

    assert_eq!(wired_to(&world, &fake).send(&stored(&browser), &big).await, Dispatch::Retry);
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn without_a_usable_key_pair_push_is_unavailable_and_every_send_is_a_retry() {
    for extra in [
        vec![("STELLARR_PUSH_ENABLED", "false")],
        vec![("STELLARR_PUSH_PRIVATE_KEY", "not-a-key")],
        vec![("STELLARR_PUSH_SUBJECT", "ftp://x")],
    ] {
        let world = World::new(&extra).await;
        let fake = FakePush::start(Behaviour::Status(201)).await;
        let browser = world.browser(world.ada, "x").await;
        let sender = wired_to(&world, &fake);

        assert!(!sender.is_available());
        assert_eq!(sender.send(&stored(&browser), &payload()).await, Dispatch::Retry);
        assert!(fake.seen().is_empty());
    }
}
