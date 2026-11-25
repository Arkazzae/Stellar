//! What YouTube's own oEmbed endpoint says about a video.
//!
//! The watch page is never scraped and the link is never requested as it was
//! pasted. The link has already been proved to be a supported YouTube
//! address and its id taken out; the request is built from that id alone.
//! Recognising the video therefore never depends on this optional request
//! succeeding, and reading the page as generic metadata, which once turned
//! real videos into blank players, is not an option.

use serde_json::Value;

use crate::link::Link;
use crate::opengraph::PageCard;
use crate::text::clean_text;

/// The largest oEmbed answer read. The real one is a few hundred bytes.
pub const MAX_OEMBED_BYTES: usize = 64 * 1024;

/// The longest video title shown, in UTF-16 code units.
pub const MAX_OEMBED_TITLE: usize = 200;

/// The longest thumbnail address read, in UTF-16 code units. A real one is
/// about sixty.
pub const MAX_THUMBNAIL_URL: usize = 2_048;

/// The address that answers for a video id: the endpoint with the canonical
/// watch page of the video as its argument.
pub(crate) fn oembed_address(endpoint: &str, video_id: &str) -> String {
    let watch_page = format!("https://www.youtube.com/watch?v={}", escape_data(video_id));
    format!("{endpoint}?url={}&format=json", escape_data(&watch_page))
}

/// Percent-encodes everything but letters, digits and `-._~`.
fn escape_data(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Keeps the title and thumbnail of the answer and nothing else: the iframe
/// markup it carries never enters a card. `None` when the document is not
/// YouTube's description of a video, is not JSON, or has neither.
pub fn parse_oembed(json: &[u8], expected_video_id: &str) -> Option<PageCard> {
    let root: Value = serde_json::from_slice(json).ok()?;
    let text = |name: &str| root.get(name).and_then(Value::as_str);
    if !text("type")?.eq_ignore_ascii_case("video")
        || !text("provider_name")?.eq_ignore_ascii_case("YouTube")
    {
        return None;
    }
    let title = clean_text(text("title"), MAX_OEMBED_TITLE);
    let thumbnail = clean_text(text("thumbnail_url"), MAX_THUMBNAIL_URL)
        .and_then(|address| trusted_thumbnail(&address, expected_video_id));
    if title.is_none() && thumbnail.is_none() {
        return None;
    }
    Some(PageCard {
        title,
        description: None,
        image_url: thumbnail,
        fallback_image_url: None,
        animation_url: None,
    })
}

/// The thumbnail only when it is on YouTube's own image hosts and belongs to
/// this very video, so that an answer cannot point the fetcher anywhere
/// else.
fn trusted_thumbnail(address: &str, video_id: &str) -> Option<String> {
    let link = Link::parse(address)?;
    if !link.is_https() || !matches!(link.host(), "i.ytimg.com" | "img.youtube.com") {
        return None;
    }
    let path = link.path();
    (path.starts_with(&format!("/vi/{video_id}/"))
        || path.starts_with(&format!("/vi_webp/{video_id}/")))
    .then(|| link.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "aqz-KE-bpKQ";

    #[test]
    fn keeps_only_real_provider_metadata_for_the_expected_video() {
        let card = parse_oembed(
            br#"{
                "type": "video",
                "provider_name": "YouTube",
                "title": "Big Buck Bunny 60fps 4K &amp; extras",
                "thumbnail_url": "https://i.ytimg.com/vi/aqz-KE-bpKQ/hqdefault.jpg",
                "html": "<iframe src=\"https://www.youtube.com/embed/evil\"></iframe>"
            }"#,
            ID,
        )
        .unwrap();
        assert_eq!(
            card.title.as_deref(),
            Some("Big Buck Bunny 60fps 4K & extras")
        );
        assert_eq!(
            card.image_url.as_deref(),
            Some("https://i.ytimg.com/vi/aqz-KE-bpKQ/hqdefault.jpg")
        );
        assert_eq!(card.description, None);
    }

    #[test]
    fn does_not_trust_a_thumbnail_outside_the_validated_provider_and_id() {
        for (provider, thumbnail) in [
            ("Vimeo", "https://i.ytimg.com/vi/aqz-KE-bpKQ/hqdefault.jpg"),
            (
                "YouTube",
                "https://evil.example/vi/aqz-KE-bpKQ/hqdefault.jpg",
            ),
            (
                "YouTube",
                "https://i.ytimg.com/vi/jNQXAC9IVRw/hqdefault.jpg",
            ),
            ("YouTube", "http://i.ytimg.com/vi/aqz-KE-bpKQ/hqdefault.jpg"),
            (
                "YouTube",
                "https://i.ytimg.com@evil.example/vi/aqz-KE-bpKQ/a.jpg",
            ),
            (
                "YouTube",
                "https://i.ytimg.com/vi/aqz-KE-bpKQ/../../x/a.jpg",
            ),
            (
                "YouTube",
                "https://i.ytimg.com/vi/aqz-KE-bpKQ/%2e%2e/%2e%2e/x/a.jpg",
            ),
            (
                "YouTube",
                "https://i.ytimg.com.evil.example/vi/aqz-KE-bpKQ/a.jpg",
            ),
        ] {
            let json = format!(
                r#"{{"type":"video","provider_name":"{provider}","thumbnail_url":"{thumbnail}"}}"#
            );
            assert_eq!(
                parse_oembed(json.as_bytes(), ID),
                None,
                "{provider} {thumbnail}"
            );
        }
    }

    #[test]
    fn a_title_alone_is_a_card_and_the_webp_path_is_trusted() {
        let card = parse_oembed(
            br#"{"type":"VIDEO","provider_name":"youtube","title":"T","thumbnail_url":"https://img.youtube.com/vi_webp/aqz-KE-bpKQ/a.webp"}"#,
            ID,
        )
        .unwrap();
        assert_eq!(card.title.as_deref(), Some("T"));
        assert!(card.image_url.is_some());
        let card = parse_oembed(
            br#"{"type":"video","provider_name":"YouTube","title":"T","thumbnail_url":"nope"}"#,
            ID,
        )
        .unwrap();
        assert_eq!(card.image_url, None);
    }

    #[test]
    fn what_is_not_the_answer_is_nothing() {
        for json in [
            &b""[..],
            b"[]",
            b"null",
            b"42",
            b"{",
            b"{\"type\":\"video\"}",
            b"{\"type\":1,\"provider_name\":\"YouTube\"}",
            b"{\"type\":\"video\",\"provider_name\":\"YouTube\"}",
            b"{\"type\":\"video\",\"provider_name\":\"YouTube\",\"title\":\"  \"}",
            b"{\"type\":\"photo\",\"provider_name\":\"YouTube\",\"title\":\"T\"}",
            b"\xff\xfe",
        ] {
            assert_eq!(parse_oembed(json, ID), None, "{json:?}");
        }
    }

    #[test]
    fn deeply_nested_json_is_refused_not_overflowed() {
        let deep = "[".repeat(100_000);
        assert_eq!(parse_oembed(deep.as_bytes(), ID), None);
    }

    #[test]
    fn builds_the_address_from_the_id_alone() {
        assert_eq!(
            oembed_address("https://www.youtube.com/oembed", ID),
            "https://www.youtube.com/oembed?url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Daqz-KE-bpKQ&format=json"
        );
    }
}
