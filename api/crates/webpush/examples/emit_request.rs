//! Prints one complete push request as JSON, for `interop.mjs` to check.
//!
//! Usage: `emit_request <p256dh> <auth> [endpoint]`, both keys in unpadded
//! base64url. A fresh VAPID key pair is generated for every run.

use std::time::{SystemTime, UNIX_EPOCH};

use stellarr_webpush::{
    PushPayload, SendOptions, ValidatedSubscription, VapidKeys, base64url, build_request,
    generate_keys,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(p256dh), Some(auth)) = (args.first(), args.get(1)) else {
        eprintln!("usage: emit_request <p256dh> <auth> [endpoint]");
        std::process::exit(2);
    };
    let endpoint = args.get(2).map_or(
        "https://push.example.test/send/subscription",
        String::as_str,
    );

    let generated = generate_keys().expect("key pair");
    let vapid = VapidKeys::from_config(
        "mailto:push@example.test",
        &generated.public_key,
        &generated.private_key,
    )
    .expect("configured keys");
    let subscription = ValidatedSubscription::validate(Some(endpoint), Some(p256dh), Some(auth))
        .expect("a valid subscription");

    let payload = PushPayload {
        id: "0123456789abcdef".into(),
        title: "Zażółć gęślą jaźń \u{1F680}".into(),
        body: "Visible body".into(),
        url: "https://chat.example.test/".into(),
        tag: "0123456789abcdef".into(),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let request = build_request(
        &vapid,
        &subscription,
        &payload.to_json(),
        &SendOptions::default(),
        now,
    )
    .expect("request");

    let headers: Vec<_> = request
        .headers
        .iter()
        .map(|(name, value)| serde_json::json!([name, value]))
        .collect();
    let out = serde_json::json!({
        "method": request.method,
        "url": request.url,
        "headers": headers,
        "body": base64url::encode(&request.body),
        "vapidPublicKey": vapid.public_key(),
        "now": now,
        "payload": {
            "id": payload.id, "title": payload.title, "body": payload.body,
            "url": payload.url, "tag": payload.tag,
        },
    });
    println!("{out}");
}
