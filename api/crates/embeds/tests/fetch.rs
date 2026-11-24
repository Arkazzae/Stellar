//! The fetching end to end against servers inside the tests. A made-up public
//! name leads to them through the resolver table of the client.

mod support;

use std::time::{Duration, Instant};

use stellarr_embeds::{
    EmbedSettings, Failure, ImageKind, Link, NoCard, PreviewKind, VideoLink, YOUTUBE,
};
use stellarr_outbound::AddressPolicy;
use support::{Server, fetcher, handler};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x01\x02\x03";

fn link(text: &str) -> Link {
    Link::parse(text).unwrap()
}

fn settings() -> EmbedSettings {
    EmbedSettings {
        timeout_seconds: 5,
        ..EmbedSettings::default()
    }
}

async fn card_of(
    server: &Server,
    settings: EmbedSettings,
) -> Result<stellarr_embeds::PageCard, Failure> {
    fetcher(&["page.test"], settings)
        .fetch_card(&link(&server.url("page.test", "/")))
        .await
}

#[tokio::test]
async fn a_normal_page_becomes_a_card_with_its_picture_and_says_who_is_asking() {
    let server = Server::start(handler(|mut conn| async move {
        if conn.request.target == "/cover.png" {
            conn.ok("image/png", PNG).await;
        } else {
            conn.ok(
                "text/html; charset=utf-8",
                "<html><head><title>Fallback</title><meta property=\"og:title\" content=\"Tytuł &amp; więcej\">\
                 <meta property=\"og:description\" content=\"Opis\"><meta property=\"og:image\" content=\"/cover.png\"></head></html>"
                    .as_bytes(),
            )
            .await;
        }
    }))
    .await;
    let fetcher = fetcher(&["page.test"], settings());

    let preview = fetcher
        .preview(&link(&server.url("page.test", "/story")))
        .await
        .unwrap();

    assert_eq!(preview.kind, PreviewKind::Link);
    assert_eq!(preview.domain, "page.test");
    assert_eq!(preview.provider, "page.test");
    assert_eq!(preview.title.as_deref(), Some("Tytuł & więcej"));
    assert_eq!(preview.description.as_deref(), Some("Opis"));
    let picture = preview.thumbnail.unwrap();
    assert_eq!(picture.bytes, PNG);
    assert_eq!(picture.content_type(), "image/png");
    assert_eq!(picture.file_name(), "preview.png");
    assert_eq!(picture.size(), PNG.len());

    let first = &server.recorded()[0];
    assert_eq!(
        first.header("host"),
        Some(format!("page.test:{}", server.addr.port()).as_str())
    );
    assert!(
        first
            .header("user-agent")
            .unwrap()
            .starts_with("StellarrBot/1.0")
    );
    assert!(first.header("accept").unwrap().starts_with("text/html"));
    assert_eq!(first.header("accept-language"), Some("pl,en;q=0.8"));
    assert_eq!(first.header("accept-encoding"), Some("gzip, deflate"));
}

#[tokio::test]
async fn a_private_address_is_refused_unless_the_operator_allows_it() {
    let server = Server::page("text/html", b"<title>Local &amp; private</title>".to_vec()).await;
    let closed = fetcher(&["page.test"], settings()).with_address_policy(AddressPolicy::PublicOnly);
    for address in [
        server.url("page.test", "/"),
        format!("http://127.0.0.1:{}/", server.addr.port()),
    ] {
        assert_eq!(
            closed.fetch_card(&link(&address)).await,
            Err(Failure::Refused),
            "{address}"
        );
    }
    assert_eq!(server.hits(), 0, "something connected");
    let preview = closed.preview(&link(&server.url("page.test", "/"))).await;
    assert_eq!(preview, Err(NoCard::Unreadable(Failure::Refused)));

    let open = fetcher(&["page.test"], settings());
    let card = open
        .fetch_card(&link(&server.url("page.test", "/")))
        .await
        .unwrap();
    assert_eq!(card.title.as_deref(), Some("Local & private"));
}

