//! The browser and system of a user-agent string, named for a person reading
//! a security alert. The same patterns as the web client's own description of
//! a device, so the mail names a device as the account's activity list does.

use stellarr_messages::{MessageKey, Messages, MissingValue};

/// A word character for the purposes of a word boundary.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `needle` occurs in `text[from..to]` with a word boundary before
/// it, and after it when `closed` is set (the needle then ends in a word
/// character). The characters around the range count for the boundaries.
fn has_word_in(text: &str, from: usize, to: usize, needle: &str, closed: bool) -> bool {
    text[from..to].match_indices(needle).any(|(offset, _)| {
        let at = from + offset;
        let before = text[..at].chars().next_back().is_none_or(|c| !is_word(c));
        let after = text[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_word(c));
        before && (!closed || after)
    })
}

fn has_word(text: &str, needle: &str, closed: bool) -> bool {
    has_word_in(text, 0, text.len(), needle, closed)
}

fn is_edge(agent: &str) -> bool {
    ["Edg/", "Edge/", "EdgA/", "EdgiOS/"]
        .iter()
        .any(|needle| has_word(agent, needle, false))
}

/// `\bVersion/[\d.]+.*\bSafari/`: a version, then somewhere later on the
/// same line a Safari token.
fn is_safari(agent: &str) -> bool {
    agent.match_indices("Version/").any(|(at, needle)| {
        let before = agent[..at].chars().next_back().is_none_or(|c| !is_word(c));
        let digits_from = at + needle.len();
        let digits = agent[digits_from..]
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b'.')
            .count();
        if !before || digits == 0 {
            return false;
        }
        let from = digits_from + digits;
        let to = agent[from..].find('\n').map_or(agent.len(), |n| from + n);
        has_word_in(agent, from, to, "Safari/", false)
    })
}

fn browser(agent: &str) -> Option<&'static str> {
    if is_edge(agent) {
        Some("Edge")
    } else if has_word(agent, "OPR/", false) || has_word(agent, "Opera", true) {
        Some("Opera")
    } else if has_word(agent, "Firefox/", false) || has_word(agent, "FxiOS/", false) {
        Some("Firefox")
    } else if has_word(agent, "Chrome/", false) || has_word(agent, "CriOS/", false) {
        Some("Chrome")
    } else if is_safari(agent) {
        Some("Safari")
    } else {
        None
    }
}

fn system(agent: &str) -> Option<&'static str> {
    if ["iPhone", "iPad", "iPod"]
        .iter()
        .any(|needle| has_word(agent, needle, true))
    {
        Some("iOS")
    } else if has_word(agent, "Android", true) {
        Some("Android")
    } else if has_word(agent, "Windows", true) {
        Some("Windows")
    } else if has_word(agent, "Mac OS X", true) || has_word(agent, "Macintosh", true) {
        Some("macOS")
    } else if has_word(agent, "CrOS", true) {
        Some("ChromeOS")
    } else if has_word(agent, "Linux", true) {
        Some("Linux")
    } else {
        None
    }
}

/// "Firefox on Linux", one of the two, or nothing when neither is recognised.
pub fn describe_device(
    user_agent: &str,
    messages: &Messages,
) -> Result<Option<String>, MissingValue> {
    match (browser(user_agent), system(user_agent)) {
        (Some(browser), Some(system)) => messages
            .get(
                MessageKey::SecurityAlertDeviceOn,
                &[("browser", &browser), ("system", &system)],
            )
            .map(Some),
        (browser, system) => Ok(browser.or(system).map(str::to_owned)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellarr_messages::Language;

    #[test]
    fn a_device_is_named_by_its_browser_and_system() {
        let english = Messages::new(Language::English);
        for (agent, expected) in [
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0",
                Some("Firefox on Linux"),
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0",
                Some("Edge on Windows"),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
                Some("Safari on iOS"),
            ),
            (
                "Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Mobile Safari/537.36",
                Some("Chrome on Android"),
            ),
            ("curl/8.0", None),
            ("", None),
        ] {
            assert_eq!(
                describe_device(agent, &english).unwrap().as_deref(),
                expected,
                "{agent}"
            );
        }
    }

    #[test]
    fn only_one_of_the_two_is_named_when_the_other_is_not_known() {
        let english = Messages::new(Language::English);

        let system_only = describe_device("Mozilla/5.0 (X11; Linux x86_64)", &english).unwrap();
        let browser_only = describe_device("Firefox/146.0", &english).unwrap();

        assert_eq!(system_only.as_deref(), Some("Linux"));
        assert_eq!(browser_only.as_deref(), Some("Firefox"));
    }

    #[test]
    fn a_name_inside_a_longer_word_is_not_a_match() {
        let english = Messages::new(Language::English);

        for agent in [
            "XFirefox/1",
            "MyLinuxBox",
            "Androidish",
            "WindowsX",
            "NotChrome/1",
            "OperaX",
            "FooEdge/1",
        ] {
            assert_eq!(describe_device(agent, &english).unwrap(), None, "{agent}");
        }
        // Opera needs the boundary after, a slash-token before-only.
        assert_eq!(
            describe_device("Opera Mini", &english).unwrap().as_deref(),
            Some("Opera")
        );
        assert_eq!(
            describe_device("OPR/90", &english).unwrap().as_deref(),
            Some("Opera")
        );
    }

    #[test]
    fn a_version_and_a_safari_token_must_share_a_line() {
        let english = Messages::new(Language::English);

        assert_eq!(
            describe_device("Version/18.1\nSafari/604", &english).unwrap(),
            None
        );
        assert_eq!(
            describe_device("Version/ Safari/604", &english).unwrap(),
            None
        );
        assert_eq!(
            describe_device("Version/18 x Safari/604", &english)
                .unwrap()
                .as_deref(),
            Some("Safari")
        );
        assert_eq!(
            describe_device("Safari/604 Version/18", &english).unwrap(),
            None
        );
    }

    #[test]
    fn the_polish_connector_is_used() {
        let polish = Messages::new(Language::Polish);

        let device = describe_device("Firefox/1 (X11; Linux)", &polish).unwrap();

        assert_eq!(device.as_deref(), Some("Firefox na Linux"));
    }
}
