//! The message: what the application sends, and its wire form.
//!
//! A message has a text part, an HTML part, and images that the HTML shows
//! through `cid:`. The wire form follows the usual layout of such mail:
//!
//! * text only: `text/plain`;
//! * text and HTML: `multipart/alternative` of the two;
//! * HTML with images: the HTML and the images in a `multipart/related`, which
//!   takes the place of the HTML part.
//!
//! Everything is 7-bit ASCII on the wire, so any server takes it, except an
//! address with non-ASCII letters before the `@`, which needs SMTPUTF8.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::address::Mailbox;
use crate::encode::{
    MAX_LINE, base64_wrapped, check_header_text, encoded_words, header, quoted_printable,
};
use crate::error::Error;

/// An image that travels inside the message and that the HTML shows through
/// `cid:` followed by `content_id`.
#[derive(Debug, Clone)]
pub struct InlineImage {
    pub content_id: String,
    pub file_name: String,
    pub content_type: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub from: Mailbox,
    /// The recipients; at least one.
    pub to: Vec<Mailbox>,
    pub subject: String,
    /// Empty means the message has no HTML part.
    pub html: String,
    /// Empty means the message has no text part.
    pub text: String,
    /// The Message-ID without the angle brackets, such as `key@host`.
    pub message_id: String,
    pub date: SystemTime,
    pub images: Vec<InlineImage>,
}

/// A message ready to send.
#[derive(Debug, Clone)]
pub struct Encoded {
    /// The whole message with CRLF line breaks and no dot-stuffing.
    pub bytes: Vec<u8>,
    /// True when a header holds non-ASCII text and the server must offer
    /// SMTPUTF8.
    pub needs_utf8: bool,
}

impl Message {
    pub fn encode(&self) -> Result<Encoded, Error> {
        if self.to.is_empty() {
            return Err(Error::invalid("the message has no recipient"));
        }
        check_header_text("the subject", &self.subject)?;
        check_id("the Message-ID", &self.message_id)?;

        let mut head = String::new();
        head.push_str(&header("Date", &rfc5322_date(self.date))?);
        head.push_str(&header("From", &self.from.header_value())?);
        let recipients: Vec<String> = self.to.iter().map(Mailbox::header_value).collect();
        let joined = if recipients.iter().any(|r| r.contains("\r\n")) {
            recipients.join(",\r\n ")
        } else {
            recipients.join(", ")
        };
        head.push_str(&header("To", &joined)?);
        let subject = if self.subject.is_ascii() {
            self.subject.clone()
        } else {
            encoded_words(&self.subject)
        };
        head.push_str(&header("Subject", &subject)?);
        head.push_str(&header("Message-ID", &format!("<{}>", self.message_id))?);
        head.push_str("MIME-Version: 1.0\r\n");

        let (entity_headers, body) = self.entity()?;
        let mut bytes = Vec::with_capacity(head.len() + entity_headers.len() + body.len() + 8);
        bytes.extend_from_slice(head.as_bytes());
        bytes.extend_from_slice(entity_headers.as_bytes());
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(&body);
        // A message ends with a line break. A single part that has its own
        // keeps it as its last; one without gains one, which is all a
        // message can do (inside a multipart, the delimiter takes that role
        // and every part comes back exactly).
        if !bytes.ends_with(b"\r\n") {
            bytes.extend_from_slice(b"\r\n");
        }

        let needs_utf8 = !head.is_ascii();
        Ok(Encoded { bytes, needs_utf8 })
    }

    /// The headers and body of the top-level entity; the body has no break at
    /// its end.
    fn entity(&self) -> Result<(String, Vec<u8>), Error> {
        let has_text = !self.text.is_empty();
        let has_html = !self.html.is_empty();
        if !self.images.is_empty() && !has_html {
            return Err(Error::invalid(
                "inline images need an HTML part to show them",
            ));
        }
        if !has_html {
            return text_entity("plain", &self.text);
        }
        let html = if self.images.is_empty() {
            text_entity("html", &self.html)?
        } else {
            let mut parts = vec![text_entity("html", &self.html)?];
            for image in &self.images {
                parts.push(image_entity(image)?);
            }
            multipart("related", "rel", parts)?
        };
        if !has_text {
            return Ok(html);
        }
        multipart(
            "alternative",
            "alt",
            vec![text_entity("plain", &self.text)?, html],
        )
    }
}

