//! Nothing that carries a key or a token reaches the log, at any level.

mod support;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::header::AUTHORIZATION;
use http::{HeaderName, HeaderValue};
use stellarr_outbound::{AddressPolicy, Client, Request, TableResolver};
use support::{Server, handler};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

/// Collects every field of every event and span, at every level.
#[derive(Clone, Default)]
struct Catch(Arc<Mutex<String>>);

impl Visit for Catch {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0.lock().unwrap(), "{}={value:?} ", field.name());
    }
}

impl Subscriber for Catch {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        attributes.record(&mut self.clone());
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, values: &Record<'_>) {
        values.record(&mut self.clone());
    }
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let _ = write!(self.0.lock().unwrap(), "{} ", event.metadata().target());
        event.record(&mut self.clone());
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

#[tokio::test]
async fn no_path_query_or_header_value_is_logged() {
    let caught = Catch::default();
    let _guard = tracing::subscriber::set_default(caught.clone());
    let end = Server::plain(handler(|mut conn| async move {
        conn.ok("text/plain", b"done").await;
    }))
    .await;
    let start = {
        let to = format!(
            "http://other.test:{}/landing/capability-two?api_key=hunter3",
            end.addr.port()
        );
        Server::plain(handler(move |mut conn| {
            let to = to.clone();
            async move { conn.redirect(302, &to).await }
        }))
        .await
    };
    let client = Client::new().with_resolver(Arc::new(TableResolver(vec![(
        "other.test".into(),
        vec!["127.0.0.1".parse().unwrap()],
    )])));
    let mut secret = HeaderValue::from_static("hunter4");
    secret.set_sensitive(true);

    let answer = client
        .send_collected(
            Request::get(&format!(
                "{}start/capability-one?api_key=hunter2",
                start.url("/")
            ))
            .unwrap()
            .policy(AddressPolicy::Anywhere)
            .max_redirects(2)
            .timeout(Duration::from_secs(10))
            .header(AUTHORIZATION, HeaderValue::from_static("Bearer hunter1"))
            .header(HeaderName::from_static("x-api-key"), secret),
        )
        .await
        .unwrap();
    // And the failure paths as well.
    let _ = client
        .send(
            Request::get("http://10.0.0.1/capability-three?api_key=hunter5")
                .unwrap()
                .policy(AddressPolicy::PublicOnly),
        )
        .await;
    let _ = client
        .send(
            Request::get("http://127.0.0.1:1/capability-six?api_key=hunter6")
                .unwrap()
                .policy(AddressPolicy::Anywhere),
        )
        .await;

    assert_eq!(answer.body.as_ref(), b"done");
    let log = caught.0.lock().unwrap().clone();
    assert!(
        !log.is_empty(),
        "the client logs nothing at all, so this proves little"
    );
    for secret in ["hunter", "capability", "api_key", "Bearer"] {
        assert!(!log.contains(secret), "{secret} in the log: {log}");
    }
}
