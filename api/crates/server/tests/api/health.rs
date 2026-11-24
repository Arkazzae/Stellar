use std::time::Duration;

use serde_json::{Value, json};

use crate::support::Server;

#[tokio::test]
async fn a_probe_leaves_no_log_lines_while_other_requests_keep_theirs() {
    let server = Server::start().await;
    let client = server.client();
    let other = format!("/api/no-such-route-{}", stellarr_api::values::Id::random());

    assert_eq!(client.get("/health/ready").await.status, 200);
    assert_eq!(client.get("/health/live").await.status, 200);
    assert_eq!(client.get(&other).await.status, 404);

    // A request is logged once its response is out; wait for the last
    // request's line, which comes after the probes'.
    let mut lines = Vec::new();
    for _ in 0..100 {
        lines = server.log();
        if lines.iter().any(|line| line.contains(&other)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("GET {other} 404 in "))),
        "{lines:#?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("/health/")),
        "{lines:#?}"
    );
}

#[tokio::test]
async fn reports_its_status() {
    let server = Server::start().await;
    let client = server.client();

    for (path, expected) in [("/health/live", "live"), ("/health/ready", "ready")] {
        let response = client.get(path).await;

        assert_eq!(response.status, 200);
        assert_eq!(
            response.header("content-type"),
            Some("application/json; charset=utf-8")
        );
        assert_eq!(response.json::<Value>(), json!({ "status": expected }));
        assert_eq!(response.text(), format!("{{\"status\":\"{expected}\"}}"));
    }
}

#[tokio::test]
async fn needs_no_identification_to_answer() {
    let server = Server::start().await;

    assert_eq!(server.anonymous().get("/health/live").await.status, 200);
    assert_eq!(server.anonymous().get("/health/ready").await.status, 200);
}

#[tokio::test]
async fn a_health_route_answers_get_and_nothing_else() {
    let server = Server::start().await;
    let client = server.client();

    let post = client
        .send(crate::support::Request::new(
            http::Method::POST,
            "/health/live",
        ))
        .await;
    let head = client
        .send(crate::support::Request::new(
            http::Method::HEAD,
            "/health/live",
        ))
        .await;

    assert_eq!(post.status, 405);
    assert_eq!(post.header("allow"), Some("GET"));
    assert!(post.body.is_empty());
    assert_eq!(head.status, 405);
}

#[tokio::test]
async fn the_log_names_where_the_server_listens_and_each_request_once() {
    let server = Server::start().await;
    let client = server.client();
    let marker = format!("/api/marker-{}", stellarr_api::values::Id::random());
    assert_eq!(client.get(&marker).await.status, 404);

    let mut lines = Vec::new();
    for _ in 0..100 {
        lines = server.log();
        if lines.iter().any(|line| line.contains(&marker)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Now listening on: http://127.0.0.1:"))
    );
    assert_eq!(
        lines.iter().filter(|line| line.contains(&marker)).count(),
        1
    );
    // The line says it is the request log's, so that its level can be set alone.
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Stellarr.Api.Requests") && line.contains(&marker))
    );
}

#[tokio::test]
async fn a_stopped_server_leaves_cleanly_on_sigterm() {
    let server = Server::start().await;
    assert_eq!(server.client().get("/health/ready").await.status, 200);

    assert!(server.stop().await);
}
