//! The metadata reader and its neighbours against hostile input: no panic,
//! and work in proportion to the size. Each case prints its time; run with
//! `--nocapture` to see them. The sizes are the default cap on a page
//! (512 KiB) and sixteen times that.

use std::time::{Duration, Instant};

use stellarr_embeds::charset::decode_page;
use stellarr_embeds::{Link, decode_entities, extract_links, parse_open_graph};

const CAP: usize = 512 * 1024;

fn page() -> Link {
    Link::parse("https://example.org/a/b").unwrap()
}

fn repeat_to(unit: &str, bytes: usize) -> String {
    unit.repeat(bytes / unit.len() + 1)
}

/// Named hostile pages, each built to the given size.
fn hostile(size: usize) -> Vec<(&'static str, String)> {
    let huge_value = format!(
        "<meta property=\"og:title\" content=\"{}\">",
        "a".repeat(size)
    );
    vec![
        ("unclosed meta", repeat_to("<meta ", size)),
        ("unclosed title", repeat_to("<title ", size)),
        ("title openings, no close", repeat_to("<title>", size)),
        ("unclosed quote", repeat_to("<meta content=\"", size)),
        ("unclosed bare", repeat_to("<meta a=b ", size)),
        ("closing head lookalikes", repeat_to("</head ", size)),
        (
            "closing head, spaces",
            format!("</head{}", " ".repeat(size)),
        ),
        (
            "a million meta tags",
            repeat_to("<meta name=x content=y>", size.max(1_000_000 * 23 / 8)),
        ),
        (
            "a million wanted meta tags",
            repeat_to("<meta name=og:title content=y>", size),
        ),
        ("huge attribute", huge_value),
        (
            "huge attribute, entities",
            format!(
                "<meta property=\"og:title\" content=\"{}\">",
                "&amp;".repeat(size / 5)
            ),
        ),
        (
            "nested quotes",
            repeat_to("<meta content='\"'\"'\"' name=\"'\">", size),
        ),
        (
            "null bytes",
            repeat_to("<meta\0 property\0=\"og:title\0\" content=\"\0\">\0", size),
        ),
        ("only brackets", repeat_to("<<<<>>>>", size)),
        ("many equals", format!("<meta {}>", "=".repeat(size))),
        (
            "many names, no values",
            format!("<meta {}>", "a ".repeat(size / 2)),
        ),
        (
            "many attributes",
            format!("<meta {}>", "a=b ".repeat(size / 4)),
        ),
        ("lone ampersands", repeat_to("&", size)),
        ("ampersand runs with hash", repeat_to("&#x&#&#;&;&", size)),
        (
            "long numeric references",
            format!("<title>&#{}1;</title>", "0".repeat(size)),
        ),
        (
            "title with many openings inside",
            format!("<title>{}</title>", repeat_to("<title>", 390)),
        ),
        (
            "multibyte everywhere",
            repeat_to("<meta name=\"ż😀\" content=\"ł\u{200b}\">", size),
        ),
    ]
}

#[test]
fn the_reader_costs_time_in_proportion_to_size_and_never_panics() {
    let mut worst = (Duration::ZERO, "");
    for size in [CAP, 16 * CAP] {
        for (name, html) in hostile(size) {
            let started = Instant::now();
            let card = parse_open_graph(&html, &page());
            let took = started.elapsed();
            println!("{size:>8} bytes  {:>9.1?}  {name}", took);
            if took > worst.0 {
                worst = (took, name);
            }
            // Whatever came out is bounded text.
            for field in [&card.title, &card.description].into_iter().flatten() {
                assert!(field.chars().count() <= 401, "{name}");
            }
        }
    }
    println!("slowest: {:?} on {}", worst.0, worst.1);
    // Debug build, sixteen times the cap: several seconds would still be
    // linear, tens would not.
    assert!(worst.0 < Duration::from_secs(20), "{worst:?}");
}

#[test]
fn the_slowest_case_at_the_cap_is_quick() {
    let mut slowest = Duration::ZERO;
    for (name, html) in hostile(CAP) {
        let started = Instant::now();
        parse_open_graph(&html, &page());
        let took = started.elapsed();
        slowest = slowest.max(took);
        assert!(took < Duration::from_secs(3), "{name}: {took:?}");
    }
    println!("slowest at the cap: {slowest:?}");
}

/// The input that holds the most memory at once: a head that is one huge
/// wanted value, decoded, collapsed and cut. Run on its own by the
/// measurement in the report.
#[test]
fn the_worst_case_for_memory() {
    let html = format!(
        "<meta property=\"og:title\" content=\"{}\"><meta property=\"og:description\" content=\"{}\">",
        "&amp; ".repeat(CAP / 12),
        "&#x1F600; ".repeat(CAP / 24)
    );
    let started = Instant::now();
    let card = parse_open_graph(&html, &page());
    println!("{} bytes in, {:?}", html.len(), started.elapsed());
    let title = card.title.unwrap();
    assert!(
        title.ends_with('…') && title.chars().count() <= 201,
        "{title}"
    );
    assert!(card.description.is_some());
}

#[test]
fn entities_of_every_form_in_a_card() {
    let html = concat!(
        "<head><meta property=\"og:title\" content=\"",
        "&amp;&lt;&gt;&quot;&apos;&nbsp;&eacute;&#233;&#xE9;&#XE9;&#0233;&#x0E9;",
        "&amp&#;&#x;&#xZ;&#99999999999;&#xD800;&bogus;&;&&amp;lt;",
        "\"></head>"
    );
    let title = parse_open_graph(html, &page()).title.unwrap();
    assert_eq!(
        title,
        "&<>\"' éééééé&amp&#;&#x;&#xZ;&#99999999999;&#xD800;&bogus;&;&&lt;"
    );
}

#[test]
fn text_helpers_survive_hostile_input_too() {
    let size = CAP;
    let started = Instant::now();
    assert_eq!(decode_entities(&"&".repeat(size)).len(), size);
    decode_entities(&repeat_to("&#x", size));
    decode_entities(&repeat_to("&amp", size));
    extract_links(&repeat_to("http://", size), 2);
    extract_links(&repeat_to("https://a)", size), 2);
    extract_links(&format!("https://a{}", ")".repeat(size)), 2);
    extract_links(&format!("https://a/{}", "(".repeat(size)), 2);
    extract_links(&repeat_to("\0\u{1}http://\u{7f}", size), 2);
    Link::parse(&format!("http://{}", "[".repeat(size)));
    Link::parse(&format!("http://a/{}", "../".repeat(size / 3)));
    Link::parse(&format!("http://{}.example/", "ż".repeat(size / 2)));
    let bytes: Vec<u8> = (0..size).map(|i| (i * 7 % 256) as u8).collect();
    for label in [
        None,
        Some("utf-8"),
        Some("utf-16"),
        Some("windows-1250"),
        Some("klingon"),
    ] {
        decode_page(&bytes, label);
    }
    decode_page(&repeat_to("<meta ", size).into_bytes(), None);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}
