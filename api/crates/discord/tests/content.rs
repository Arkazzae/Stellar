//! Discord message text rewritten for Stellarr. Every case is also read back
//! by the mention reader that decides whom to notify, which must find exactly
//! the accounts the resolver chose to mention and nobody else.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use stellarr_discord::{
    ImportResolver, MentionText, ReferenceResolver, escape, extract_normalized_usernames, rewrite,
};

struct Known;

impl ReferenceResolver for Known {
    fn user(&self, user_id: &str) -> Option<MentionText> {
        Some(match user_id {
            "1" => MentionText::account("ada"),
            "2" => MentionText::account("ada_"),
            "3" => MentionText::label("Bea (left)"),
            "4" => MentionText::account("bad name!"),
            "5" => MentionText::label("**Bob**"),
            "6" => MentionText::label(""),
            "7" => MentionText::label("`x"),
            "8" => MentionText::label("a\n```\nb"),
            "9" => MentionText::label("@ada"),
            _ => return None,
        })
    }

    fn role(&self, role_id: &str) -> Option<String> {
        (role_id == "10").then(|| "@Moderators".to_owned())
    }

    fn channel(&self, channel_id: &str) -> Option<String> {
        match channel_id {
            "20" => Some("#general".to_owned()),
            "21" => Some("#http://evil.test".to_owned()),
            _ => None,
        }
    }

    fn emoji(&self, emoji_id: &str) -> Option<String> {
        match emoji_id {
            "30" => Some("party".to_owned()),
            "31" => Some("Party".to_owned()),
            _ => None,
        }
    }
}

