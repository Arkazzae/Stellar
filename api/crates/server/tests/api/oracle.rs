//! The same requests to this server and to the first one, side by side.
//!
//! Not part of the ordinary run: it needs the first server, which the
//! integrator's `oracle` script starts in a container, and a fresh one for
//! each part, because the rate limits count across the whole run.
//!
//! ```sh
//! oracle start skeleton 5191 Production        # prints the setup code
//! ORACLE_SETUP_CODE=XXXX-XXXX-XXXX ORACLE_PART=1 cargo test -p stellarr-api --test api oracle -- --ignored --nocapture
//! oracle stop skeleton; oracle start skeleton 5191 Production
//! ORACLE_SETUP_CODE=... ORACLE_PART=2 cargo test ... 
//! ```
//!
//! Both servers run in the Production environment. What differs by nature is
//! replaced before the two answers are compared: ids, moments, hues taken from
//! ids, trace identifiers and cookie values.

use std::net::SocketAddr;

use http::Method;
use serde_json::{Value, json};
use stellarr_api::values::Timestamp;

use crate::support::{Client, Request, Response, Server};

/// What a browser sees of an answer, as text, with what differs by nature
/// replaced.
fn describe(response: &Response) -> String {
    let mut lines = vec![response.status.to_string()];
    let mut headers: Vec<String> = Vec::new();
    for name in ["cache-control", "content-type", "expires", "location", "pragma", "allow", "retry-after"] {
        if let Some(value) = response.header(name) {
            headers.push(format!("{name}: {value}"));
        }
    }
    for value in response.headers.get_all("set-cookie") {
        headers.push(format!("set-cookie: {}", scrub_cookie(value.to_str().unwrap_or(""))));
    }
    lines.extend(headers);
    lines.push(format!("BODY: {}", scrub(&response.text())));
    lines.join("\n")
}

/// The cookie with its value and a 30-day expiry replaced.
fn scrub_cookie(line: &str) -> String {
    let mut parts = line.split("; ");
    let first = parts.next().unwrap_or("");
    let (name, value) = first.split_once('=').unwrap_or((first, ""));
    let mut text = if value.is_empty() { first.to_string() } else { format!("{name}=<value>") };
    for part in parts {
        text.push_str("; ");
        if let Some(date) = part.strip_prefix("expires=").filter(|date| !date.contains("1970")) {
            text.push_str(&format!("expires=in {} minutes", minutes_from_now(date)));
        } else {
            text.push_str(part);
        }
    }
    text
}

/// How far ahead an HTTP date is, in minutes, to the nearest ten, so that the
/// seconds between the two servers' answers do not show.
fn minutes_from_now(date: &str) -> i64 {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let parts: Vec<&str> = date.split_whitespace().collect();
    let month = MONTHS.iter().position(|m| *m == parts[2]).expect("a month") + 1;
    let moment = Timestamp::parse(&format!("{}-{month:02}-{:0>2} {}", parts[3], parts[1], parts[4])).expect("a date");
    let seconds = Timestamp::now().until(moment).as_secs() as i64;
    (seconds + 300) / 600 * 10
}

