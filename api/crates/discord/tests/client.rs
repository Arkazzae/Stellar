//! The reader's requests: where they go, how they wait out rate limits on a
//! clock the test holds, and what each failure becomes.

mod support;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use stellarr_discord::{
    BotIdentity, BotToken, DiscordClient, FailureKind, RequestPolicy, parse_template_link,
};
use support::{Answer, Rig, TOKEN};

const ME: &str = r#"{"id":"198622483471925248","username":"importer","bot":true}"#;

fn seconds(list: &[f64]) -> Vec<Duration> {
    list.iter().map(|&s| Duration::from_secs_f64(s)).collect()
}

#[tokio::test]
async fn a_template_is_read_without_a_token_from_an_address_built_from_its_code() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/template-hgM48av5Q69A.json"
    ))
    .unwrap();
    let rig = Rig::new(move |_, _| Answer::json(fixture.clone())).await;
    let code = parse_template_link("https://discord.new/hgM48av5Q69A").unwrap();

    let snapshot = rig.client.read_template(&code).await.unwrap();

    let request = rig.server.recorded().remove(0);
    assert_eq!(rig.server.hits(), 1);
    assert_eq!(request.method, "GET");
    assert_eq!(request.target, "/api/v10/guilds/templates/hgM48av5Q69A");
    assert_eq!(request.header("authorization"), None);
    assert!(
        request
            .header("user-agent")
            .unwrap()
            .starts_with("DiscordBot (")
    );
    assert_eq!(request.header("accept"), Some("application/json"));
    assert_eq!(snapshot.name, "Friends & Family");
    assert_eq!(snapshot.channels.len(), 5);
}

#[tokio::test]
async fn an_unknown_template_says_so() {
    let rig = Rig::new(|_, _| Answer::error(404, 10057, "Unknown Guild Template")).await;
    let code = parse_template_link("abcdef").unwrap();

    let failure = rig.client.read_template(&code).await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::UnknownResource);
    assert_eq!(failure.discord_code(), Some(10057));
    assert_eq!(failure.status(), Some(404));
    assert_eq!(failure.route(), "GET guilds/templates/abcdef");
    assert_eq!(
        failure.to_string(),
        "Discord refused GET guilds/templates/abcdef: HTTP 404 (code 10057): Unknown Guild Template"
    );
}

#[tokio::test]
async fn a_bot_request_carries_the_token_in_its_header_only() {
    let rig = Rig::new(|_, _| Answer::json(ME)).await;

    let bot = rig.bot().current_user().await.unwrap();

    let request = rig.server.recorded().remove(0);
    assert_eq!(request.target, "/api/v10/users/@me");
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bot {TOKEN}").as_str())
    );
    assert!(!request.target.contains(TOKEN));
    assert_eq!(
        bot,
        BotIdentity {
            id: "198622483471925248".into(),
            username: "importer".into(),
            is_bot: true
        }
    );
}

#[test]
fn a_pasted_token_is_forgiven_its_prefix_and_spaces() {
    for pasted in [
        format!("Bot {TOKEN}"),
        format!("  {TOKEN}\n"),
        format!("bot {TOKEN}"),
        format!("BOT   {TOKEN}  "),
    ] {
        let token = BotToken::new(&pasted).unwrap();
        // It prints as its type and nothing else, in every form.
        assert_eq!(format!("{token:?}"), "BotToken");
        assert_eq!(format!("{token}"), "BotToken");
        assert_eq!(format!("{token:#?}"), "BotToken");
    }
}

#[test]
fn a_token_is_one_word() {
    for pasted in [
        "",
        "   ",
        "Bot ",
        "Bot",
        "bot",
        "two words",
        "tab\there",
        "new\nline",
        "nul\u{0}x",
        "\u{7f}",
    ] {
        let error = BotToken::new(pasted).unwrap_err();
        // The refusal does not repeat what was pasted.
        assert!(!format!("{error} {error:?}").contains("words"));
    }
    assert!(BotToken::new("Botany").is_ok());
}

