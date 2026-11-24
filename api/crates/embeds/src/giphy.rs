//! The canonical page of a GIPHY GIF, recognised without touching the
//! network.
//!
//! GIPHY requires that its API and media are requested by the viewer's own
//! client, and forbids partners from caching its media. The link therefore
//! stays in the message as written, and the server neither previews nor
//! copies it.

use crate::link::Link;

/// The id of the GIF, when the link is `https://giphy.com/gifs/<slug>`, where
/// the slug is `some-words-<id>` or just `<id>`.
pub fn parse_giphy_link(link: &Link) -> Option<String> {
    if !link.is_https() || link.domain() != "giphy.com" {
        return None;
    }
    let segments: Vec<&str> = link.path().split('/').filter(|s| !s.is_empty()).collect();
    let ["gifs", slug] = segments.as_slice() else {
        return None;
    };
    let candidate = slug.rsplit('-').next().unwrap_or(slug);
    (!candidate.is_empty() && candidate.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| candidate.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(url: &str) -> Option<String> {
        parse_giphy_link(&Link::parse(url).unwrap())
    }

    #[test]
    fn recognises_canonical_gif_pages() {
        assert_eq!(
            id("https://giphy.com/gifs/reaction-happy-AbCd1234").as_deref(),
            Some("AbCd1234")
        );
        assert_eq!(
            id("https://www.giphy.com/gifs/AbCd1234").as_deref(),
            Some("AbCd1234")
        );
    }

    #[test]
    fn refuses_non_canonical_or_untrusted_addresses() {
        for url in [
            "http://giphy.com/gifs/AbCd1234",
            "https://giphy.com/stickers/AbCd1234",
            "https://giphy.com.evil.example/gifs/AbCd1234",
            "https://media.giphy.com/media/AbCd1234/giphy.gif",
            "https://giphy.com/gifs/a/b",
            "https://giphy.com/gifs/slug-",
            "https://giphy.com/gifs/",
        ] {
            assert_eq!(id(url), None, "{url}");
        }
    }
}