#[tokio::test]
async fn pages_are_read_in_the_character_set_they_declare() {
    let polish_1250: &[u8] = b"\xa3\xf3d\x9f";
    let polish_8859_2: &[u8] = b"\xa3\xf3d\xbc";
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        (
            "text/html; charset=iso-8859-1",
            [&b"<title>Caf\xe9 \xdcn\xefcode</title>"[..]].concat(),
            "Café Ünïcode",
        ),
        (
            "text/html; charset=windows-1250",
            [&b"<title>"[..], polish_1250, b"</title>"].concat(),
            "Łódź",
        ),
        (
            "text/html; charset=\"ISO-8859-2\"",
            [&b"<title>"[..], polish_8859_2, b"</title>"].concat(),
            "Łódź",
        ),
        (
            "text/html; charset=windows-1251",
            b"<title>\xcf\xf0\xe8\xe2\xe5\xf2</title>".to_vec(),
            "Привет",
        ),
        (
            "text/html; charset=koi8-r",
            b"<title>\xf0\xd2\xc9\xd7\xc5\xd4</title>".to_vec(),
            "Привет",
        ),
        (
            "text/html; charset=utf-8",
            "<title>Zażółć</title>".as_bytes().to_vec(),
            "Zażółć",
        ),
        (
            "text/html",
            [&b"\xef\xbb\xbf"[..], "<title>Zażółć</title>".as_bytes()].concat(),
            "Zażółć",
        ),
        (
            "text/html; charset=windows-1251",
            [&b"\xef\xbb\xbf"[..], "<title>Zażółć</title>".as_bytes()].concat(),
            "Zażółć",
        ),
        (
            "text/html",
            [
                &b"<meta charset=windows-1250><title>"[..],
                polish_1250,
                b"</title>",
            ]
            .concat(),
            "Łódź",
        ),
        (
            "application/xhtml+xml",
            "<title>Zażółć</title>".as_bytes().to_vec(),
            "Zażółć",
        ),
        // Unknown and unsupported names read as UTF-8.
        (
            "text/html; charset=klingon",
            "<title>Zażółć</title>".as_bytes().to_vec(),
            "Zażółć",
        ),
        (
            "text/html; charset=shift_jis",
            b"<title>a\x83\x41b</title>".to_vec(),
            "a\u{FFFD}Ab",
        ),
    ];
    for (content_type, body, expected) in cases {
        let server = Server::page(Box::leak(content_type.to_owned().into_boxed_str()), body).await;
        let card = card_of(&server, settings())
            .await
            .unwrap_or_else(|e| panic!("{content_type}: {e}"));
        assert_eq!(card.title.as_deref(), Some(expected), "{content_type}");
    }

    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend(
        "<title>Zażółć 😀</title>"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    let server = Server::page("text/html", utf16).await;
    assert_eq!(
        card_of(&server, settings()).await.unwrap().title.as_deref(),
        Some("Zażółć 😀")
    );
}

#[tokio::test]
async fn a_page_announcing_more_than_the_cap_is_not_read_and_a_streamed_one_is_read_to_the_cap() {
    let page = format!(
        "<html><head><meta property=\"og:title\" content=\"Streamed\">{}<meta property=\"og:description\" content=\"Past the cap\"></head></html>",
        " ".repeat(2048)
    );
    let small = EmbedSettings {
        max_html_bytes: 1024,
        ..settings()
    };

    let announced = Server::page("text/html", page.clone().into_bytes()).await;
    assert_eq!(
        card_of(&announced, small.clone()).await,
        Err(Failure::TooLarge)
    );

    let page_bytes = page.into_bytes();
    let streamed = Server::start(handler(move |mut conn| {
        let page = page_bytes.clone();
        async move { conn.streamed("text/html", &page).await }
    }))
    .await;
    let card = card_of(&streamed, small).await.unwrap();
    assert_eq!(card.title.as_deref(), Some("Streamed"));
    assert_eq!(card.description, None);
}