#[tokio::test]
async fn the_apps_flags_say_which_privileged_intents_are_on() {
    // (flags, server members, message content)
    let cases: [(i64, bool, bool); 5] = [
        (1 << 15 | 1 << 19, true, true),
        (1 << 14 | 1 << 18, true, true),
        (1 << 15, true, false),
        (1 << 19, false, true),
        (1 << 12 | 1 << 13, false, false),
    ];
    for (flags, members, content) in cases {
        let rig = Rig::new(move |_, _| {
            Answer::json(format!(r#"{{"id":"1","name":"Importer","flags":{flags}}}"#))
        })
        .await;
        let application = rig.bot().current_application().await.unwrap();
        assert_eq!(rig.server.targets(), ["/api/v10/applications/@me"]);
        assert_eq!(
            (
                application.server_members_intent,
                application.message_content_intent
            ),
            (members, content),
            "{flags}"
        );
        assert_eq!(application.name, "Importer");
    }
}

/* ------------------------------------------------------------ rate limits */

#[tokio::test]
async fn an_exhausted_bucket_is_waited_out_before_the_next_request() {
    let rig = Rig::new(|_, index| match index {
        0 => Answer::json(ME)
            .with_header("X-RateLimit-Limit", "5")
            .with_header("X-RateLimit-Remaining", "0")
            .with_header("X-RateLimit-Reset", "1470173023")
            .with_header("X-RateLimit-Reset-After", "2.5")
            .with_header("X-RateLimit-Bucket", "abcd1234"),
        1 => Answer::json(ME)
            .with_header("X-RateLimit-Remaining", "4")
            .with_header("X-RateLimit-Bucket", "abcd1234"),
        _ => Answer::json(ME),
    })
    .await;
    let bot = rig.bot();

    bot.current_user().await.unwrap();
    assert!(rig.clock.waits().is_empty());
    bot.current_user().await.unwrap();
    bot.current_user().await.unwrap();

    // Reset-After, a duration, is used and not the epoch in Reset.
    assert_eq!(rig.clock.waits(), seconds(&[2.5]));
    assert_eq!(rig.server.hits(), 3);
}

#[tokio::test]
async fn the_reset_epoch_is_used_when_there_is_no_duration() {
    let rig = Rig::new(|_, index| match index {
        0 => Answer::json(ME)
            .with_header("X-RateLimit-Remaining", "0")
            .with_header("X-RateLimit-Reset", "1795435207"),
        _ => Answer::json(ME),
    })
    .await;
    let bot = rig.bot();

    bot.current_user().await.unwrap();
    bot.current_user().await.unwrap();

    // The clock stands at 1795435200, seven seconds before the reset.
    assert_eq!(rig.clock.waits(), seconds(&[7.0]));
}

#[tokio::test]
async fn a_429_is_waited_out_and_the_request_sent_again() {
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::too_many(1.5, false)
        } else {
            Answer::json(ME)
        }
    })
    .await;

    let bot = rig.bot().current_user().await.unwrap();

    assert_eq!(bot.username, "importer");
    assert_eq!(rig.clock.waits(), seconds(&[1.5]));
    assert_eq!(rig.server.hits(), 2);
}

#[tokio::test]
async fn a_429_without_json_falls_back_to_retry_after() {
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::status(429, "<html>Slow down</html>")
                .with_header("Content-Type", "text/html")
                .with_header("Retry-After", "4")
        } else {
            Answer::json(ME)
        }
    })
    .await;

    rig.bot().current_user().await.unwrap();

    assert_eq!(rig.clock.waits(), seconds(&[4.0]));
}

#[tokio::test]
async fn retry_after_may_be_a_date_and_a_missing_one_is_five_seconds() {
    // The clock stands at 2026-11-23T12:00:00Z.
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::status(429, "").with_header("Retry-After", "Mon, 23 Nov 2026 12:00:09 GMT")
        } else {
            Answer::json(ME)
        }
    })
    .await;
    rig.bot().current_user().await.unwrap();
    assert_eq!(rig.clock.waits(), seconds(&[9.0]));

    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::status(429, "no help")
        } else {
            Answer::json(ME)
        }
    })
    .await;
    rig.bot().current_user().await.unwrap();
    assert_eq!(rig.clock.waits(), seconds(&[5.0]));

    // A date in the past is no wait at all.
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::status(429, "").with_header("Retry-After", "Sun, 06 Nov 1994 08:49:37 GMT")
        } else {
            Answer::json(ME)
        }
    })
    .await;
    rig.bot().current_user().await.unwrap();
    assert!(rig.clock.waits().is_empty());
}

