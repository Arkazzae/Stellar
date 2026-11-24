//! The bot token appears nowhere it should not: not in an error's text, not
//! in its debug form, not in what any value of the crate prints and not in
//! what is logged, on every way a read can fail.

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use stellarr_discord::{BotReader, BotToken, DiscordError, ReactionEmoji, RequestPolicy};
use support::{Answer, Catch, Rig, TOKEN};

/// Every piece of the token a careless message could carry.
fn pieces() -> Vec<String> {
    let mut all = vec![TOKEN.to_owned(), format!("Bot {TOKEN}")];
    all.extend(
        TOKEN
            .split('.')
            .filter(|part| part.len() >= 8)
            .map(str::to_owned),
    );
    all
}

fn assert_clean(what: &str, text: &str) {
    for piece in pieces() {
        assert!(!text.contains(&piece), "{what} holds {piece:?}: {text}");
    }
}

fn assert_error_is_clean(what: &str, error: &DiscordError) {
    assert_clean(what, &error.to_string());
    assert_clean(what, &format!("{error:?}"));
    assert_clean(what, &format!("{error:#?}"));
    assert_clean(what, error.route());
}

/// Discord's text can say anything, the token included.
fn echoing(status: u16) -> Answer {
    Answer::error(
        status,
        0,
        &format!("401: Unauthorized, Bot {TOKEN} is not valid, {TOKEN}"),
    )
}