#[tokio::test]
async fn a_page_of_exactly_the_cap_is_whole() {
    let mut page = b"<title>Exact</title>".to_vec();
    page.resize(1024, b' ');
    let server = Server::page("text/html", page).await;
    let card = card_of(
        &server,
        EmbedSettings {
            max_html_bytes: 1024,
            ..settings()
        },
    )
    .await
    .unwrap();
    assert_eq!(card.title.as_deref(), Some("Exact"));
}

#[tokio::test]
async fn a_picture_over_the_cap_is_dropped_whether_announced_or_streamed() {
    let mut oversized = PNG.to_vec();
    oversized.extend(vec![0u8; 4096]);
    let small = EmbedSettings {
        max_image_bytes: 1024,
        ..settings()
    };
    let fetcher = fetcher(&["page.test"], small);

    let announced = Server::page("image/png", oversized.clone()).await;
    assert_eq!(
        fetcher
            .fetch_image(&link(&announced.url("page.test", "/a.png")))
            .await,
        Err(Failure::TooLarge)
    );

    let streamed = Server::start(handler(move |mut conn| {
        let body = oversized.clone();
        async move { conn.streamed("image/png", &body).await }
    }))
    .await;
    assert_eq!(
        fetcher
            .fetch_image(&link(&streamed.url("page.test", "/a.png")))
            .await,
        Err(Failure::TooLarge)
    );

    // Exactly the cap is kept.
    let mut exact = PNG.to_vec();
    exact.resize(1024, 0);
    let exact_server = Server::page("image/png", exact.clone()).await;
    let image = fetcher
        .fetch_image(&link(&exact_server.url("page.test", "/a.png")))
        .await
        .unwrap();
    assert_eq!(image.bytes, exact);
}

#[tokio::test]
async fn redirects_are_followed_within_the_limit_and_relative_addresses_resolve_from_the_end() {
    let end = Server::start(handler(|mut conn| async move {
        conn.ok(
            "text/html",
            b"<meta property=\"og:title\" content=\"End\"><meta property=\"og:image\" content=\"pic.png\">",
        )
        .await;
    }))
    .await;
    let port = end.addr.port();
    let middle = Server::start(handler(move |mut conn| async move {
        conn.redirect(301, &format!("http://end.test:{port}/deep/page"))
            .await;
    }))
    .await;
    let start_port = middle.addr.port();
    let start = Server::start(handler(move |mut conn| async move {
        conn.redirect(302, &format!("http://middle.test:{start_port}/x"))
            .await;
    }))
    .await;
    let fetcher = fetcher(
        &["start.test", "middle.test", "end.test"],
        EmbedSettings {
            max_redirects: 2,
            ..settings()
        },
    );

    let card = fetcher
        .fetch_card(&link(&start.url("start.test", "/")))
        .await
        .unwrap();
    assert_eq!(card.title.as_deref(), Some("End"));
    assert_eq!(
        card.image_url,
        Some(format!("http://end.test:{port}/deep/pic.png"))
    );

    let tight = self::fetcher(
        &["start.test", "middle.test", "end.test"],
        EmbedSettings {
            max_redirects: 1,
            ..settings()
        },
    );
    assert_eq!(
        tight.fetch_card(&link(&start.url("start.test", "/"))).await,
        Err(Failure::Redirects)
    );
}

#[tokio::test]
async fn a_redirect_loop_ends_and_a_relative_location_works() {
    let looping = Server::start(handler(|mut conn| async move {
        conn.redirect(302, "/again").await
    }))
    .await;
    let fetcher = fetcher(&["page.test"], settings());
    assert_eq!(
        fetcher
            .fetch_card(&link(&looping.url("page.test", "/")))
            .await,
        Err(Failure::Redirects)
    );
    assert!(looping.hits() <= 5);

    let relative = Server::start(handler(|mut conn| async move {
        if conn.request.target == "/final" {
            conn.ok("text/html", b"<title>Relative</title>").await;
        } else {
            conn.redirect(307, "final").await;
        }
    }))
    .await;
    let card = fetcher
        .fetch_card(&link(&relative.url("page.test", "/")))
        .await
        .unwrap();
    assert_eq!(card.title.as_deref(), Some("Relative"));
}