#[tokio::test]
async fn a_global_limit_holds_back_every_route() {
    let rig = Rig::configured(
        |request, _| {
            if request.path().ends_with("/users/@me") {
                Answer::too_many(3.0, true)
                    .with_header("X-RateLimit-Global", "true")
                    .with_header("X-RateLimit-Scope", "global")
            } else {
                Answer::json(r#"{"id":"1","name":"Importer","flags":0}"#)
            }
        },
        |config| {
            config.policy = RequestPolicy {
                max_rate_limit_retries: 0,
                ..RequestPolicy::default()
            }
        },
    )
    .await;
    let bot = rig.bot();

    let failure = bot.current_user().await.unwrap_err();
    assert_eq!(failure.kind(), FailureKind::RateLimited);
    assert!(rig.clock.waits().is_empty());

    // Another route, never limited itself, still waits for the global limit.
    bot.current_application().await.unwrap();
    assert_eq!(rig.clock.waits(), seconds(&[3.0]));
}

#[tokio::test]
async fn a_limit_on_one_route_does_not_hold_back_another() {
    let rig = Rig::configured(
        |request, _| {
            if request.path().ends_with("/users/@me") {
                Answer::too_many(30.0, false)
            } else {
                Answer::json(r#"{"id":"1","name":"Importer","flags":0}"#)
            }
        },
        |config| {
            config.policy = RequestPolicy {
                max_rate_limit_retries: 0,
                ..RequestPolicy::default()
            }
        },
    )
    .await;
    let bot = rig.bot();

    assert_eq!(
        bot.current_user().await.unwrap_err().kind(),
        FailureKind::RateLimited
    );
    bot.current_application().await.unwrap();
    assert!(rig.clock.waits().is_empty());
}

#[tokio::test]
async fn a_bucket_is_counted_for_each_channel_and_shared_by_routes_of_one_shape() {
    // Messages of two channels share a bucket by name but not their counts.
    let rig = Rig::new(|request, _| {
        if request.path().contains("/channels/10/") {
            Answer::json("[]")
                .with_header("X-RateLimit-Bucket", "msgs")
                .with_header("X-RateLimit-Remaining", "0")
                .with_header("X-RateLimit-Reset-After", "4")
        } else {
            Answer::json("[]")
                .with_header("X-RateLimit-Bucket", "msgs")
                .with_header("X-RateLimit-Remaining", "9")
        }
    })
    .await;
    let bot = rig.bot();

    bot.messages("10", None).unwrap().next_page().await.unwrap();
    bot.messages("20", None).unwrap().next_page().await.unwrap();
    assert!(rig.clock.waits().is_empty());
    bot.messages("10", None).unwrap().next_page().await.unwrap();
    assert_eq!(rig.clock.waits(), seconds(&[4.0]));
}

#[tokio::test]
async fn a_wait_longer_than_the_bound_fails_at_once_and_says_how_long() {
    // The documentation's example of a shared resource's limit.
    let rig = Rig::new(|_, _| {
        Answer::too_many(1336.57, false)
            .with_header("Retry-After", "1337")
            .with_header("X-RateLimit-Scope", "shared")
    })
    .await;

    let failure = rig.bot().current_user().await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::RateLimited);
    assert_eq!(
        failure.retry_after(),
        Some(Duration::from_secs_f64(1336.57))
    );
    assert_eq!(failure.status(), Some(429));
    assert_eq!(
        failure.to_string(),
        "Discord asked to wait 1336.57 s before GET users/@me, longer than the import waits"
    );
    assert!(rig.clock.waits().is_empty());
    assert_eq!(rig.server.hits(), 1);
}

#[tokio::test]
async fn repeated_limits_give_up_after_the_bound() {
    let rig = Rig::configured(
        |_, _| Answer::too_many(2.0, false),
        |config| {
            config.policy = RequestPolicy {
                max_rate_limit_retries: 3,
                ..RequestPolicy::default()
            }
        },
    )
    .await;

    let failure = rig.bot().current_user().await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::RateLimited);
    assert_eq!(rig.server.hits(), 4);
    assert_eq!(rig.clock.waits(), seconds(&[2.0, 2.0, 2.0]));
}

