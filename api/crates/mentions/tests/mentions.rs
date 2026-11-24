//! Who a message names with `@username`. These are the cases of the server's
//! own reader of mentions, which this one must agree with, and the shared
//! cases the web renderer is held to as well (`tests/fixtures/message-syntax.json`).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use stellarr_mentions::extract_normalized_usernames;

fn found(content: &str) -> BTreeSet<String> {
    extract_normalized_usernames(content)
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn names_in_prose_are_mentions() {
    let cases: &[(&str, &[&str])] = &[
        ("Hej @Ada", &["ADA"]),
        ("@ada @ADA @Ada", &["ADA"]),
        ("@maja i @jo-1, @ilya_x", &["MAJA", "JO-1", "ILYA_X"]),
        ("(@ada)", &["ADA"]),
        ("**@ada**", &["ADA"]),
        ("~~@ada~~", &["ADA"]),
        ("||@ada||", &["ADA"]),
        ("<b>@ada</b>", &["ADA"]),
        ("<code>@ada</code>", &["ADA"]),
        ("@żaneta", &["ŻANETA"]),
        ("@Мая", &["МАЯ"]),
        ("@ada,", &["ADA"]),
        ("@ada.x", &["ADA.X"]),
        ("@adahttps://example.test/x", &["ADA"]),
    ];
    for (content, expected) in cases {
        assert_eq!(found(content), set(expected), "{content:?}");
    }
}

#[test]
fn short_names_and_mail_addresses_are_not_mentions() {
    for content in ["", "@", "@a", "maja@ada.pl"] {
        assert!(found(content).is_empty(), "{content:?}");
    }
}

#[test]
fn code_spans_hide_names() {
    for content in [
        "`@ada`",
        "``@ada``",
        "```@ada```",
        "`` `@ada` ``",
        "`a``@ada``",
        "``a`@ada``",
        "HTML `<code>@ada</code>`",
        "<code>`@ada`</code>",
    ] {
        assert!(found(content).is_empty(), "{content:?}");
    }
}

#[test]
fn an_unmatched_run_stays_literal_and_does_not_hide_what_follows() {
    let cases: &[(&str, &[&str])] = &[
        ("`@ada", &["ADA"]),
        ("``@ada", &["ADA"]),
        ("`@ada``", &["ADA"]),
        ("``@ada`", &["ADA"]),
        ("``a` @ada", &["ADA"]),
    ];
    for (content, expected) in cases {
        assert_eq!(found(content), set(expected), "{content:?}");
    }
}

#[test]
fn fenced_blocks_hide_names() {
    for content in [
        "```\n@ada\n```",
        "```text\n@ada\n```",
        "```@ada\ncode\n```",
        "```\n@ada\n``` \t",
        "````\n@ada\n```",
    ] {
        assert!(found(content).is_empty(), "{content:?}");
    }
}

#[test]
fn only_a_real_fenced_block_hides_a_name() {
    let cases: &[(&str, &[&str])] = &[
        ("```@ada", &["ADA"]),
        ("```\n@ada", &["ADA"]),
        ("```\n@ada\n```\n@bea", &["BEA"]),
        ("```\n@ada\n```\n@bea\n```\n@cee\n```", &["BEA"]),
        (
            "` start\n```\nnot here\n```\n@ada\n```\nnot here\n```\nend `",
            &["ADA"],
        ),
    ];
    for (content, expected) in cases {
        assert_eq!(found(content), set(expected), "{content:?}");
    }
}

#[test]
fn an_escaped_at_is_not_a_mention() {
    for content in ["\\@ada", "a\\@ada", "\\@ada \\@bea"] {
        assert!(found(content).is_empty(), "{content:?}");
    }
}

#[test]
fn an_escaped_character_does_not_join_the_word_around_a_mention() {
    let cases: &[(&str, &[&str])] = &[
        ("\\\\@ada", &["ADA"]),
        ("\\*@ada", &["ADA"]),
        ("\\_@ada", &["ADA"]),
        ("\\.@ada", &["ADA"]),
        ("\\-@ada", &["ADA"]),
        ("\\@@ada", &["ADA"]),
        ("\\@ada \\@bea @cee", &["CEE"]),
    ];
    for (content, expected) in cases {
        assert_eq!(found(content), set(expected), "{content:?}");
    }
}

#[test]
fn a_bare_address_hides_its_names() {
    for content in [
        "https://example.test/@ada",
        "http://example.test/@ada",
        "https://example.test/a/@ada?x=1",
        "https://example.test/@ada)",
        "https://example.test/[y]@ada",
        "https://example.test/@ada]@bea",
        "xhttps://example.test/@ada",
        "https://example.test/@ada@bea",
    ] {
        assert!(found(content).is_empty(), "{content:?}");
    }
}

#[test]
fn an_address_ends_like_the_transcript_says_it_does() {
    let cases: &[(&str, &[&str])] = &[
        ("HTTP://example.test/@ada", &["ADA"]),
        ("https://example.test/@ada\n@bea", &["BEA"]),
        ("https://example.test/@ada. @bea", &["BEA"]),
        ("https://example.test/@ada, @bea", &["BEA"]),
        ("https://example.test/@ada?x=@bea @cee", &["CEE"]),
    ];
    for (content, expected) in cases {
        assert_eq!(found(content), set(expected), "{content:?}");
    }
}

#[test]
fn valid_http_links_hide_labels_and_targets() {
    for content in [
        "[@ada](https://example.test/x)",
        "[see @ada](http://example.test/x)",
        "[@ada](https://example.test/a_(b))",
        "[@ada](HTTPS://example.test/x)",
        "[@ada](http://example.test/@bea)",
        "[https://example.test/@ada](http://example.test/y)",
        "[[@ada](https://inner.test)](https://outer.test)",
        "\\![@ada](https://example.test/x)",
    ] {
        assert!(found(content).is_empty(), "{content:?}");
    }
}

#[test]
fn names_outside_a_valid_link_are_mentions() {
    let cases: &[(&str, &[&str])] = &[
        ("[@ada](https://example.test/x) @bea", &["BEA"]),
        ("[x](https://outer.test) [@ada](ftp://x)", &["ADA"]),
        ("[@ada](https://x) @bea [@cee](ftp://y)", &["BEA", "CEE"]),
        ("[a](https://x)@ada", &["ADA"]),
    ];
    for (content, expected) in cases {
        assert_eq!(found(content), set(expected), "{content:?}");
    }
}

#[test]
fn malformed_non_http_links_and_images_stay_literal() {
    let cases: &[(&str, &[&str])] = &[
        ("[@ada](ftp://example.test/x)", &["ADA"]),
        ("[@ada](mailto:ada@example.test)", &["ADA"]),
        ("[@ada](https://example.test/x", &["ADA"]),
        ("[@ada] (https://example.test/x)", &["ADA"]),
        ("[@ada](www.example.test)", &["ADA"]),
        ("[@ada](#anchor)", &["ADA"]),
        ("[@ada](https://example.test/a b)", &["ADA"]),
        ("[@ada](http://)", &["ADA"]),
        ("![@ada](https://example.test/x)", &["ADA"]),
        ("![@ada](ftp://example.test/x)", &["ADA"]),
        ("[@ada](ftp://x/@bea)", &["ADA", "BEA"]),
        ("[@ada](ftp://x) @bea", &["ADA", "BEA"]),
    ];
    for (content, expected) in cases {
        assert_eq!(found(content), set(expected), "{content:?}");
    }
}

#[test]
fn paired_underscores_format_the_name_instead_of_naming_another_member() {
    let cases: &[(&str, &[&str])] = &[
        ("_@ada_", &["ADA"]),
        ("__@ada__", &["ADA"]),
        ("_@ada_ @ada_", &["ADA", "ADA_"]),
        ("__@ada__ @ada__", &["ADA", "ADA__"]),
        ("_@ada__", &["ADA"]),
        ("__@ada_", &["ADA"]),
        ("___@ada___", &["ADA"]),
        ("@ada_", &["ADA_"]),
        ("@ada__", &["ADA__"]),
        ("@a_da", &["A_DA"]),
        ("_https://x_). @ada_", &["ADA_"]),
        ("_https://x)_ @ada_", &["ADA_"]),
        ("__https://x__). @ada__", &["ADA__"]),
        ("\\_@ada_", &["ADA_"]),
        ("**_@ada_**", &["ADA"]),
        ("~~_@ada_~~", &["ADA"]),
        ("||_@ada_||", &["ADA"]),
        ("_**@ada**_", &["ADA"]),
        ("_~~@ada~~_", &["ADA"]),
        ("_||@ada||_", &["ADA"]),
        ("*@ada_*", &["ADA_"]),
        ("_@ada *x_ y*", &["ADA"]),
        ("_x ~~@ada_ y~~", &["ADA"]),
        ("_x ||@ada_ y||", &["ADA"]),
    ];
    for (content, expected) in cases {
        assert_eq!(found(content), set(expected), "{content:?}");
    }
}

#[test]
fn unpaired_or_crossed_underscores_remain_name_characters() {
    for content in [
        "_@ada",
        "_@ada\\_",
        "foo_@ada_",
        "_@ada_bar",
        "* _@ada* _",
        "*x _@ada* y_",
        "~~x _@ada~~ y_",
        "||x _@ada|| y_",
    ] {
        assert!(found(content).is_empty(), "{content:?}");
    }
}

#[test]
fn a_url_inside_a_label_does_not_swallow_the_mention_after_the_link() {
    let cases: &[(&str, &[&str])] = &[
        ("[https://x.test](http://y)@ada", &["ADA"]),
        ("[https://x.test/@bea](http://y)@ada", &["ADA"]),
        (
            "[http://x](http://y)@ada [x](http://z)@bea",
            &["ADA", "BEA"],
        ),
        ("_[http://x](http://y)@ada_", &["ADA"]),
        ("[a](http://x)@ada_", &["ADA_"]),
        ("[a](http://x)_@ada_", &["ADA"]),
    ];
    for (content, expected) in cases {
        assert_eq!(found(content), set(expected), "{content:?}");
    }
}

#[test]
fn a_url_in_literal_text_still_hides_all_of_its_names() {
    for content in [
        "[http://x](@ada)",
        "[https://x.test/@ada](@bea)",
        "https://x.test/@ada_",
        "_https://x.test/@ada_",
    ] {
        assert!(found(content).is_empty(), "{content:?}");
    }
}

#[test]
fn a_name_is_the_whole_word() {
    let x = |count: usize| "x".repeat(count);
    assert_eq!(found(&format!("@{}", x(32))), set(&[&"X".repeat(32)]));
    assert_eq!(
        found(&format!("@{}m", x(31))),
        set(&[&format!("{}M", "X".repeat(31))])
    );
    assert!(found(&format!("@{}", x(33))).is_empty());
    assert!(found(&format!("@{}", x(40))).is_empty());
    assert!(found(&format!("@{}more", x(32))).is_empty());
}

#[test]
fn a_name_is_counted_in_utf16_units_as_the_web_client_counts() {
    // Sixteen letters outside the first plane are thirty-two units.
    let wide = "\u{1D4D0}".repeat(16);
    assert_eq!(found(&format!("@{wide}")).len(), 1);
    assert!(found(&format!("@{wide}\u{1D4D0}")).is_empty());
}

#[test]
fn a_link_ends_at_the_next_whitespace() {
    assert_eq!(found("https://example.test/@jo\n@ada"), set(&["ADA"]));
}

#[test]
fn hostile_text_is_read_in_linear_time() {
    let repeat = |piece: &str, times: usize| piece.repeat(times);
    let contents = [
        repeat("`", 100_000),
        repeat("*", 100_000),
        repeat("_@ada_ @ada_ ", 10_000),
        repeat("*x _@ada* y_ ", 10_000),
        repeat("[http://x](http://y)@ada ", 8_000),
        repeat("[", 100_000),
        repeat("`a", 50_000),
        repeat("[x](http://a(", 20_000),
        repeat("http://a)*", 10_000),
        repeat("```\n@ada\n", 16_000),
        repeat("\\_", 50_000) + "@ada",
        repeat("||a ", 50_000),
        repeat("~~a_*", 50_000),
        repeat("[a](http://b)", 30_000),
        "(".repeat(200_000) + "@ada",
    ];

    let started = Instant::now();
    for content in &contents {
        found(content);
    }
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the hostile bodies took {:?}, which looks super-linear",
        started.elapsed()
    );
}

#[test]
fn every_shared_case_of_the_message_syntax_gives_the_same_names() {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/message-syntax.json")).unwrap();
    let messages = contract["messages"].as_array().unwrap();
    assert!(!messages.is_empty());
    for message in messages {
        let name = message["name"].as_str().unwrap();
        let content = message["content"].as_str().unwrap();
        let expected: BTreeSet<String> = message["mentions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|mention| mention.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(found(content), expected, "{name}: {content:?}");
    }
}