/// Replaces ids, moments, hues and trace identifiers.
fn scrub(text: &str) -> String {
    let bytes: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let is_hex = |c: char| c.is_ascii_hexdigit();
    let matches_at = |i: usize, shape: &str| -> bool {
        // `x` stands for a hex digit, anything else for itself.
        shape.chars().enumerate().all(|(k, s)| bytes.get(i + k).is_some_and(|c| if s == 'x' { is_hex(*c) } else { *c == s }))
    };
    while i < bytes.len() {
        if matches_at(i, "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx") {
            out.push_str("<id>");
            i += 36;
        } else if matches_at(i, "00-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-xxxxxxxxxxxxxxxx-00") {
            out.push_str("<trace>");
            i += 55;
        } else if bytes[i].is_ascii_digit() && matches_at_digits_time(&bytes, i) {
            let mut j = i + 19;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == '.') {
                j += 1;
            }
            if bytes.get(j) == Some(&'Z') {
                j += 1;
            }
            out.push_str("<time>");
            i = j;
        } else if bytes[i..].starts_with(&"oklch(0.58 0.14 ".chars().collect::<Vec<_>>()) {
            out.push_str("oklch(<hue>)");
            i += 16;
            while bytes[i] != ')' {
                i += 1;
            }
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn matches_at_digits_time(bytes: &[char], i: usize) -> bool {
    let shape = "dddd-dd-ddTdd:dd:dd";
    shape.chars().enumerate().all(|(k, s)| bytes.get(i + k).is_some_and(|c| if s == 'd' { c.is_ascii_digit() } else { *c == s }))
}

enum Step {
    /// A request on a named client: its name, a label, method, path, body.
    Send(&'static str, &'static str, Method, &'static str, Option<Value>),
    /// A client that has the cookies another had at this point.
    Copy(&'static str, &'static str),
    /// An invite to the instance's server, made by the signed-in owner on the
    /// named client: through the first server's own route there, written into
    /// the database here. Later bodies name it as `$INVITE`.
    Invite(&'static str),
}

fn send(jar: &'static str, label: &'static str, method: Method, path: &'static str, body: Option<Value>) -> Step {
    Step::Send(jar, label, method, path, body)
}

fn registration() -> Value {
    json!({ "username": "owner", "displayName": "First Owner", "email": "owner@example.test", "password": "Corona!2026-long", "setupCode": "$SETUP" })
}

fn part_one() -> Vec<Step> {
    let login = |user: &str| json!({ "username": user, "password": "Corona!2026-long" });
    vec![
        send("a", "registration open", Method::GET, "/api/auth/registration", None),
        send("a", "me anonymous", Method::GET, "/api/me", None),
        send("a", "logout anonymous", Method::POST, "/api/auth/logout", None),
        send("a", "session anonymous", Method::GET, "/api/session", None),
        send("a", "register: wrong setup code", Method::POST, "/api/auth/register", Some(json!({ "username": "owner", "displayName": "First Owner", "email": "owner@example.test", "password": "Corona!2026-long", "setupCode": "AAAA-AAAA-AAAA" }))),
        send("a", "register: invalid fields", Method::POST, "/api/auth/register", Some(json!({ "username": "x", "displayName": "", "email": "nope", "password": "1" }))),
        send("a", "register: nothing", Method::POST, "/api/auth/register", Some(json!({}))),
        send("a", "register: external", Method::POST, "/api/auth/register", Some(json!({ "username": "owner", "displayName": "O", "email": "owner@example.test", "external": true }))),
        send("a", "register: founding with a wizard", Method::POST, "/api/auth/register", Some({
            let mut body = registration();
            body["initialServer"] = json!({ "name": "Crew", "textChannelName": "Lobby", "voiceChannelName": "" });
            body
        })),
        send("b", "registration closed", Method::GET, "/api/auth/registration", None),
        send("a", "session after founding", Method::GET, "/api/session", None),
        send("a", "me after founding", Method::GET, "/api/me", None),
        send("b", "login wrong", Method::POST, "/api/auth/login", Some(json!({ "username": "owner", "password": "wrong" }))),
        send("b", "login unknown", Method::POST, "/api/auth/login", Some(json!({ "username": "nobody", "password": "wrong" }))),
        send("b", "login by email in capitals", Method::POST, "/api/auth/login", Some(login("OWNER@example.test"))),
        send("c", "login that is not kept", Method::POST, "/api/auth/login", Some(json!({ "username": "owner", "password": "Corona!2026-long", "keepSignedIn": false }))),
        send("d", "login that links", Method::POST, "/api/auth/login", Some(json!({ "username": "owner", "password": "Corona!2026-long", "linkPending": true }))),
        send("b", "session", Method::GET, "/api/session", None),
        Step::Copy("b", "b-before-logout"),
        send("b", "logout", Method::POST, "/api/auth/logout", None),
        Step::Copy("b-before-logout", "stale"),
        send("stale", "session with a revoked cookie", Method::GET, "/api/session", None),
        Step::Copy("b-before-logout", "stale-2"),
        send("stale-2", "me with a revoked cookie", Method::GET, "/api/me", None),
        Step::Copy("b-before-logout", "stale-3"),
        send("stale-3", "logout with a revoked cookie", Method::POST, "/api/auth/logout", None),
        send("c", "another session survives", Method::GET, "/api/session", None),
        send("e", "login without a body", Method::POST, "/api/auth/login", None),
        send("e", "login with broken json", Method::POST, "/api/auth/login", Some(Value::String("{bad".into()))),
        send("e", "login with an empty object", Method::POST, "/api/auth/login", Some(json!({}))),
    ]
}

fn part_two() -> Vec<Step> {
    vec![
        send("a", "register: founding with the defaults", Method::POST, "/api/auth/register", Some(registration())),
        send("b", "register: second without an invite", Method::POST, "/api/auth/register", Some(json!({ "username": "maya", "displayName": "Maya", "email": "maya@example.test", "password": "Corona!2026-long" }))),
        send("b", "register: bad invite", Method::POST, "/api/auth/register", Some(json!({ "username": "maya", "displayName": "Maya", "email": "maya@example.test", "password": "Corona!2026-long", "inviteCode": "nope" }))),
        send("b", "register: name and address taken", Method::POST, "/api/auth/register", Some(json!({ "username": "OWNER", "displayName": "Maya", "email": "Owner@Example.test", "password": "Corona!2026-long", "inviteCode": "nope" }))),
        send("b", "register: a password over the limit", Method::POST, "/api/auth/register", Some(json!({ "username": "maya", "displayName": "Maya", "email": "maya@example.test", "password": "p".repeat(257), "inviteCode": "nope" }))),
    ]
}

/// A registration with an invite, and an invalid starter server first.
fn part_three() -> Vec<Step> {
    let maya = |name: &str| json!({ "username": name, "displayName": "Maya Lin", "email": format!("{name}@example.test"), "password": "Corona!2026-long", "inviteCode": "$INVITE" });
    vec![
        send("a", "register: an invalid starter server", Method::POST, "/api/auth/register", Some({
            let mut body = registration();
            body["initialServer"] = json!({ "name": "Crew", "textChannelName": "two words" });
            body
        })),
        send("a", "register: founding with a voice name only", Method::POST, "/api/auth/register", Some({
            let mut body = registration();
            body["initialServer"] = json!({ "name": "Crew", "voiceChannelName": "hall" });
            body
        })),
        Step::Invite("a"),
        send("m", "register: with an invite", Method::POST, "/api/auth/register", Some(maya("maya"))),
        send("m", "session of the new member", Method::GET, "/api/session", None),
        send("m", "me of the new member", Method::GET, "/api/me", None),
        send("n", "register: the same invite again", Method::POST, "/api/auth/register", Some(maya("jo"))),
    ]
}

async fn run(steps: &[Step], oracle: (SocketAddr, &str), ours: (SocketAddr, &str), server: &Server) -> Vec<String> {
    let mut theirs: Vec<(String, Client)> = Vec::new();
    let mut mine: Vec<(String, Client)> = Vec::new();
    let mut differences = Vec::new();
    let mut invites = (String::new(), String::new());
    for step in steps {
        match step {
            Step::Invite(jar) => {
                let owner = &theirs.iter().find(|(name, _)| name == jar).expect("an owner").1;
                let made = owner.post_json("/api/admin/invites", &json!({})).await;
                assert_eq!(made.status, 201, "{}", made.text());
                let url = made.json::<Value>()["url"].as_str().unwrap().to_string();
                invites = (url.rsplit('/').next().unwrap().to_string(), server.invite());
            }
            Step::Copy(from, to) => {
                for (jars, (address, _)) in [(&mut theirs, oracle), (&mut mine, ours)] {
                    let source = jars.iter().find(|(name, _)| name == from).map(|(_, client)| client);
                    let copy = Client::browser(address).with_cookies_of(source.expect("a jar to copy"));
                    jars.retain(|(name, _)| name != to);
                    jars.push((to.to_string(), copy));
                }
            }
            Step::Send(jar, label, method, path, body) => {
                let mut answers = Vec::new();
                for (jars, (address, code)) in [(&mut theirs, oracle), (&mut mine, ours)] {
                    if !jars.iter().any(|(name, _)| name == jar) {
                        jars.push((jar.to_string(), Client::browser(address)));
                    }
                    let client = &jars.iter().find(|(name, _)| name == jar).unwrap().1;
                    let mut request = Request::new(method.clone(), path);
                    request = match body {
                        Some(Value::String(raw)) => request.body("application/json", raw),
                        Some(value) => {
                            let invite = if address == oracle.0 { &invites.0 } else { &invites.1 };
                            request.json(&serde_json::from_str(&value.to_string().replace("$SETUP", code).replace("$INVITE", invite)).unwrap())
                        }
                        None => request,
                    };
                    answers.push(describe(&client.send(request).await));
                }
                let verdict = if answers[0] == answers[1] { "same" } else { "DIFFERENT" };
                println!("{verdict:9} {label}");
                if answers[0] != answers[1] {
                    differences.push(format!("{label}\n--- first server\n{}\n--- this server\n{}", answers[0], answers[1]));
                }
            }
        }
    }
    differences
}

#[tokio::test]
#[ignore = "needs the first server: see the top of this file"]
async fn this_server_answers_as_the_first_one_does() {
    let oracle: SocketAddr = std::env::var("ORACLE_ADDRESS").unwrap_or_else(|_| "127.0.0.1:5191".into()).parse().expect("an address");
    let theirs = std::env::var("ORACLE_SETUP_CODE").expect("ORACLE_SETUP_CODE: the code the first server logged");
    let steps = match std::env::var("ORACLE_PART").as_deref() {
        Ok("2") => part_two(),
        Ok("3") => part_three(),
        _ => part_one(),
    };
    let server = Server::start_with(&[("ASPNETCORE_ENVIRONMENT", "Production"), ("Rtc__TurnSharedSecret", "a-secret")]).await;
    let ours = server.setup_code().await;

    let differences = run(&steps, (oracle, &theirs), (server.address(), &ours), &server).await;

    for difference in &differences {
        println!("\n{difference}");
    }
    assert!(differences.is_empty(), "{} answers differ", differences.len());
}