#[tokio::test]
async fn a_redirect_to_a_private_address_is_refused_at_that_hop() {
    // The first hop is loopback, which this policy allows; the second names
    // a host that resolves to a private address, which it does not.
    let first = Server::start(handler(|mut conn| async move {
        conn.redirect(302, "http://inner.test:80/secret").await;
    }))
    .await;
    let client = {
        use std::sync::Arc;
        use stellarr_outbound::{Client, TableResolver};
        Client::new().with_resolver(Arc::new(TableResolver(vec![
            ("page.test".into(), vec!["127.0.0.1".parse().unwrap()]),
            ("inner.test".into(), vec!["10.1.2.3".parse().unwrap()]),
        ])))
    };
    let fetcher = stellarr_embeds::Fetcher::with_client(client, settings())
        .with_address_policy(AddressPolicy::LoopbackOnly);
    assert_eq!(
        fetcher
            .fetch_card(&link(&first.url("page.test", "/")))
            .await,
        Err(Failure::Refused)
    );
    assert_eq!(first.hits(), 1);
}

#[tokio::test]
async fn a_host_that_never_answers_or_stalls_in_the_body_gives_up_and_the_next_fetch_works() {
    let silent = Server::start(handler(|mut conn| async move { conn.stall().await })).await;
    let stalled_page = Server::start(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 100\r\n\r\n")
            .await;
        conn.stall().await;
    }))
    .await;
    let stalled_image = Server::start(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 100\r\n\r\n")
            .await;
        conn.stall().await;
    }))
    .await;
    let healthy = Server::page("text/html", b"<title>Next card</title>".to_vec()).await;
    let fetcher = fetcher(
        &["page.test"],
        EmbedSettings {
            timeout_seconds: 1,
            ..settings()
        },
    );

    for (what, result) in [
        (
            "silent",
            fetcher
                .fetch_card(&link(&silent.url("page.test", "/")))
                .await
                .err(),
        ),
        (
            "stalled page",
            fetcher
                .fetch_card(&link(&stalled_page.url("page.test", "/")))
                .await
                .err(),
        ),
        (
            "stalled image",
            fetcher
                .fetch_image(&link(&stalled_image.url("page.test", "/")))
                .await
                .err(),
        ),
    ] {
        assert_eq!(result, Some(Failure::Timeout), "{what}");
    }
    let started = Instant::now();
    let _ = fetcher
        .fetch_card(&link(&silent.url("page.test", "/")))
        .await;
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "took {:?}",
        started.elapsed()
    );

    let card = fetcher
        .fetch_card(&link(&healthy.url("page.test", "/")))
        .await
        .unwrap();
    assert_eq!(card.title.as_deref(), Some("Next card"));
}

#[tokio::test]
async fn a_picture_that_stalls_leaves_the_card_without_its_picture() {
    let stalled = Server::start(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 100\r\n\r\n")
            .await;
        conn.stall().await;
    }))
    .await;
    let port = stalled.addr.port();
    let page = Server::start(handler(move |mut conn| async move {
        let body = format!("<title>Image times out</title><meta property=\"og:image\" content=\"http://img.test:{port}/p.png\">");
        conn.ok("text/html", body.as_bytes()).await;
    }))
    .await;
    let fetcher = fetcher(
        &["page.test", "img.test"],
        EmbedSettings {
            timeout_seconds: 1,
            ..settings()
        },
    );
    let preview = fetcher
        .preview(&link(&page.url("page.test", "/")))
        .await
        .unwrap();
    assert_eq!(preview.title.as_deref(), Some("Image times out"));
    assert!(preview.thumbnail.is_none());
}

