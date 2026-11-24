//! Links that get a card of their own kind, recognised from the address
//! alone.
//!
//! Nothing here makes a request. The id is in the address and the viewer's
//! own client plays the video, so a player can be offered the moment a
//! message is posted, whether or not this server can reach the host. A title
//! and a thumbnail are fetched afterwards, if the host answers.

use crate::link::Link;

pub const YOUTUBE: &str = "youtube";
pub const VIMEO: &str = "vimeo";

/// The furthest into a video a deep link may point: a day. A start past it
/// is not a deep link, and the bound keeps a hostile value from reaching
/// clients as anything to reason about.
pub const MAX_START_SECONDS: i64 = 24 * 60 * 60;

/// A link a player can be put behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoLink {
    pub provider: &'static str,
    pub id: String,
    pub start_seconds: Option<u32>,
}

/// The video behind a link, when it is one of the supported hosts.
pub fn parse_video_link(link: &Link) -> Option<VideoLink> {
    let segments: Vec<&str> = link.path().split('/').filter(|s| !s.is_empty()).collect();
    let (provider, id) = match host_without_prefix(link.host()) {
        "youtube.com" | "youtube-nocookie.com" => (YOUTUBE, youtube_id(link, &segments)),
        "youtu.be" => (YOUTUBE, segments.first().map(|s| (*s).to_owned())),
        "vimeo.com" | "player.vimeo.com" => (VIMEO, vimeo_id(&segments)),
        _ => return None,
    };
    let id = id?;
    if !is_valid_id(provider, &id) {
        return None;
    }
    Some(VideoLink {
        provider,
        id,
        start_seconds: start_seconds(link),
    })
}

/// The host without the `www.` or `m.` that only change the layout.
fn host_without_prefix(host: &str) -> &str {
    host.strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .unwrap_or(host)
}

fn youtube_id(link: &Link, segments: &[&str]) -> Option<String> {
    // /watch?v=ID is the canonical form. The path forms are what a phone's
    // share sheet pastes.
    if let ["embed" | "shorts" | "live" | "v", id, ..] = segments {
        return Some((*id).to_owned());
    }
    query_value(link.query(), "v")
}

fn vimeo_id(segments: &[&str]) -> Option<String> {
    // player.vimeo.com/video/ID, besides vimeo.com/ID.
    match segments {
        ["video", id, ..] | [id, ..] => Some((*id).to_owned()),
        [] => None,
    }
}

fn is_valid_id(provider: &str, id: &str) -> bool {
    if provider == YOUTUBE {
        id.len() == 11
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    } else {
        (6..=12).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
    }
}

fn start_seconds(link: &Link) -> Option<u32> {
    let fragment_time = link.fragment().and_then(|fragment| {
        let fragment = fragment.trim_start_matches('#');
        (fragment.len() >= 2
            && fragment.is_char_boundary(2)
            && fragment[..2].eq_ignore_ascii_case("t="))
        .then(|| fragment[2..].to_owned())
    });
    // A key that is present with an empty value is an answer: the next
    // source is not asked.
    let raw = query_value(link.query(), "t")
        .or_else(|| query_value(link.query(), "start"))
        .or(fragment_time)?;
    if raw.is_empty() {
        return None;
    }
    let total = parse_timestamp(&raw)?;
    (total > 0 && total <= MAX_START_SECONDS).then_some(total as u32)
}

/// Hours, minutes and seconds in any combination: `1h2m3s`, `90s` or `90`.
/// A count too large to mean anything counts as nothing, as does a total
/// that does not fit.
fn parse_timestamp(raw: &str) -> Option<i64> {
    // A single line break at the end is let through.
    let raw = raw.strip_suffix('\n').unwrap_or(raw);
    let bytes = raw.as_bytes();
    let mut at = 0;
    let mut stage = 0;
    let (mut hours, mut minutes, mut seconds) = (0i64, 0i64, 0i64);
    while at < bytes.len() {
        let digits_start = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        if at == digits_start {
            return None;
        }
        let count = raw[digits_start..at].parse::<i32>().map_or(0, i64::from);
        match bytes.get(at).map(u8::to_ascii_lowercase) {
            Some(b'h') if stage < 1 => {
                hours = count;
                stage = 1;
                at += 1;
            }
            Some(b'm') if stage < 2 => {
                minutes = count;
                stage = 2;
                at += 1;
            }
            Some(b's') => {
                seconds = count;
                at += 1;
                break;
            }
            None => {
                seconds = count;
                break;
            }
            Some(_) => return None,
        }
    }
    (at == bytes.len()).then(|| hours * 3600 + minutes * 60 + seconds)
}