#[tokio::test]
async fn a_limit_that_was_given_up_on_is_still_remembered() {
    let rig = Rig::configured(
        |_, _| Answer::too_many(50.0, false),
        |config| {
            config.policy = RequestPolicy {
                max_rate_limit_retries: 0,
                ..RequestPolicy::default()
            }
        },
    )
    .await;
    let bot = rig.bot();

    assert_eq!(
        bot.current_user().await.unwrap_err().kind(),
        FailureKind::RateLimited
    );
    // Fifty seconds are left, which is inside the bound: the next ask waits them out first.
    assert_eq!(
        bot.current_user().await.unwrap_err().kind(),
        FailureKind::RateLimited
    );

    assert_eq!(rig.clock.waits(), seconds(&[50.0]));
    assert_eq!(rig.server.hits(), 2);
}

#[tokio::test]
async fn a_limit_longer_than_the_bound_fails_the_next_ask_without_a_request() {
    let rig = Rig::new(|_, _| Answer::too_many(100.0, false)).await;
    let bot = rig.bot();

    assert_eq!(
        bot.current_user().await.unwrap_err().retry_after(),
        Some(Duration::from_secs(100))
    );
    let again = bot.current_user().await.unwrap_err();

    assert_eq!(again.kind(), FailureKind::RateLimited);
    assert_eq!(again.retry_after(), Some(Duration::from_secs(100)));
    assert_eq!(rig.server.hits(), 1);
    assert!(rig.clock.waits().is_empty());
    // Once the time has passed, the route is open again.
    rig.clock.advance(Duration::from_secs(101));
    assert_eq!(
        bot.current_user().await.unwrap_err().kind(),
        FailureKind::RateLimited
    );
    assert_eq!(rig.server.hits(), 2);
}

#[tokio::test]
async fn cancelling_during_a_wait_stops_the_request() {
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::too_many(30.0, false)
        } else {
            Answer::json(ME)
        }
    })
    .await;
    rig.clock.hold.store(true, Ordering::SeqCst);
    let bot = rig.bot();

    // The read waits on the held clock for ever; dropping it is how it is cancelled.
    let outcome = tokio::time::timeout(Duration::from_millis(500), bot.current_user()).await;

    assert!(outcome.is_err());
    assert_eq!(rig.clock.waits(), seconds(&[30.0]));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.server.hits(), 1);
}

/* --------------------------------------------------------------- failures */

#[tokio::test]
async fn each_refusal_becomes_a_kind_the_import_can_act_on() {
    // (status, Discord's code or none, the kind)
    let cases: [(u16, Option<i32>, FailureKind); 9] = [
        (401, Some(0), FailureKind::InvalidToken),
        (401, Some(50014), FailureKind::InvalidToken),
        (403, Some(50001), FailureKind::MissingAccess),
        (403, Some(50013), FailureKind::MissingAccess),
        (404, Some(10003), FailureKind::UnknownResource),
        (404, Some(10004), FailureKind::UnknownResource),
        (400, Some(50035), FailureKind::UnexpectedResponse),
        (405, None, FailureKind::UnexpectedResponse),
        (302, None, FailureKind::UnexpectedResponse),
    ];
    for (status, code, kind) in cases {
        let rig = Rig::new(move |_, _| match code {
            Some(code) => Answer::error(status, code, "No"),
            None => Answer::status(status, ""),
        })
        .await;

        let failure = rig.bot().current_user().await.unwrap_err();

        assert_eq!(failure.kind(), kind, "{status} {code:?}");
        assert_eq!(failure.status(), Some(status));
        assert_eq!(failure.discord_code(), code);
        assert_eq!(rig.server.hits(), 1);
    }
}

#[tokio::test]
async fn server_errors_are_tried_twice_more_then_reported() {
    let rig = Rig::new(|_, _| Answer::status(502, "bad gateway")).await;

    let failure = rig.bot().current_user().await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::Unreachable);
    assert_eq!(failure.status(), Some(502));
    assert_eq!(
        failure.to_string(),
        "GET users/@me failed: Discord failed with HTTP 502"
    );
    assert_eq!(rig.server.hits(), 3);
    assert_eq!(rig.clock.waits(), seconds(&[1.0, 2.0]));
}