/// `text/<subtype>` in UTF-8: as it is when it is plain ASCII with short
/// lines, quoted-printable otherwise.
fn text_entity(subtype: &str, text: &str) -> Result<(String, Vec<u8>), Error> {
    let normal = normalise_breaks(text);
    let plain = normal.is_ascii()
        && !normal
            .bytes()
            .any(|b| (b < 0x20 && b != b'\t' && b != b'\r' && b != b'\n') || b == 0x7f)
        && normal
            .split("\r\n")
            .all(|line| line.len() <= 76 && !line.ends_with([' ', '\t']));
    let (encoding, body) = if plain {
        ("7bit", normal.into_bytes())
    } else {
        ("quoted-printable", quoted_printable(normal.as_bytes()))
    };
    let headers = format!(
        "Content-Type: text/{subtype}; charset=utf-8\r\nContent-Transfer-Encoding: {encoding}\r\n"
    );
    Ok((headers, body))
}

fn image_entity(image: &InlineImage) -> Result<(String, Vec<u8>), Error> {
    check_id("a Content-Id", &image.content_id)?;
    let (kind, subtype) = image
        .content_type
        .split_once('/')
        .filter(|(k, s)| is_token(k) && is_token(s))
        .ok_or_else(|| Error::invalid(format!("{:?} is not a content type", image.content_type)))?;
    check_header_text("an image file name", &image.file_name)?;
    let name = parameter("name", &image.file_name);
    let filename = parameter("filename", &image.file_name);
    let mut headers = header("Content-Type", &format!("{kind}/{subtype}; {name}"))?;
    headers.push_str("Content-Transfer-Encoding: base64\r\n");
    headers.push_str(&header("Content-Id", &format!("<{}>", image.content_id))?);
    headers.push_str(&header(
        "Content-Disposition",
        &format!("inline; {filename}"),
    )?);
    Ok((headers, base64_wrapped(&image.content)))
}

/// A multipart entity, with a boundary that occurs in none of its parts.
fn multipart(
    subtype: &str,
    tag: &str,
    parts: Vec<(String, Vec<u8>)>,
) -> Result<(String, Vec<u8>), Error> {
    let boundary = (0u32..)
        .map(|n| format!("=_stellarr_{tag}_{n:x}"))
        .find(|candidate| {
            let marker = format!("--{candidate}");
            parts.iter().all(|(h, b)| {
                !contains(h.as_bytes(), marker.as_bytes()) && !contains(b, marker.as_bytes())
            })
        })
        .ok_or_else(|| Error::invalid("no boundary found"))?;
    let mut body = Vec::new();
    for (headers, part) in &parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(headers.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(part);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--").as_bytes());
    // A related part names the type of the part that starts it (RFC 2387).
    let root = if subtype == "related" {
        "; type=\"text/html\""
    } else {
        ""
    };
    let headers =
        format!("Content-Type: multipart/{subtype}{root};\r\n boundary=\"{boundary}\"\r\n");
    Ok((headers, body))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Every line break becomes CRLF, whatever it was.
fn normalise_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 40);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\r\n");
            }
            '\n' => out.push_str("\r\n"),
            c => out.push(c),
        }
    }
    out
}

fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`{|}~".contains(c))
}

/// A `Message-ID` or `Content-Id` body: printable ASCII with one `@`, and
/// nothing that would close the brackets around it.
fn check_id(what: &str, id: &str) -> Result<(), Error> {
    let ok = id.len() <= MAX_LINE / 2
        && id
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"<>()[]\\\",;:".contains(&b))
        && id.split('@').count() == 2
        && !id.starts_with('@')
        && !id.ends_with('@');
    if ok {
        Ok(())
    } else {
        Err(Error::invalid(format!(
            "{what} {id:?} is not of the form local@domain in plain ASCII"
        )))
    }
}

/// A parameter of a header: `name="value"`, or the RFC 2231 form with the
/// value percent-encoded when it holds more than plain ASCII.
fn parameter(name: &str, value: &str) -> String {
    let plain =
        value.bytes().all(|b| b.is_ascii_graphic() || b == b' ') && !value.contains(['"', '\\']);
    if plain {
        return format!("{name}=\"{value}\"");
    }
    let mut out = format!("{name}*=utf-8''");
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `Tue, 24 Nov 2026 10:00:00 +0000`.
pub(crate) fn rfc5322_date(time: SystemTime) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX / 2),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX / 2),
    };
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    // Civil date from a day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{}, {day:02} {} {year:04} {:02}:{:02}:{:02} +0000",
        DAYS[(days + 4).rem_euclid(7) as usize],
        MONTHS[(month - 1) as usize],
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn dates_are_written_in_the_rfc_form() {
        // Checked against Python's email.utils.format_datetime.
        assert_eq!(rfc5322_date(at(0)), "Thu, 01 Jan 1970 00:00:00 +0000");
        assert_eq!(
            rfc5322_date(at(951_782_400)),
            "Tue, 29 Feb 2000 00:00:00 +0000"
        );
        assert_eq!(
            rfc5322_date(at(1_795_000_000)),
            "Wed, 18 Nov 2026 11:06:40 +0000"
        );
        assert_eq!(
            rfc5322_date(at(4_102_444_799)),
            "Thu, 31 Dec 2099 23:59:59 +0000"
        );
    }

    fn sample() -> Message {
        Message {
            from: Mailbox::parse("Stellarr <noreply@chat.test>").unwrap(),
            to: vec![Mailbox::parse("jo@example.test").unwrap()],
            subject: "Zmień hasło w Orbit & Co".into(),
            html: "<p>Treść</p>".into(),
            text: "Treść".into(),
            message_id: "password-reset/ab12@stellarr".into(),
            date: at(1_795_000_000),
            images: vec![],
        }
    }

    fn wire(message: &Message) -> String {
        String::from_utf8(message.encode().unwrap().bytes).unwrap()
    }

    #[test]
    fn a_message_has_the_headers_and_is_ascii_on_the_wire() {
        let text = wire(&sample());
        assert!(text.is_ascii());
        assert!(text.starts_with(
            "Date: Wed, 18 Nov 2026 11:06:40 +0000\r\nFrom: Stellarr <noreply@chat.test>\r\n"
        ));
        assert!(text.contains("\r\nTo: jo@example.test\r\n"));
        assert!(text.contains("\r\nSubject: =?utf-8?b?"));
        assert!(text.contains("\r\nMessage-ID: <password-reset/ab12@stellarr>\r\n"));
        assert!(text.contains("\r\nMIME-Version: 1.0\r\n"));
        assert!(text.contains("multipart/alternative"));
        assert!(!text.contains("multipart/related"));
        assert!(text.ends_with("--\r\n"));
        assert!(
            !text.replace("\r\n", "").contains(['\r', '\n']),
            "only CRLF breaks"
        );
    }

    #[test]
    fn a_single_part_ends_with_exactly_one_line_break() {
        let mut m = sample();
        m.html.clear();
        m.text = "abc\n".into();
        assert!(wire(&m).ends_with("\r\n\r\nabc\r\n"));
        m.text = "abc".into();
        assert!(wire(&m).ends_with("\r\n\r\nabc\r\n"));
        m.text = "abc \n".into();
        assert!(wire(&m).ends_with("quoted-printable\r\n\r\nabc=20\r\n"));
    }

    #[test]
    fn every_id_the_application_makes_is_accepted() {
        let guid = "0123456789abcdef0123456789abcdef";
        for kind in [
            "password-reset",
            "email-change",
            "delivery-test",
            "email-code",
            "security-event",
            "activity-notification",
        ] {
            let mut m = sample();
            m.message_id = format!("{kind}/{guid}@stellarr");
            let text = wire(&m);
            assert!(
                text.contains(&format!("\r\nMessage-ID: <{kind}/{guid}@stellarr>\r\n")),
                "{kind}"
            );
        }
    }

    #[test]
    fn related_names_the_type_of_its_root() {
        let mut m = sample();
        m.images = vec![InlineImage {
            content_id: "a@b".into(),
            file_name: "a.png".into(),
            content_type: "image/png".into(),
            content: vec![1],
        }];
        assert!(
            wire(&m).contains("Content-Type: multipart/related; type=\"text/html\";\r\n boundary=")
        );
    }

    #[test]
    fn a_text_only_message_is_one_part() {
        let mut m = sample();
        m.html.clear();
        let text = wire(&m);
        assert!(text.contains("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nTre=C5=9B=C4=87\r\n"));
        assert!(!text.contains("multipart"));
    }

    #[test]
    fn plain_ascii_text_stays_unencoded() {
        let mut m = sample();
        m.html.clear();
        m.text = "Hello\nthere".into();
        assert!(wire(&m).ends_with("Content-Transfer-Encoding: 7bit\r\n\r\nHello\r\nthere\r\n"));
    }

    #[test]
    fn images_go_into_a_related_part_with_the_html() {
        let mut m = sample();
        m.images = vec![InlineImage {
            content_id: "lockup@stellarr".into(),
            file_name: "lockup.png".into(),
            content_type: "image/png".into(),
            content: (0..=255).collect(),
        }];
        let text = wire(&m);
        let alt = text.find("multipart/alternative").unwrap();
        let rel = text.find("multipart/related").unwrap();
        assert!(alt < rel);
        assert!(text.contains("Content-Id: <lockup@stellarr>\r\n"));
        assert!(text.contains("Content-Disposition: inline; filename=\"lockup.png\"\r\n"));
        assert!(text.contains("Content-Transfer-Encoding: base64\r\n"));
        assert!(text.lines().all(|l| l.len() <= 78));
    }

    #[test]
    fn a_boundary_never_occurs_inside_a_part() {
        let mut m = sample();
        m.html = "<p>x</p>\n--=_stellarr_alt_0\n--=_stellarr_alt_1 more".into();
        m.text = "--=_stellarr_alt_0".into();
        let text = wire(&m);
        let line = text.lines().find(|l| l.contains("boundary=")).unwrap();
        assert!(line.contains("_alt_2"), "{line}");
    }

    #[test]
    fn long_html_lines_are_wrapped_and_the_text_survives() {
        let mut m = sample();
        m.html = format!("<p>{}</p>", "zażółć ".repeat(500));
        let text = wire(&m);
        assert!(text.lines().all(|l| l.len() <= 78), "a line is too long");
    }

    #[test]
    fn unsafe_input_is_refused() {
        let mut m = sample();
        m.subject = "x\r\nBcc: evil@example.test".into();
        assert!(m.encode().is_err());
        let mut m = sample();
        m.message_id = "a b@stellarr".into();
        assert!(m.encode().is_err());
        let mut m = sample();
        m.to.clear();
        assert!(m.encode().is_err());
        let mut m = sample();
        m.html.clear();
        m.images = vec![InlineImage {
            content_id: "a@b".into(),
            file_name: "a.png".into(),
            content_type: "image/png".into(),
            content: vec![1],
        }];
        assert!(m.encode().is_err());
        let mut m = sample();
        m.images = vec![InlineImage {
            content_id: "a@b".into(),
            file_name: "a.png".into(),
            content_type: "image/png\r\nX: y".into(),
            content: vec![1],
        }];
        assert!(m.encode().is_err());
    }

    #[test]
    fn a_non_ascii_file_name_uses_the_rfc_2231_form() {
        assert_eq!(
            parameter("filename", "zażółć.png"),
            "filename*=utf-8''za%C5%BC%C3%B3%C5%82%C4%87.png"
        );
        assert_eq!(parameter("filename", "a b.png"), "filename=\"a b.png\"");
        assert_eq!(
            parameter("filename", "a\"b.png"),
            "filename*=utf-8''a%22b.png"
        );
    }

    #[test]
    fn a_non_ascii_local_part_marks_the_message_as_utf8() {
        let mut m = sample();
        assert!(!m.encode().unwrap().needs_utf8);
        m.to = vec![Mailbox::parse("zażółć@example.test").unwrap()];
        assert!(m.encode().unwrap().needs_utf8);
    }
}
