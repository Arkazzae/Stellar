//! Reading a `Content-Type` header.

/// What a `Content-Type` header says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentType {
    /// `type/subtype` in lower case.
    pub media_type: String,
    /// The value of the `charset` parameter, quotes removed.
    pub charset: Option<String>,
}

/// Reads a header value. `None` when it is not `type/subtype` followed by
/// parameters, which is how a page that lies about itself in the header
/// ends up with no type at all.
pub fn parse_content_type(value: &str) -> Option<ContentType> {
    let mut parts = split_parameters(value).into_iter();
    let media = parts.next()?.trim().to_ascii_lowercase();
    let (kind, subtype) = media.split_once('/')?;
    if !is_token(kind) || !is_token(subtype) {
        return None;
    }
    let mut charset = None;
    for parameter in parts {
        let Some((name, value)) = parameter.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("charset") && charset.is_none() {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            charset = Some(value.to_owned());
        }
    }
    Some(ContentType {
        media_type: media,
        charset,
    })
}

/// Splits at semicolons that are not inside a quoted string.
fn split_parameters(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (at, c) in value.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                parts.push(&value[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_type_and_the_charset() {
        let parsed = parse_content_type("Text/HTML; Charset=\"ISO-8859-2\"; q=1").unwrap();
        assert_eq!(parsed.media_type, "text/html");
        assert_eq!(parsed.charset.as_deref(), Some("ISO-8859-2"));
        let parsed = parse_content_type("text/html;charset='utf-8'").unwrap();
        assert_eq!(parsed.charset.as_deref(), Some("utf-8"));
        assert_eq!(parse_content_type("image/png").unwrap().charset, None);
    }

    #[test]
    fn a_semicolon_in_quotes_does_not_split() {
        let parsed = parse_content_type("text/html; x=\"a;charset=bad\"; charset=utf-8").unwrap();
        assert_eq!(parsed.charset.as_deref(), Some("utf-8"));
    }

    #[test]
    fn refuses_what_is_not_a_media_type() {
        for value in [
            "",
            "text",
            "text/",
            "/html",
            "te xt/html",
            "text/ht ml",
            ";charset=utf-8",
        ] {
            assert_eq!(parse_content_type(value), None, "{value:?}");
        }
    }
}