#[tokio::test]
async fn a_server_error_that_passes_is_not_a_failure() {
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::status(503, "")
        } else {
            Answer::json(ME)
        }
    })
    .await;

    assert_eq!(rig.bot().current_user().await.unwrap().username, "importer");
}

#[tokio::test]
async fn a_redirect_is_an_answer_and_is_not_followed() {
    let elsewhere = support::Server::start(|_, _| Answer::json(ME)).await;
    let target = format!("http://discord.test:{}/steal", elsewhere.addr.port());
    let rig = Rig::new(move |_, _| Answer::status(302, "").with_header("Location", &target)).await;

    let failure = rig.bot().current_user().await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::UnexpectedResponse);
    assert_eq!(failure.status(), Some(302));
    assert_eq!(rig.server.hits(), 1);
    // The other host, which a redirect would have sent the token to, was never asked.
    assert_eq!(elsewhere.hits(), 0);
}

#[tokio::test]
async fn nothing_listening_and_nothing_resolving_are_unreachable() {
    let rig = Rig::new(|_, _| Answer::json(ME)).await;
    let port = rig.server.addr.port();
    drop(rig);
    // The port is closed now: the connection is refused.
    let clock = support::FakeClock::new();
    let refused = DiscordClient::with_config(support::config_for(port, clock.clone()));
    let failure = refused
        .for_bot(BotToken::new(TOKEN).unwrap())
        .current_user()
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), FailureKind::Unreachable);
    assert!(
        failure.to_string().contains("could not reach Discord"),
        "{failure}"
    );

    // A name the table does not know.
    let mut config = support::config_for(port, clock);
    config.base = format!("http://nowhere.test:{port}/api/v10/");
    let failure = DiscordClient::with_config(config)
        .for_bot(BotToken::new(TOKEN).unwrap())
        .current_user()
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), FailureKind::Unreachable);
}

#[tokio::test]
async fn the_production_address_is_not_reachable_from_a_private_one() {
    // The default configuration refuses private addresses by name of its policy, so a
    // resolver that points Discord's host at the loopback gets nowhere.
    let mut config = support::config_for(1, support::FakeClock::new());
    config.base = "http://discord.test:1/api/v10/".to_owned();
    config.address_policy = stellarr_outbound::AddressPolicy::PublicOnly;
    let failure = DiscordClient::with_config(config)
        .for_bot(BotToken::new(TOKEN).unwrap())
        .current_user()
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), FailureKind::Unreachable);
    assert!(
        failure.to_string().contains("address not allowed"),
        "{failure}"
    );
}

#[tokio::test]
async fn an_answer_that_hangs_up_is_unreachable() {
    let rig = Rig::new(|_, _| Answer::Close).await;
    assert_eq!(
        rig.bot().current_user().await.unwrap_err().kind(),
        FailureKind::Unreachable
    );
}

#[tokio::test]
async fn an_answer_that_never_comes_times_out() {
    let rig = Rig::configured(
        |_, _| Answer::Stall,
        |config| config.timeout = Duration::from_millis(300),
    )
    .await;
    let started = Instant::now();

    let failure = rig.bot().current_user().await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::Unreachable);
    assert_eq!(
        failure.to_string(),
        "GET users/@me failed: Discord did not answer in time"
    );
    assert!(started.elapsed() < Duration::from_secs(8));
}

#[tokio::test]
async fn an_answer_that_stalls_halfway_times_out() {
    let rig = Rig::configured(
        |_, _| Answer::StallHalfway(br#"{"id":"#.to_vec()),
        |config| config.timeout = Duration::from_millis(300),
    )
    .await;
    let started = Instant::now();

    let failure = rig.bot().current_user().await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::Unreachable);
    assert!(started.elapsed() < Duration::from_secs(8));
}

