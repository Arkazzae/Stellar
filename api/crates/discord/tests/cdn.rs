use stellarr_discord::{
    StickerFormat, avatar, emoji_url, is_cdn_url, member_avatar, server_icon, sticker_url,
};

#[test]
fn accepts_https_on_the_two_cdn_hosts() {
    for url in [
        "https://cdn.discordapp.com/attachments/1012345678900020080/1234567891233211234/my_image.png?ex=65d903de&is=65c68ede&hm=2481f30dd67f503f54d020ae3b5533b9987fae4e55f2b4e3926e08a3fa3ee24f&",
        "https://media.discordapp.net/attachments/1/2/clip.mp4",
        "https://media.discordapp.net/stickers/749054660769218631.gif",
        "https://CDN.DiscordApp.com/emojis/41771983429993937.webp",
        "https://cdn.discordapp.com:443/icons/1/abc.png",
    ] {
        assert!(is_cdn_url(Some(url)), "{url}");
    }
}

#[test]
fn refuses_everything_else() {
    assert!(!is_cdn_url(None));
    for url in [
        "",
        "/attachments/1/2/a.png",
        "cdn.discordapp.com/attachments/1/2/a.png",
        "http://cdn.discordapp.com/attachments/1/2/a.png",
        "ftp://cdn.discordapp.com/a.png",
        "https://cdn.discordapp.com.evil.test/a.png",
        "https://cdn.discordapp.com@evil.test/a.png",
        "https://evil.test@cdn.discordapp.com/a.png",
        "https://cdn.discordapp.com:8443/a.png",
        "https://evilcdn.discordapp.com/a.png",
        "https://images-ext-1.discordapp.net/external/x/https/evil.test/a.png",
        "https://discord.com/api/v10/users/@me",
        "https://cdn.discordapp.com./a.png",
        "https://127.0.0.1/a.png",
        // Not Discord's whatever a browser or another parser makes of them.
        "https://evil.test\\@cdn.discordapp.com/a.png",
        "https://cdn.discordapp.com\\@evil.test/a.png",
        "https://cdn.discordapp.com%2f@evil.test/a.png",
        "https://[::1]/a.png",
        "https://cdn.discordapp.com:0/a.png",
        "https://cdn.discordapp.com:/a.png",
        "https://cdn.discordapp.com:443:443/a.png",
        "https:///cdn.discordapp.com/a.png",
        "https://\u{ff43}dn.discordapp.com/a.png",
        "javascript:alert(1)//cdn.discordapp.com",
    ] {
        assert!(!is_cdn_url(Some(url)), "{url}");
    }
}

#[test]
fn builds_addresses_that_pass_its_own_check() {
    let built = [
        server_icon(
            "678070694164299796",
            Some("f749bb0cbeeb26ef21eca719337d20f1"),
        ),
        avatar(
            "268473310986240001",
            Some("a_bab14f271d565501444b2ca3be944b25"),
        ),
        member_avatar("1", "2", Some("abc")),
        Some(emoji_url("41771983429993937", true)),
        Some(sticker_url("749054660769218631", StickerFormat::Gif)),
        Some(sticker_url("749054660769218631", StickerFormat::Lottie)),
    ];
    for url in &built {
        assert!(is_cdn_url(url.as_deref()), "{url:?}");
    }
    assert_eq!(
        built[0].as_deref(),
        Some(
            "https://cdn.discordapp.com/icons/678070694164299796/f749bb0cbeeb26ef21eca719337d20f1.png"
        )
    );
    assert_eq!(
        built[1].as_deref(),
        Some(
            "https://cdn.discordapp.com/avatars/268473310986240001/a_bab14f271d565501444b2ca3be944b25.webp?animated=true"
        )
    );
    assert_eq!(
        built[4].as_deref(),
        Some("https://media.discordapp.net/stickers/749054660769218631.gif")
    );
    assert_eq!(avatar("1", None), None);
    assert_eq!(avatar("1", Some("")), None);
}

#[test]
fn an_id_or_hash_that_is_not_plain_cannot_change_where_the_address_goes() {
    for hostile in [
        "../x",
        "a/b",
        "a?b",
        "a#b",
        "a@b",
        "a b",
        "%2e%2e",
        "a\\b",
        "\u{1F600}",
        "a:b",
    ] {
        for url in [
            server_icon(hostile, Some("abc")),
            server_icon("1", Some(hostile)),
            avatar(hostile, Some("a_x")),
            member_avatar(hostile, hostile, Some(hostile)),
            Some(emoji_url(hostile, false)),
            Some(sticker_url(hostile, StickerFormat::Png)),
            Some(sticker_url(hostile, StickerFormat::Gif)),
        ] {
            let url = url.unwrap();
            assert!(is_cdn_url(Some(&url)), "{hostile:?} gave {url}");
            let path = url.split_once("discordapp.").map_or("", |(_, rest)| rest);
            assert!(
                !path.contains("/../") && !path.contains('@') && !path.contains('\\'),
                "{url}"
            );
        }
    }
    assert_eq!(
        emoji_url("a/b", true),
        "https://cdn.discordapp.com/emojis/a%2Fb.webp?animated=true"
    );
}