#[tokio::test]
async fn a_page_that_lies_about_its_type_is_not_taken_for_what_it_claims() {
    let fetcher = fetcher(&["page.test"], settings());
    let image = |server: &Server| link(&server.url("page.test", "/a"));

    // Claims a picture, is a script.
    let script = Server::page("image/png", b"<script>alert(1)</script>".to_vec()).await;
    assert_eq!(
        fetcher.fetch_image(&image(&script)).await,
        Err(Failure::NotAnImage)
    );
    // Claims one kind, is another.
    let wrong_kind = Server::page("image/jpeg", PNG.to_vec()).await;
    assert_eq!(
        fetcher.fetch_image(&image(&wrong_kind)).await,
        Err(Failure::NotAnImage)
    );
    // Is a picture, claims a page, or nothing helpful.
    for claimed in [
        "text/html",
        "application/octet-stream",
        "image/svg+xml",
        "image/jpg",
    ] {
        let server =
            Server::page(Box::leak(claimed.to_owned().into_boxed_str()), PNG.to_vec()).await;
        assert_eq!(
            fetcher.fetch_image(&image(&server)).await,
            Err(Failure::NotAnImage),
            "{claimed}"
        );
    }
    // Claims a page, is a picture: read as a page, it says nothing.
    let png_as_page = Server::page("text/html", PNG.to_vec()).await;
    assert_eq!(
        fetcher
            .preview(&link(&png_as_page.url("page.test", "/")))
            .await,
        Err(NoCard::Empty)
    );
    // Is a page, claims a picture or JSON: not read.
    for claimed in ["image/png", "application/json", "text/plain"] {
        let server = Server::page(
            Box::leak(claimed.to_owned().into_boxed_str()),
            b"<title>X</title>".to_vec(),
        )
        .await;
        assert_eq!(
            fetcher
                .fetch_card(&link(&server.url("page.test", "/")))
                .await,
            Err(Failure::NotAPage),
            "{claimed}"
        );
    }
    // No type at all.
    let untyped = Server::start(handler(|mut conn| async move {
        conn.respond("200 OK", &[], b"<title>X</title>").await;
    }))
    .await;
    assert_eq!(
        fetcher
            .fetch_card(&link(&untyped.url("page.test", "/")))
            .await,
        Err(Failure::NotAPage)
    );
    // A zero-length picture.
    let empty = Server::page("image/png", Vec::new()).await;
    assert_eq!(
        fetcher.fetch_image(&image(&empty)).await,
        Err(Failure::Empty)
    );
}

#[tokio::test]
async fn every_kind_of_picture_is_kept_when_its_bytes_agree() {
    let fetcher = fetcher(&["page.test"], settings());
    for (media, bytes, kind) in [
        ("image/jpeg", &b"\xff\xd8\xff\xe0abc"[..], ImageKind::Jpeg),
        ("image/png", PNG, ImageKind::Png),
        ("IMAGE/GIF", b"GIF89a....", ImageKind::Gif),
        ("image/webp", b"RIFF\x04\0\0\0WEBPVP8 ", ImageKind::Webp),
    ] {
        let server =
            Server::page(Box::leak(media.to_owned().into_boxed_str()), bytes.to_vec()).await;
        let image = fetcher
            .fetch_image(&link(&server.url("page.test", "/a")))
            .await
            .unwrap();
        assert_eq!(image.kind, kind);
        assert_eq!(image.bytes, bytes);
    }
}

#[tokio::test]
async fn an_answer_that_is_not_a_success_gives_no_card() {
    let fetcher = fetcher(&["page.test"], settings());
    let missing = Server::start(handler(|mut conn| async move {
        conn.respond(
            "404 Not Found",
            &[("Content-Type", "text/html")],
            b"<title>Gone</title>",
        )
        .await;
    }))
    .await;
    assert_eq!(
        fetcher
            .fetch_card(&link(&missing.url("page.test", "/")))
            .await,
        Err(Failure::Status(404))
    );
    assert_eq!(
        fetcher.preview(&link(&missing.url("page.test", "/"))).await,
        Err(NoCard::Unreadable(Failure::Status(404)))
    );
}