#[tokio::test]
async fn an_answer_over_the_ceiling_is_refused() {
    let big = format!(r#"{{"id":"1","username":"{}"}}"#, "x".repeat(20_000));
    let rig = Rig::configured(
        move |_, _| Answer::json(big.clone()),
        |config| config.max_response_bytes = 4096,
    )
    .await;

    let failure = rig.bot().current_user().await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::Unreachable);
    assert!(
        failure.to_string().contains("larger than the import reads"),
        "{failure}"
    );
    // Without the ceiling it reads.
    let rig = Rig::new(move |_, _| {
        Answer::json(format!(
            r#"{{"id":"1","username":"{}"}}"#,
            "x".repeat(20_000)
        ))
    })
    .await;
    assert_eq!(
        rig.bot().current_user().await.unwrap().username.len(),
        20_000
    );
}

#[tokio::test]
async fn an_error_body_over_the_cap_is_cut_and_still_a_refusal() {
    let long = "y".repeat(200_000);
    let rig = Rig::new(move |_, _| {
        Answer::status(403, format!(r#"{{"message":"{long}","code":50013}}"#))
    })
    .await;

    let failure = rig.bot().current_user().await.unwrap_err();

    // The body is cut inside its text, so it does not read: the status still says what happened.
    assert_eq!(failure.kind(), FailureKind::MissingAccess);
    assert_eq!(failure.discord_code(), None);
    // A long message that fits is quoted, but not without bound.
    let long = "z".repeat(5_000);
    let rig = Rig::new(move |_, _| Answer::error(403, 50013, &long)).await;
    let failure = rig.bot().current_user().await.unwrap_err();
    assert_eq!(failure.discord_code(), Some(50013));
    assert!(
        failure.to_string().len() < 700,
        "{}",
        failure.to_string().len()
    );
}

#[tokio::test]
async fn an_answer_of_another_shape_is_unexpected() {
    // (body, content type, fails)
    let cases: [(&str, &str, bool); 7] = [
        (
            r#"{"id": [1, 2], "username": {"a": 1}}"#,
            "application/json",
            false,
        ),
        ("not json", "application/json", true),
        ("<html></html>", "text/html", true),
        ("null", "application/json", true),
        ("", "application/json", true),
        ("[1,2]", "application/json", true),
        (
            r#"{"id":"1"}"#,
            "application/problem+json; charset=utf-8",
            false,
        ),
    ];
    for (body, content_type, fails) in cases {
        let rig = Rig::new(move |_, _| {
            Answer::status(200, body).with_header("Content-Type", content_type)
        })
        .await;

        let read = rig.bot().current_user().await;

        if fails {
            assert_eq!(
                read.unwrap_err().kind(),
                FailureKind::UnexpectedResponse,
                "{body:?} as {content_type}"
            );
        } else {
            // Odd values in fields read as absent rather than failing the answer.
            let identity = read.unwrap();
            assert_eq!(identity.username, "");
            assert!(!identity.is_bot);
        }
    }
}

#[tokio::test]
async fn deeply_nested_json_fails_the_answer_cleanly() {
    let nested = format!(
        r#"{{"id":"1","extra":{}{}}}"#,
        "[".repeat(100_000),
        "]".repeat(100_000)
    );
    let rig = Rig::new(move |_, _| Answer::json(nested.clone())).await;

    let failure = rig.bot().current_user().await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::UnexpectedResponse);
    assert!(failure.to_string().contains("another shape"));
}

#[tokio::test]
async fn an_answer_without_a_content_type_is_read_as_json() {
    let rig = Rig::new(|_, _| Answer::status(200, ME)).await;
    assert_eq!(rig.bot().current_user().await.unwrap().username, "importer");
}

#[tokio::test]
async fn numbers_where_text_is_expected_and_the_other_way_read_both_ways() {
    let rig = Rig::new(|_, _| {
        Answer::json(
            r#"{"id": 198622483471925248, "username": "importer", "bot": true, "flags": "5"}"#,
        )
    })
    .await;
    assert_eq!(
        rig.bot().current_user().await.unwrap().id,
        "198622483471925248"
    );
}

#[tokio::test]
async fn the_configuration_and_the_reader_print_without_secrets() {
    let rig = Rig::new(|_, _| Answer::json(ME)).await;
    let bot = rig.bot();
    for text in [
        format!("{:?}", rig.client),
        format!("{:?}", bot),
        format!("{:#?}", rig.client),
    ] {
        assert!(!text.contains(TOKEN), "{text}");
        assert!(!text.contains("MTE5ODYy"), "{text}");
    }
    assert_eq!(format!("{bot:?}"), "BotReader");
}
