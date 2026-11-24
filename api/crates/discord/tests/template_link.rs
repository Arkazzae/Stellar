use stellarr_discord::{TemplateLinkRefusal, parse_template_link};

#[test]
fn reads_the_code_out_of_every_shape_discord_hands_out() {
    for input in [
        "hgM48av5Q69A",
        "  hgM48av5Q69A\n",
        "https://discord.new/hgM48av5Q69A",
        "http://discord.new/hgM48av5Q69A",
        "discord.new/hgM48av5Q69A",
        "www.discord.new/hgM48av5Q69A",
        "https://discord.new/hgM48av5Q69A/",
        "HTTPS://Discord.New/hgM48av5Q69A",
        "https://discord.com/template/hgM48av5Q69A",
        "https://www.discord.com/template/hgM48av5Q69A",
        "discord.com/template/hgM48av5Q69A",
        "https://discordapp.com/template/hgM48av5Q69A",
        "www.discordapp.com/template/hgM48av5Q69A",
        "https://discord.com/TEMPLATE/hgM48av5Q69A",
    ] {
        let code = parse_template_link(input).unwrap_or_else(|why| panic!("{input:?}: {why:?}"));
        assert_eq!(code.as_str(), "hgM48av5Q69A", "{input:?}");
    }
}

#[test]
fn refuses_anything_else_and_says_why() {
    use TemplateLinkRefusal::{Empty, InvalidCode, InviteLink, NotATemplateLink};
    let cases = [
        ("", Empty),
        ("   ", Empty),
        ("https://discord.gg/abcdef", InviteLink),
        ("discord.gg/abcdef", InviteLink),
        ("https://discord.com/invite/abcdef", InviteLink),
        (
            "https://discord.new.evil.test/hgM48av5Q69A",
            NotATemplateLink,
        ),
        ("https://evildiscord.new/hgM48av5Q69A", NotATemplateLink),
        (
            "https://discord.com.evil.test/template/hgM48av5Q69A",
            NotATemplateLink,
        ),
        (
            "https://discord.new@evil.test/hgM48av5Q69A",
            NotATemplateLink,
        ),
        (
            "https://evil.test@discord.new/hgM48av5Q69A",
            NotATemplateLink,
        ),
        ("https://discord.new:8443/hgM48av5Q69A", NotATemplateLink),
        ("https://discord.new/hgM48av5Q69A?ref=x", NotATemplateLink),
        ("https://discord.new/hgM48av5Q69A#top", NotATemplateLink),
        ("https://discord.new/hgM48av5Q69A/extra", NotATemplateLink),
        (
            "https://discord.com/template/hgM48av5Q69A/extra",
            NotATemplateLink,
        ),
        ("https://discord.com/hgM48av5Q69A", NotATemplateLink),
        ("https://discord.com/channels/1/2", NotATemplateLink),
        ("https://discord.new/", NotATemplateLink),
        ("https://discord.new", NotATemplateLink),
        ("discord.new", NotATemplateLink),
        ("ftp://discord.new/hgM48av5Q69A", NotATemplateLink),
        ("javascript://discord.new/hgM48av5Q69A", NotATemplateLink),
        ("https://discord.new\\@evil.test/x", NotATemplateLink),
        ("hgM48 av5Q69A", NotATemplateLink),
        ("https://discord.new/..", InvalidCode),
        ("https://discord.new/%2e%2e", InvalidCode),
        ("https://discord.new/abc%2Fdef", InvalidCode),
        ("x", InvalidCode),
        ("abc!def", InvalidCode),
        // A code that ends in a line break is not a code.
        ("https://discord.new/abc\n/", InvalidCode),
        ("ab\ncd", InvalidCode),
        ("https://discord.new/\u{ff48}gM48av5Q69A", InvalidCode),
        ("https://discord.new//", NotATemplateLink),
    ];
    for (input, expected) in cases {
        assert_eq!(
            parse_template_link(input).unwrap_err(),
            expected,
            "{input:?}"
        );
    }
}

#[test]
fn a_code_is_two_to_sixty_four_safe_characters() {
    assert!(parse_template_link("ab").is_ok());
    assert!(parse_template_link(&"a".repeat(64)).is_ok());
    assert_eq!(
        parse_template_link(&"a".repeat(65)).unwrap_err(),
        TemplateLinkRefusal::InvalidCode
    );
    assert!(parse_template_link("a_b-c").is_ok());
    // Not a panic, whatever the text: long, empty pieces, multibyte at the cuts.
    for input in [
        "://",
        "https://",
        "https:///",
        "http://discord.new/\u{1F600}",
        "\u{1F600}",
        "www.",
        "www./a",
        "discord.new/\u{0}",
        &"/".repeat(10_000),
        &"discord.new/".repeat(1000),
    ] {
        let _ = parse_template_link(input);
    }
}