/// A body made by Python's gzip (see `fixtures/gzip.py`).
fn gzipped(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gzip")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// A server that answers every request with `body`, declared as `coding`.
async fn compressed(content_type: &'static str, coding: &'static str, body: Vec<u8>) -> Server {
    Server::start(handler(move |mut conn| {
        let body = body.clone();
        async move {
            conn.respond(
                "200 OK",
                &[("Content-Type", content_type), ("Content-Encoding", coding)],
                &body,
            )
            .await;
        }
    }))
    .await
}

#[tokio::test]
async fn a_page_answered_with_a_real_gzip_body_becomes_a_card() {
    let server = compressed("text/html", "gzip", gzipped("page.html.gz")).await;
    let card = card_of(&server, settings()).await.unwrap();
    assert_eq!(card.title.as_deref(), Some("Compressed card"));
    assert_eq!(card.description.as_deref(), Some("Said in gzip"));
}

#[tokio::test]
async fn a_compressed_page_that_is_cut_short_gives_no_card() {
    let whole = gzipped("page.html.gz");
    for cut in [4, whole.len() / 2, whole.len() - 3] {
        let server = compressed("text/html", "gzip", whole[..cut].to_vec()).await;
        assert_eq!(
            card_of(&server, settings()).await,
            Err(Failure::Network),
            "cut at {cut}"
        );
    }
}

#[tokio::test]
async fn an_answer_in_a_coding_the_client_does_not_undo_gives_no_card() {
    for coding in ["br", "zstd", "gzip, br"] {
        let server = compressed("text/html", coding, gzipped("page.html.gz")).await;
        assert_eq!(
            card_of(&server, settings()).await,
            Err(Failure::Network),
            "{coding}"
        );
    }
}

#[tokio::test]
async fn a_compressed_page_past_the_cap_is_read_to_the_cap_like_a_streamed_one() {
    // The size is not announced up front, since the length on the wire says
    // nothing about it: the page is read as far as the cap, and the
    // description past the cap is not seen.
    let server = compressed("text/html", "gzip", gzipped("long-page.html.gz")).await;
    let card = card_of(
        &server,
        EmbedSettings {
            max_html_bytes: 1024,
            ..settings()
        },
    )
    .await
    .unwrap();
    assert_eq!(card.title.as_deref(), Some("Streamed"));
    assert_eq!(card.description, None);

    // A page that inflates to megabytes from a few kilobytes costs the cap.
    let bomb = compressed("text/html", "gzip", gzipped("bomb-page.html.gz")).await;
    let card = card_of(
        &bomb,
        EmbedSettings {
            max_html_bytes: 1024,
            ..settings()
        },
    )
    .await
    .unwrap();
    assert_eq!(card.title.as_deref(), Some("Bomb"));
}

#[tokio::test]
async fn a_compressed_picture_over_the_cap_is_dropped_like_an_uncompressed_one() {
    let small = EmbedSettings {
        max_image_bytes: 1024,
        ..settings()
    };
    let fetcher = fetcher(&["page.test"], small);
    for file in ["big-picture.png.gz", "bomb-picture.png.gz"] {
        let server = compressed("image/png", "gzip", gzipped(file)).await;
        assert_eq!(
            fetcher
                .fetch_image(&link(&server.url("page.test", "/a.png")))
                .await,
            Err(Failure::TooLarge),
            "{file}"
        );
    }
}

#[tokio::test]
async fn a_compressed_picture_is_stored_as_the_bytes_an_uncompressed_fetch_stores() {
    let fetcher = fetcher(&["page.test"], settings());
    let plain = Server::page("image/png", PNG.to_vec()).await;
    let packed = compressed("image/png", "gzip", gzipped("picture.png.gz")).await;
    let from_plain = fetcher
        .fetch_image(&link(&plain.url("page.test", "/a.png")))
        .await
        .unwrap();
    let from_packed = fetcher
        .fetch_image(&link(&packed.url("page.test", "/a.png")))
        .await
        .unwrap();
    assert_eq!(from_packed, from_plain);
    assert_eq!(from_packed.bytes, PNG);

    // A damaged or cut body is no picture at all, not a shorter one.
    let whole = gzipped("picture.png.gz");
    let mut damaged = whole.clone();
    let last = damaged.len() - 6;
    damaged[last] ^= 1;
    for body in [whole[..whole.len() - 2].to_vec(), damaged] {
        let server = compressed("image/png", "gzip", body).await;
        assert_eq!(
            fetcher
                .fetch_image(&link(&server.url("page.test", "/a.png")))
                .await,
            Err(Failure::Network)
        );
    }
}

#[tokio::test]
async fn a_name_in_another_script_is_requested_in_punycode() {
    let server = Server::page("text/html", "<title>Książki</title>".as_bytes().to_vec()).await;
    let fetcher = fetcher(&["xn--ksiki-5wa74f.test"], settings());
    let card = fetcher
        .fetch_card(&link(&format!(
            "http://książki.test:{}/ścieżka?x=ż",
            server.addr.port()
        )))
        .await
        .unwrap();
    assert_eq!(card.title.as_deref(), Some("Książki"));
    let first = &server.recorded()[0];
    assert_eq!(first.target, "/%C5%9Bcie%C5%BCka?x=%C5%BC");
}

#[tokio::test]
async fn gifs_are_left_alone_and_switched_off_previews_fetch_nothing() {
    let server = Server::page("text/html", b"<title>X</title>".to_vec()).await;
    let fetcher = fetcher(&["page.test"], settings());
    assert_eq!(
        fetcher
            .preview(&link("https://giphy.com/gifs/happy-AbCd1234"))
            .await,
        Err(NoCard::Giphy)
    );

    let off = self::fetcher(
        &["page.test"],
        EmbedSettings {
            enabled: false,
            ..settings()
        },
    );
    assert_eq!(
        off.preview(&link(&server.url("page.test", "/"))).await,
        Err(NoCard::Disabled)
    );
    let video = off
        .preview(&link("https://youtu.be/aqz-KE-bpKQ?t=95"))
        .await
        .unwrap();
    assert_eq!(video.kind, PreviewKind::Video);
    assert_eq!(video.provider, "youtube");
    assert_eq!(video.provider_id.as_deref(), Some("aqz-KE-bpKQ"));
    assert_eq!(video.start_seconds, Some(95));
    assert_eq!(
        (video.title, video.description, video.thumbnail),
        (None, None, None)
    );
    assert_eq!(server.hits(), 0);
}

#[tokio::test]
async fn a_youtube_link_is_a_player_whatever_the_endpoint_says() {
    let endpoint_for = |server: &Server| format!("http://yt.test:{}/oembed", server.addr.port());
    let video = VideoLink {
        provider: YOUTUBE,
        id: "aqz-KE-bpKQ".into(),
        start_seconds: None,
    };

    let answering = Server::start(handler(|mut conn| async move {
        conn.ok(
            "application/json; charset=utf-8",
            br#"{"type":"video","provider_name":"YouTube","title":"Big Buck Bunny","thumbnail_url":"https://i.ytimg.com/vi/aqz-KE-bpKQ/hqdefault.jpg"}"#,
        )
        .await;
    }))
    .await;
    let fetcher = fetcher(&["yt.test"], settings()).with_oembed_endpoint(endpoint_for(&answering));
    let card = fetcher.fetch_youtube(&video).await.unwrap();
    assert_eq!(card.title.as_deref(), Some("Big Buck Bunny"));
    let target = &answering.recorded()[0].target;
    assert_eq!(
        target,
        "/oembed?url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Daqz-KE-bpKQ&format=json"
    );
    assert_eq!(
        answering.recorded()[0].header("accept"),
        Some("application/json")
    );

    // The card is a video card with the title; the thumbnail is on a host this
    // test does not reach, so the card goes without it.
    let preview = fetcher
        .preview(&link("https://www.youtube.com/watch?v=aqz-KE-bpKQ&t=1m"))
        .await
        .unwrap();
    assert_eq!(preview.kind, PreviewKind::Video);
    assert_eq!(preview.title.as_deref(), Some("Big Buck Bunny"));
    assert_eq!(preview.start_seconds, Some(60));
    assert!(preview.thumbnail.is_none());

    // A bad answer of any kind still leaves a player.
    for (status, content_type, body) in [
        ("500 Oops", "application/json", &b"{}"[..]),
        (
            "200 OK",
            "text/html",
            br#"{"type":"video","provider_name":"YouTube","title":"T"}"#,
        ),
        ("200 OK", "application/json", b"not json"),
        ("404 Not Found", "application/json", b"{}"),
    ] {
        let bad = Server::start(handler(move |mut conn| async move {
            conn.respond(status, &[("Content-Type", content_type)], body)
                .await;
        }))
        .await;
        let fetcher =
            self::fetcher(&["yt.test"], settings()).with_oembed_endpoint(endpoint_for(&bad));
        assert!(
            fetcher.fetch_youtube(&video).await.is_err(),
            "{status} {content_type}"
        );
        let preview = fetcher
            .preview(&link("https://youtu.be/aqz-KE-bpKQ"))
            .await
            .unwrap();
        assert_eq!(preview.kind, PreviewKind::Video);
        assert_eq!(preview.title, None);
    }

    // An oversized answer is dropped.
    let big = Server::start(handler(|mut conn| async move {
        conn.ok("application/json", &vec![b' '; 100 * 1024]).await;
    }))
    .await;
    let fetcher = self::fetcher(&["yt.test"], settings()).with_oembed_endpoint(endpoint_for(&big));
    assert_eq!(fetcher.fetch_youtube(&video).await, Err(Failure::TooLarge));
}

#[tokio::test]
async fn vimeo_is_read_as_an_ordinary_page_and_an_unreachable_one_is_still_a_player() {
    let fetcher = fetcher(&["vimeo.com"], settings());
    // The name is in the table but nothing listens on that port.
    let preview = fetcher
        .preview(&link("http://vimeo.com:9/123456789"))
        .await
        .unwrap();
    assert_eq!(preview.kind, PreviewKind::Video);
    assert_eq!(preview.provider, "vimeo");
    assert_eq!(preview.title, None);
}

#[tokio::test]
async fn a_page_that_says_nothing_gets_no_card() {
    let server = Server::page(
        "text/html",
        b"<html><head></head><body>Hi</body></html>".to_vec(),
    )
    .await;
    let fetcher = fetcher(&["page.test"], settings());
    assert_eq!(
        fetcher.preview(&link(&server.url("page.test", "/"))).await,
        Err(NoCard::Empty)
    );
}

/// Fetches one real public page and prints the card. Run by hand:
/// `cargo test -p stellarr-embeds --test fetch -- --ignored --nocapture`
#[tokio::test]
#[ignore = "reaches the public internet"]
async fn a_real_page_by_hand() {
    let fetcher = stellarr_embeds::Fetcher::new(EmbedSettings::default());
    let started = Instant::now();
    let result = fetcher.preview(&link("https://www.rust-lang.org/")).await;
    println!("took {:?}", started.elapsed());
    match result {
        Ok(p) => println!(
            "{:?} {} provider={} title={:?} description={:?} picture={:?}",
            p.kind,
            p.url,
            p.provider,
            p.title,
            p.description,
            p.thumbnail.map(|t| (t.content_type(), t.size()))
        ),
        Err(reason) => println!("no card: {reason}"),
    }
}