fn names(list: &str) -> BTreeSet<String> {
    list.split(',')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn rewrites_discord_tokens_into_stellarr_text() {
    // (Discord's text, what Stellarr gets, the names it mentions)
    let cases: &[(&str, &str, &str)] = &[
        // Users: the resolver decides between a mention and a label.
        ("hi <@1>", "hi @ada", "ADA"),
        ("hi <@!1>", "hi @ada", "ADA"),
        ("<@2> ok", "@ada_ ok", "ADA_"),
        ("**<@1>**", "**@ada**", "ADA"),
        ("<@3>", "Bea (left)", ""),
        ("<@4>", r"\@bad name!", ""),
        ("<@5>", r"\*\*Bob\*\*", ""),
        ("<@99>", r"\@unknown-user", ""),
        // A mention keeps its edge: Stellarr reads the whole word after the at sign.
        ("thanks <@1>.", r"thanks @ada\.", "ADA"),
        ("<@1>_", r"@ada\_", "ADA"),
        ("<@1>-ish", r"@ada\-ish", "ADA"),
        ("<@1>s", r"\@adas", ""),
        ("x<@1>", r"x\@ada", ""),
        ("<@1><@1>", r"@ada\@ada", "ADA"),
        ("<@1><t:0:d>", r"\@ada1970-01-01", ""),
        // An underscore that pairs with one outside would cut ada_ down to ada.
        ("_hi <@2>_", r"_hi \@ada__", ""),
        // Roles and channels have no mention in Stellarr; unknown ids stay readable.
        ("<@&10> please", r"\@Moderators please", ""),
        ("<@&99>", r"\@deleted-role", ""),
        ("see <#20>", "see #general", ""),
        ("<#21>", r"#http\://evil.test", ""),
        ("<#99>", "#deleted-channel", ""),
        // Custom emoji become the server's shortcode, or their own name as text.
        ("<:party:30>", ":party:", ""),
        ("<a:party:30>!", ":party:!", ""),
        ("<:party:30>s", ":party:s", ""),
        ("<:pepe_hands:99>", r":pepe\_hands:", ""),
        ("<:Party:31>", r"\:Party:", ""),
        ("x:ab<:party:30>", r"x\:ab:party:", ""),
        // Times, in UTC, as the style showed them.
        ("<t:1618953630>", "2021-04-20 21:20 UTC", ""),
        ("<t:1618953630:f>", "2021-04-20 21:20 UTC", ""),
        ("<t:1618953630:R>", "2021-04-20 21:20 UTC", ""),
        ("<t:1618953630:d>", "2021-04-20", ""),
        ("<t:1618953630:D>", "2021-04-20", ""),
        ("<t:1618953630:t>", "21:20 UTC", ""),
        ("<t:1618953630:T>", r"21\:20:30 UTC", ""),
        ("<t:1618953630:S>", r"2021-04-20 21\:20:30 UTC", ""),
        ("<t:-1>", "1969-12-31 23:59 UTC", ""),
        ("<t:99999999999999999>", "<t:99999999999999999>", ""),
        // Nothing inside code is touched.
        ("`<@1>`", "`<@1>`", ""),
        ("``a ` <@&10>``", "``a ` <@&10>``", ""),
        ("```<@1> @ada```", "```<@1> @ada```", ""),
        (
            "```js\nhi <@1> <:party:30> @ada\n```",
            "```js\nhi <@1> <:party:30> @ada\n```",
            "",
        ),
        ("a `<@1>` b <@1>", "a `<@1>` b @ada", "ADA"),
        ("`unclosed <@1>", "`unclosed @ada", "ADA"),
        // Text that only Stellarr would read as a mention or an emoji stays text.
        ("@everyone and @here", r"\@everyone and \@here", ""),
        ("hey @ada", r"hey \@ada", ""),
        ("(@ada)", r"(\@ada)", ""),
        ("_@ada_", r"_\@ada_", ""),
        ("mail me@ada.pl", "mail me@ada.pl", ""),
        ("at 18:30:00", r"at 18\:30:00", ""),
        (":party: time", r"\:party: time", ""),
        (
            "https://example.test/@ada:x: and @ada",
            r"https://example.test/@ada:x: and \@ada",
            "",
        ),
        // Discord's escapes are Stellarr's escapes.
        (r"\*not bold\* \@ada", r"\*not bold\* \@ada", ""),
        (r"\<@1>", r"\<\@1>", ""),
        ("<@123", r"<\@123", ""),
        ("", "", ""),
    ];

    for (discord, expected, mentions) in cases {
        let rewritten = rewrite(discord, &Known);
        assert_eq!(&rewritten, expected, "rewriting {discord:?}");
        assert_eq!(
            extract_normalized_usernames(&rewritten),
            names(mentions),
            "mentions of {rewritten:?}"
        );
    }
}

#[test]
fn no_mix_of_pieces_mentions_anyone_the_resolver_did_not_name() {
    let fragments = [
        "<@1>",
        "<@2>",
        "<@99>",
        "<@6>",
        "<@7>",
        "<@8>",
        "<@9>",
        "<@&10>",
        "<#20>",
        "<:party:30>",
        "<t:0>",
        "@ada",
        "@everyone",
        "_",
        "__",
        "*",
        ".",
        "-",
        "a",
        "1",
        " ",
        "`",
        "\\",
        "\n",
        "```\n",
        "]",
        ":",
        "https://x.test/",
        "\u{FEFF}",
        "\u{0085}",
        "\u{200B}",
    ];
    let allowed = names("ADA,ADA_");

    for first in fragments {
        for second in fragments {
            for third in fragments {
                let discord = format!("{first}{second}{third}");
                let found = extract_normalized_usernames(&rewrite(&discord, &Known));
                assert!(found.is_subset(&allowed), "{discord:?} mentions {found:?}");
            }
        }
    }
}

#[test]
fn an_address_ends_where_stellarr_ends_it() {
    let cases = [
        // Stellarr ends an address at a byte-order mark; what follows is text again.
        (
            "https://x.test/\u{FEFF}@ada",
            "https://x.test/\u{FEFF}\\@ada",
        ),
        ("https://x.test/\u{0085}@ada", "https://x.test/\u{0085}@ada"),
        // The web client ends an address at a closing bracket.
        ("https://x.test/]@ada", "https://x.test/]\\@ada"),
        ("https://mastodon.test/@ada", "https://mastodon.test/@ada"),
    ];
    for (discord, expected) in cases {
        let rewritten = rewrite(discord, &Known);
        assert_eq!(rewritten, expected, "rewriting {discord:?}");
        assert!(
            extract_normalized_usernames(&rewritten).is_empty(),
            "{rewritten:?}"
        );
    }
}

/// A resolver that is asked for everything and answers as little as it can.
struct Nobody;

impl ReferenceResolver for Nobody {
    fn user(&self, _: &str) -> Option<MentionText> {
        None
    }
    fn role(&self, _: &str) -> Option<String> {
        None
    }
    fn channel(&self, _: &str) -> Option<String> {
        None
    }
    fn emoji(&self, _: &str) -> Option<String> {
        None
    }
}

#[test]
fn what_the_import_makes_up_is_shown_as_it_is() {
    assert_eq!(escape("plain"), "plain");
    assert_eq!(escape("*bold* and _it_"), r"\*bold\* and \_it\_");
    assert_eq!(escape("@ada and @everyone"), r"\@ada and \@everyone");
    assert_eq!(
        escape("see https://evil.test/x"),
        r"see https\://evil.test/x"
    );
    assert_eq!(
        escape("[a](https://evil.test)"),
        r"\[a\](https\://evil.test)"
    );
    assert_eq!(escape("a:party:b"), r"a\:party:b");
    assert_eq!(escape(""), "");
    for text in ["@ada", "x @ada_", "``` @ada", "<@1>", "@@ada"] {
        assert!(
            extract_normalized_usernames(&escape(text)).is_empty(),
            "{text:?}"
        );
    }
}

/// Text that tries to hurt: it must come back quickly, mention nobody and
/// never panic, however it nests, closes or fails to close.
#[test]
fn hostile_text_is_read_in_bounded_time_and_mentions_nobody() {
    let big = 200_000;
    let hostile: Vec<(&str, String)> = vec![
        ("nested emphasis", "*_~|".repeat(big / 4) + "@ada"),
        ("unclosed emphasis", "*".repeat(big) + " @ada"),
        ("alternating emphasis", "_a*".repeat(big / 3) + " @ada"),
        ("spoilers", "||a ".repeat(big / 4) + "@ada"),
        ("brackets", "[".repeat(big) + "](http://a) @ada"),
        ("link chain", "[a](http://a)".repeat(big / 13) + " @ada"),
        (
            "parentheses",
            "[x](http://".to_owned() + &"(".repeat(big) + " @ada",
        ),
        (
            "backticks of every length",
            (1..600).map(|n| "`".repeat(n) + "a ").collect::<String>() + "@ada",
        ),
        ("unclosed backticks", "`".repeat(big) + "<@1>"),
        ("fences", "```\n".repeat(big / 4) + "@ada"),
        (
            "unclosed fence",
            "```js\n".to_owned() + &"@ada ".repeat(big / 5),
        ),
        ("tokens", "<@1>".repeat(big / 4)),
        (
            "broken tokens",
            "<@111111111111111111111111".repeat(big / 26) + " @ada",
        ),
        (
            "emoji names",
            ("<:".to_owned() + &"x".repeat(70) + ":1>").repeat(big / 80),
        ),
        ("times", "<t:99999999999999999999999:d>".repeat(big / 28)),
        ("long name", "@".to_owned() + &"a".repeat(big)),
        ("many ats", "@".repeat(big) + "ada"),
        ("addresses", "https://a ".repeat(big / 10) + "@ada"),
        (
            "one long address",
            "https://".to_owned() + &"a)".repeat(big / 2) + " @ada",
        ),
        ("colons", ":ab".repeat(big / 3) + ":"),
        ("escapes", "\\".repeat(big) + "@ada"),
        ("surrogates", "\u{1F600}@".repeat(big / 6) + "ada"),
        ("zero width", "\u{200B}@\u{200B}ada".repeat(big / 8)),
    ];

    for (what, text) in &hostile {
        let started = Instant::now();
        let rewritten = rewrite(text, &Known);
        let found = extract_normalized_usernames(&rewritten);
        let allowed = names("ADA,ADA_");
        assert!(found.is_subset(&allowed), "{what}: {found:?}");
        // The ones that hold a real token may mention ada; the rest may not.
        if !text.contains("<@1>") {
            assert!(found.is_empty(), "{what}: {found:?}");
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{what} took {:?}",
            started.elapsed()
        );

        let again = escape(text);
        assert!(extract_normalized_usernames(&again).is_empty(), "{what}");
        assert!(rewrite(text, &Nobody).len() >= text.len().min(1), "{what}");
    }
}

#[test]
fn mentions_of_things_that_do_not_exist_become_text() {
    let text =
        "<@18446744073709551616> <@&0> <#0> <:x:0> <t:abc> <@1x> <@> <#> <::1> <:a b:1> <@!!1>";
    let rewritten = rewrite(text, &Nobody);
    assert!(
        extract_normalized_usernames(&rewritten).is_empty(),
        "{rewritten:?}"
    );
    assert!(rewritten.contains("unknown-user"));
    assert!(rewritten.contains("deleted-role"));
    assert!(rewritten.contains("deleted-channel"));
    // What is not a token is left as written, with its at signs escaped.
    assert!(rewritten.contains("<t:abc>"));
}

#[test]
fn a_username_the_resolver_gives_that_is_not_one_is_a_label() {
    struct Odd(String);
    impl ReferenceResolver for Odd {
        fn user(&self, _: &str) -> Option<MentionText> {
            Some(MentionText::account(self.0.clone()))
        }
        fn role(&self, _: &str) -> Option<String> {
            None
        }
        fn channel(&self, _: &str) -> Option<String> {
            None
        }
        fn emoji(&self, _: &str) -> Option<String> {
            None
        }
    }

    for odd in [
        "a",
        "",
        "has space",
        &"x".repeat(33),
        "tab\t",
        "new\nline",
        "ab\n",
        "emoji\u{1F600}x",
    ] {
        let rewritten = rewrite("<@1>", &Odd(odd.to_string()));
        assert!(
            extract_normalized_usernames(&rewritten).is_empty(),
            "{odd:?} gave {rewritten:?}"
        );
    }
    // The longest name and non-Latin letters are names.
    let long = "x".repeat(32);
    assert_eq!(rewrite("<@1>", &Odd(long.clone())), format!("@{long}"));
    assert_eq!(rewrite("<@1>", &Odd("Zażółć".into())), "@Zażółć");
}

#[test]
fn the_import_resolver_asks_first_and_never_mentions_anybody() {
    let map = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    };
    let text = "hi <@1> and <@2>, see <#20> and <#21>, <@&10> <:party:30>";

    // The first pass knows the roles and notes what it is asked.
    let recording = ImportResolver::recording(map(&[("10", "@Moderators")]));
    let first = rewrite(text, &recording);
    assert_eq!(
        recording.asked_users().into_iter().collect::<Vec<_>>(),
        ["1", "2"]
    );
    assert_eq!(
        recording.asked_channels().into_iter().collect::<Vec<_>>(),
        ["20", "21"]
    );
    assert!(
        first.contains(r"\@unknown-user")
            && first.contains("#deleted-channel")
            && first.contains(r"\@Moderators")
    );

    // The second knows the people. A name that is another account's name is still a label.
    let known = ImportResolver::new(
        map(&[("1", "ada"), ("2", "Bea (left)")]),
        map(&[("20", "#general")]),
        map(&[("10", "@Moderators")]),
    );
    let second = rewrite(text, &known);
    assert_eq!(
        second,
        r"hi \@ada and \@Bea (left), see #general and #deleted-channel, \@Moderators \:party:"
    );
    assert!(extract_normalized_usernames(&second).is_empty());
    // A custom emoji keeps its name as text.
    assert_eq!(rewrite("<:party:30>", &known), r"\:party:");
}