#[tokio::test]
async fn no_way_to_fail_shows_the_token_in_an_error_or_in_the_log() {
    let log = Catch::default();
    let _guard = tracing::subscriber::set_default(log.clone());
    let long_before = "x".repeat(490);

    // Each case: a name, what Discord does, and how the read is configured.
    type Case = (
        &'static str,
        Box<dyn Fn(&support::Recorded, usize) -> Answer + Send + Sync>,
    );
    let cases: Vec<Case> = vec![
        ("401 echoing the token", Box::new(|_, _| echoing(401))),
        ("403 echoing the token", Box::new(|_, _| echoing(403))),
        ("404 echoing the token", Box::new(|_, _| echoing(404))),
        ("400 echoing the token", Box::new(|_, _| echoing(400))),
        (
            "a message the cut would run through",
            Box::new(move |_, _| Answer::error(403, 1, &format!("{long_before}{TOKEN} and on"))),
        ),
        (
            "a 429 echoing the token",
            Box::new(|_, _| Answer::too_many(2.0, false).with_header("X-Echo", TOKEN)),
        ),
        (
            "a 429 that asks for too long",
            Box::new(|_, _| Answer::too_many(5000.0, true)),
        ),
        (
            "a server error",
            Box::new(|_, _| Answer::status(500, TOKEN)),
        ),
        (
            "not json",
            Box::new(|_, _| Answer::status(200, TOKEN).with_header("Content-Type", "text/plain")),
        ),
        (
            "broken json holding the token",
            Box::new(|_, _| Answer::json(format!(r#"{{"id": "{TOKEN}"#))),
        ),
        (
            "the wrong shape holding the token",
            Box::new(|_, _| Answer::json(format!(r#"[{{"id":"{TOKEN}"}}]"#))),
        ),
        ("null", Box::new(|_, _| Answer::json("null"))),
        ("nesting", Box::new(|_, _| Answer::json("[".repeat(10_000)))),
        (
            "a redirect to the token",
            Box::new(|_, _| {
                Answer::status(302, "")
                    .with_header("Location", &format!("http://discord.test/{TOKEN}"))
            }),
        ),
        ("hanging up", Box::new(|_, _| Answer::Close)),
    ];

    for (what, handler) in cases {
        let rig = Rig::configured(handler, |config| {
            config.policy = RequestPolicy {
                max_rate_limit_retries: 2,
                server_error_pause: Duration::ZERO,
                ..RequestPolicy::default()
            }
        })
        .await;
        let bot = rig.bot();

        for (via, error) in [
            ("user", bot.current_user().await.err()),
            ("application", bot.current_application().await.err()),
            ("server", bot.read_server("1").await.err()),
            (
                "messages",
                bot.messages("1", None).unwrap().next_page().await.err(),
            ),
            ("members", bot.members("1").unwrap().next_page().await.err()),
            ("pins", bot.pins("1").unwrap().next_page().await.err()),
            (
                "threads",
                bot.archived_threads("1", true)
                    .unwrap()
                    .next_page()
                    .await
                    .err(),
            ),
        ] {
            if let Some(error) = error {
                assert_error_is_clean(&format!("{what} via {via}"), &error);
            }
        }
    }

    // The ways that need a different rig.
    let rig = Rig::configured(
        |_, _| Answer::Stall,
        |config| config.timeout = Duration::from_millis(150),
    )
    .await;
    assert_error_is_clean("a stall", &rig.bot().current_user().await.unwrap_err());
    let rig = Rig::configured(
        |_, _| Answer::StallHalfway(b"[".to_vec()),
        |config| config.timeout = Duration::from_millis(150),
    )
    .await;
    assert_error_is_clean(
        "a stall halfway",
        &rig.bot().current_user().await.unwrap_err(),
    );
    let rig = Rig::configured(
        |_, _| Answer::json(format!(r#"{{"id":"{}"}}"#, "x".repeat(10_000))),
        |config| config.max_response_bytes = 2048,
    )
    .await;
    assert_error_is_clean(
        "an answer over the ceiling",
        &rig.bot().current_user().await.unwrap_err(),
    );

    // Nothing listening, nothing resolving, nothing allowed.
    let clock = support::FakeClock::new();
    let rig = Rig::new(|_, _| Answer::json("{}")).await;
    let port = rig.server.addr.port();
    drop(rig);
    let make = |change: &dyn Fn(&mut stellarr_discord::ClientConfig)| {
        let mut config = support::config_for(port, clock.clone());
        change(&mut config);
        stellarr_discord::DiscordClient::with_config(config).for_bot(BotToken::new(TOKEN).unwrap())
    };
    assert_error_is_clean(
        "a refused connection",
        &make(&|_| {}).current_user().await.unwrap_err(),
    );
    assert_error_is_clean(
        "an unknown name",
        &make(&|c| c.base = format!("http://{TOKEN_LOWER}.test:{port}/"))
            .current_user()
            .await
            .unwrap_err(),
    );
    assert_error_is_clean(
        "a refused address",
        &make(&|c| c.address_policy = stellarr_outbound::AddressPolicy::PublicOnly)
            .current_user()
            .await
            .unwrap_err(),
    );
    assert_error_is_clean(
        "an address that is not one",
        &make(&|c| c.base = format!("not a url {TOKEN}"))
            .current_user()
            .await
            .unwrap_err(),
    );

    // The log has lines (waits and limits), and none of them has the token.
    let logged = log.text();
    assert!(logged.contains("rate"), "nothing was logged: {logged:?}");
    assert_clean("the log", &logged);
}

const TOKEN_LOWER: &str = "unknown-host";

#[tokio::test]
async fn an_id_that_is_not_one_is_not_repeated_and_the_token_as_an_id_least_of_all() {
    let rig = Rig::new(|_, _| Answer::json("{}")).await;
    let bot = rig.bot();
    let emoji = ReactionEmoji {
        id: None,
        name: "x".into(),
        animated: false,
    };

    for error in [
        bot.server(TOKEN).await.unwrap_err(),
        bot.read_server(TOKEN).await.unwrap_err(),
        bot.messages(TOKEN, None).unwrap_err(),
        bot.messages("1", Some(TOKEN)).unwrap_err(),
        bot.members(TOKEN).unwrap_err(),
        bot.reaction_users("1", TOKEN, &emoji, false).unwrap_err(),
    ] {
        assert_error_is_clean("an invalid id", &error);
    }
    assert_eq!(rig.server.hits(), 0);
}

#[tokio::test]
async fn the_token_in_a_route_is_redacted_there_too() {
    // A reaction's emoji is part of the path, and the caller chooses its name.
    let rig = Rig::new(|_, _| Answer::error(404, 10014, "Unknown Emoji")).await;
    let emoji = ReactionEmoji {
        id: None,
        name: TOKEN.into(),
        animated: false,
    };

    let error = rig
        .bot()
        .reaction_users("1", "2", &emoji, false)
        .unwrap()
        .next_page()
        .await
        .unwrap_err();

    assert_error_is_clean("the route", &error);
    assert!(error.route().contains("[token]"), "{}", error.route());
}

#[tokio::test]
async fn the_token_in_a_waiting_log_line_is_redacted_too() {
    let log = Catch::default();
    let _guard = tracing::subscriber::set_default(log.clone());
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::too_many(1.0, false)
        } else {
            Answer::json("[]")
        }
    })
    .await;
    let emoji = ReactionEmoji {
        id: None,
        name: TOKEN.into(),
        animated: false,
    };

    rig.bot()
        .reaction_users("1", "2", &emoji, false)
        .unwrap()
        .next_page()
        .await
        .unwrap();

    let logged = log.text();
    assert!(
        logged.contains("[token]") && logged.contains("limited"),
        "{logged}"
    );
    assert_clean("the log", &logged);
    assert_eq!(rig.clock.waits().len(), 1);
}

#[tokio::test]
async fn nothing_that_holds_the_token_prints_it() {
    let rig = Rig::new(|_, _| Answer::json(r#"{"id":"1"}"#)).await;
    let token = BotToken::new(TOKEN).unwrap();
    let bot: BotReader = rig.client.for_bot(token.clone());
    bot.current_user().await.unwrap();

    let pages = (
        bot.members("1").unwrap(),
        bot.messages("1", Some("2")).unwrap(),
        bot.pins("1").unwrap(),
    );
    for text in [
        format!("{token:?}"),
        format!("{token}"),
        format!("{token:#?}"),
        format!("{bot:?}"),
        format!("{bot:#?}"),
        format!("{pages:?}"),
        format!("{pages:#?}"),
        format!("{:?}", rig.client),
        format!("{:#?}", rig.client),
    ] {
        assert_clean("a printed value", &text);
    }
    assert_eq!(format!("{token:?}"), "BotToken");
    // The request really carried it, so the checks above are not about an empty string.
    assert!(
        rig.server.recorded()[0]
            .header("authorization")
            .unwrap()
            .contains(TOKEN)
    );
    // And the clock was never involved in any of it.
    assert!(!rig.clock.hold.load(Ordering::SeqCst));
}