/// The value of a query key, the way a form decoder reads it: keys without
/// regard to case, `+` for a space, `%XX` as bytes of UTF-8, and a key given
/// twice reads as its values joined by commas.
pub(crate) fn query_value(query: Option<&str>, key: &str) -> Option<String> {
    let query = query?;
    let mut found: Option<Vec<String>> = None;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if !pair.contains('=') {
            // A bare word is a key with no name, which nobody asks for.
            continue;
        }
        if percent_decode(name).eq_ignore_ascii_case(key) {
            found
                .get_or_insert_with(Vec::new)
                .push(percent_decode(value));
        }
    }
    found.map(|values| values.join(","))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' if at + 2 < bytes.len()
                && hex(bytes[at + 1]).is_some()
                && hex(bytes[at + 2]).is_some() =>
            {
                out.push(hex(bytes[at + 1]).unwrap() * 16 + hex(bytes[at + 2]).unwrap());
                at += 2;
            }
            other => out.push(other),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|d| d as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(url: &str) -> Option<VideoLink> {
        parse_video_link(&Link::parse(url).unwrap())
    }

    #[test]
    fn recognises_youtube() {
        for (url, id) in [
            ("https://www.youtube.com/watch?v=aqz-KE-bpKQ", "aqz-KE-bpKQ"),
            ("https://youtu.be/jNQXAC9IVRw", "jNQXAC9IVRw"),
            (
                "https://m.youtube.com/watch?v=aqz-KE-bpKQ&feature=share",
                "aqz-KE-bpKQ",
            ),
            ("https://www.youtube.com/shorts/aqz-KE-bpKQ", "aqz-KE-bpKQ"),
            (
                "https://www.youtube-nocookie.com/embed/aqz-KE-bpKQ",
                "aqz-KE-bpKQ",
            ),
            (
                "https://www.youtube.com/live/aqz-KE-bpKQ?si=x",
                "aqz-KE-bpKQ",
            ),
            ("https://WWW.YouTube.com/watch?V=aqz-KE-bpKQ", "aqz-KE-bpKQ"),
        ] {
            let link = video(url).unwrap_or_else(|| panic!("{url}"));
            assert_eq!(link.provider, YOUTUBE);
            assert_eq!(link.id, id);
        }
    }

    #[test]
    fn recognises_vimeo() {
        for url in [
            "https://vimeo.com/123456789",
            "https://player.vimeo.com/video/123456789",
        ] {
            let link = video(url).unwrap();
            assert_eq!(link.provider, VIMEO);
            assert_eq!(link.id, "123456789");
        }
    }

    #[test]
    fn reads_the_start_timestamp() {
        for (url, expected) in [
            ("https://www.youtube.com/watch?v=eRsGyueVLvQ&t=95s", 95),
            ("https://www.youtube.com/watch?v=eRsGyueVLvQ&t=1h2m3s", 3723),
            ("https://youtu.be/eRsGyueVLvQ#t=30", 30),
            ("https://youtu.be/eRsGyueVLvQ?start=45", 45),
            ("https://youtu.be/eRsGyueVLvQ?t=2m", 120),
            ("https://youtu.be/eRsGyueVLvQ?t=1H1M1S", 3661),
            ("https://youtu.be/eRsGyueVLvQ#T=7", 7),
        ] {
            assert_eq!(video(url).unwrap().start_seconds, Some(expected), "{url}");
        }
    }

    #[test]
    fn refuses_anything_else() {
        for url in [
            "https://youtube.com.evil.example/watch?v=aqz-KE-bpKQ",
            "https://www.youtube.com/watch?v=../../etc/passwd",
            "https://www.youtube.com/watch?v=short",
            "https://example.org/watch?v=aqz-KE-bpKQ",
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ&v=aqz-KE-bpKQ",
            "https://vimeo.com/12345",
            "https://vimeo.com/1234567890123",
            "https://vimeo.com/",
            "https://www.youtube.com/embed/",
            "https://notyoutu.be/aqz-KE-bpKQ",
        ] {
            assert!(video(url).is_none(), "{url}");
        }
    }

    #[test]
    fn ignores_a_start_time_that_is_not_a_reasonable_one() {
        for url in [
            "https://www.youtube.com/watch?v=eRsGyueVLvQ&t=99999999999999999999s",
            "https://www.youtube.com/watch?v=eRsGyueVLvQ&t=25h",
            "https://www.youtube.com/watch?v=eRsGyueVLvQ&t=soon",
            "https://www.youtube.com/watch?v=eRsGyueVLvQ&t=0",
            "https://www.youtube.com/watch?v=eRsGyueVLvQ&t=",
            "https://www.youtube.com/watch?v=eRsGyueVLvQ&t=1m1h",
            "https://www.youtube.com/watch?v=eRsGyueVLvQ&t=1s2",
            "https://www.youtube.com/watch?v=eRsGyueVLvQ&t=9999999h",
            "https://www.youtube.com/watch?v=eRsGyueVLvQ&t=1&t=2",
        ] {
            let link = video(url).unwrap_or_else(|| panic!("{url}"));
            assert_eq!(link.start_seconds, None, "{url}");
        }
    }

    #[test]
    fn an_empty_t_is_an_answer_and_start_is_not_asked() {
        assert_eq!(
            video("https://youtu.be/eRsGyueVLvQ?t=&start=30")
                .unwrap()
                .start_seconds,
            None
        );
    }

    #[test]
    fn query_values_are_decoded_and_joined() {
        assert_eq!(query_value(Some("a=1&A=2"), "a").as_deref(), Some("1,2"));
        assert_eq!(query_value(Some("a=%C5%BC+x"), "a").as_deref(), Some("ż x"));
        assert_eq!(query_value(Some("a=%zz%"), "a").as_deref(), Some("%zz%"));
        assert_eq!(query_value(Some("flag&b=1"), "flag"), None);
        assert_eq!(query_value(None, "a"), None);
    }
}
