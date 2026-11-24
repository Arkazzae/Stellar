//! A small XML reader for what S3 and compatible stores send, and an escaper
//! for what is sent back.
//!
//! It builds a tree because every answer is small enough, and it refuses what
//! a store never needs to say: a doctype or entity declaration, a character
//! that XML forbids, a document nested without sense or grown past a bound.
//! Namespaces are dropped: only the local part of a name is kept, since the
//! root's namespace differs between servers.

use std::fmt;

/// The largest document read. A page of a thousand keys is about 400 KiB, even
/// with the longest keys S3 allows.
pub const MAX_DOCUMENT: usize = 16 * 1024 * 1024;
const MAX_DEPTH: usize = 32;
const MAX_ELEMENTS: usize = 200_000;
const MAX_NAME: usize = 256;
const MAX_TEXT: usize = 128 * 1024;
const MAX_ATTRIBUTES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XmlError {
    TooLarge,
    TooDeep,
    NotUtf8,
    /// A doctype, entity declaration or other markup this reader refuses.
    Forbidden,
    /// The document stops, or a tag or entity is cut off, before it is whole.
    Truncated,
    Malformed(&'static str),
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("the document is larger than the reader allows"),
            Self::TooDeep => f.write_str("the document is nested too deeply"),
            Self::NotUtf8 => f.write_str("the document is not UTF-8"),
            Self::Forbidden => f.write_str("the document declares a doctype or entities"),
            Self::Truncated => f.write_str("the document ends too early"),
            Self::Malformed(why) => write!(f, "the document is malformed: {why}"),
        }
    }
}

impl std::error::Error for XmlError {}

/// One element: its local name, its own character data, its child elements.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// The text of the first child with this name, as sent.
    pub fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(|child| child.text.as_str())
    }
}

pub fn parse(input: &[u8]) -> Result<Node, XmlError> {
    if input.len() > MAX_DOCUMENT {
        return Err(XmlError::TooLarge);
    }
    let text = std::str::from_utf8(input).map_err(|_| XmlError::NotUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    Reader {
        src: text.as_bytes(),
        text,
        at: 0,
        elements: 0,
    }
    .document()
}

struct Reader<'a> {
    src: &'a [u8],
    text: &'a str,
    at: usize,
    elements: usize,
}

impl<'a> Reader<'a> {
    fn document(mut self) -> Result<Node, XmlError> {
        let mut stack: Vec<(String, Node)> = Vec::new();
        let mut root: Option<Node> = None;
        while self.at < self.src.len() {
            if self.src[self.at] != b'<' {
                let run = self.take_until(b'<');
                match stack.last_mut() {
                    Some((_, node)) => append_text(node, &decode_text(run)?)?,
                    None if run.bytes().all(is_blank) => {}
                    None => return Err(XmlError::Malformed("text outside the root element")),
                }
                continue;
            }
            let rest = &self.text[self.at..];
            if rest.starts_with("<?") {
                self.skip_past("?>")?;
            } else if rest.starts_with("<!--") {
                self.at += 4;
                self.skip_past("-->")?;
            } else if rest.starts_with("<![CDATA[") {
                self.at += 9;
                let start = self.at;
                self.skip_past("]]>")?;
                let data = &self.text[start..self.at - 3];
                match stack.last_mut() {
                    Some((_, node)) => append_text(node, data)?,
                    None => return Err(XmlError::Malformed("data outside the root element")),
                }
            } else if rest.starts_with("<!") {
                return Err(XmlError::Forbidden);
            } else if rest.starts_with("</") {
                self.at += 2;
                let name = self.take_until(b'>').trim_end_matches(is_blank_char);
                if self.at >= self.src.len() {
                    return Err(XmlError::Truncated);
                }
                self.at += 1;
                let (open, node) = stack
                    .pop()
                    .ok_or(XmlError::Malformed("an end tag with no start"))?;
                if open != name {
                    return Err(XmlError::Malformed("an end tag does not match its start"));
                }
                match stack.last_mut() {
                    Some((_, parent)) => parent.children.push(node),
                    None if root.is_none() => root = Some(node),
                    None => return Err(XmlError::Malformed("more than one root element")),
                }
            } else {
                self.at += 1;
                let (qname, empty) = self.start_tag()?;
                if stack.len() >= MAX_DEPTH {
                    return Err(XmlError::TooDeep);
                }
                self.elements += 1;
                if self.elements > MAX_ELEMENTS {
                    return Err(XmlError::TooLarge);
                }
                let node = Node {
                    name: local_name(qname).to_owned(),
                    ..Node::default()
                };
                if empty {
                    match stack.last_mut() {
                        Some((_, parent)) => parent.children.push(node),
                        None if root.is_none() => root = Some(node),
                        None => return Err(XmlError::Malformed("more than one root element")),
                    }
                } else {
                    if stack.is_empty() && root.is_some() {
                        return Err(XmlError::Malformed("more than one root element"));
                    }
                    stack.push((qname.to_owned(), node));
                }
            }
        }
        if !stack.is_empty() {
            return Err(XmlError::Truncated);
        }
        root.ok_or(XmlError::Malformed("no root element"))
    }

    /// Reads a start tag after its `<`; returns the qualified name and whether
    /// the tag closed itself.
    fn start_tag(&mut self) -> Result<(&'a str, bool), XmlError> {
        let start = self.at;
        while self.at < self.src.len()
            && !is_blank(self.src[self.at])
            && !matches!(self.src[self.at], b'/' | b'>')
        {
            self.at += 1;
        }
        let name = &self.text[start..self.at];
        check_name(name)?;
        let mut attributes = 0;
        loop {
            self.skip_blanks();
            match self.src.get(self.at) {
                None => return Err(XmlError::Truncated),
                Some(b'>') => {
                    self.at += 1;
                    return Ok((name, false));
                }
                Some(b'/') => {
                    if self.src.get(self.at + 1) == Some(&b'>') {
                        self.at += 2;
                        return Ok((name, true));
                    }
                    return Err(XmlError::Malformed("a stray slash in a tag"));
                }
                Some(_) => {}
            }
            attributes += 1;
            if attributes > MAX_ATTRIBUTES {
                return Err(XmlError::TooLarge);
            }
            let from = self.at;
            while self.at < self.src.len()
                && !is_blank(self.src[self.at])
                && self.src[self.at] != b'='
            {
                if matches!(self.src[self.at], b'>' | b'/' | b'<') {
                    return Err(XmlError::Malformed("an attribute without a value"));
                }
                self.at += 1;
            }
            check_name(&self.text[from..self.at])?;
            self.skip_blanks();
            if self.src.get(self.at) != Some(&b'=') {
                return Err(if self.at >= self.src.len() {
                    XmlError::Truncated
                } else {
                    XmlError::Malformed("an attribute without a value")
                });
            }
            self.at += 1;
            self.skip_blanks();
            let quote = match self.src.get(self.at) {
                Some(&q @ (b'"' | b'\'')) => q,
                Some(_) => return Err(XmlError::Malformed("an unquoted attribute value")),
                None => return Err(XmlError::Truncated),
            };
            self.at += 1;
            let value = self.take_until(quote);
            if self.at >= self.src.len() {
                return Err(XmlError::Truncated);
            }
            self.at += 1;
            if value.contains('<') {
                return Err(XmlError::Malformed("a < inside an attribute value"));
            }
            decode_text(value)?;
        }
    }

    /// Returns the text up to the next `byte` (or the end) and stops on it.
    fn take_until(&mut self, byte: u8) -> &'a str {
        let start = self.at;
        let end = self.src[start..]
            .iter()
            .position(|&b| b == byte)
            .map_or(self.src.len(), |n| start + n);
        self.at = end;
        &self.text[start..end]
    }

    fn skip_past(&mut self, end: &str) -> Result<(), XmlError> {
        match self.text[self.at..].find(end) {
            Some(n) => {
                self.at += n + end.len();
                Ok(())
            }
            None => Err(XmlError::Truncated),
        }
    }

    fn skip_blanks(&mut self) {
        while self.at < self.src.len() && is_blank(self.src[self.at]) {
            self.at += 1;
        }
    }
}

fn is_blank(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn is_blank_char(ch: char) -> bool {
    ch.is_ascii() && is_blank(ch as u8)
}

fn local_name(qname: &str) -> &str {
    qname.rsplit(':').next().unwrap_or(qname)
}

fn check_name(name: &str) -> Result<(), XmlError> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '-' | '.'))
        && name.chars().all(|c| {
            !c.is_ascii() || c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
        });
    if ok {
        Ok(())
    } else {
        Err(XmlError::Malformed("a bad name"))
    }
}

fn append_text(node: &mut Node, more: &str) -> Result<(), XmlError> {
    if node.text.len() + more.len() > MAX_TEXT {
        return Err(XmlError::TooLarge);
    }
    node.text.push_str(more);
    Ok(())
}

/// Resolves the five named entities and numeric character references, and
/// refuses raw characters XML forbids.
fn decode_text(run: &str) -> Result<String, XmlError> {
    if !run.contains('&') {
        check_chars(run)?;
        return Ok(run.to_owned());
    }
    let mut out = String::with_capacity(run.len());
    let mut rest = run;
    while let Some(amp) = rest.find('&') {
        check_chars(&rest[..amp])?;
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let semi = after
            .find(';')
            .filter(|&n| n <= 10)
            .ok_or(XmlError::Malformed("an unfinished entity"))?;
        let entity = &after[..semi];
        match entity {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => out.push(char_reference(entity)?),
        }
        rest = &after[semi + 1..];
    }
    check_chars(rest)?;
    out.push_str(rest);
    Ok(out)
}

fn char_reference(entity: &str) -> Result<char, XmlError> {
    let bad = XmlError::Malformed("an unknown entity or bad character reference");
    let digits = entity.strip_prefix('#').ok_or(bad.clone())?;
    let (digits, radix) = match digits.strip_prefix('x') {
        Some(hex) => (hex, 16),
        None => (digits, 10),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return Err(bad);
    }
    let code = u32::from_str_radix(digits, radix).map_err(|_| bad.clone())?;
    char::from_u32(code).filter(|&c| is_xml_char(c)).ok_or(bad)
}

fn check_chars(run: &str) -> Result<(), XmlError> {
    if run.chars().all(is_xml_char) {
        Ok(())
    } else {
        Err(XmlError::Malformed("a character XML forbids"))
    }
}

fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}

/// Escapes text for an element's content.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(doc: &str) -> Node {
        parse(doc.as_bytes()).unwrap_or_else(|e| panic!("{doc:?}: {e}"))
    }

    fn err(doc: &str) -> XmlError {
        parse(doc.as_bytes()).expect_err(doc)
    }

    #[test]
    fn reads_a_namespaced_document() {
        let root = ok(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>b</Name><Empty/><Contents><Key>a</Key></Contents></ListBucketResult>\n",
        );
        assert_eq!(root.name, "ListBucketResult");
        assert_eq!(root.child_text("Name"), Some("b"));
        assert_eq!(root.child("Empty").map(|n| n.text.as_str()), Some(""));
        assert_eq!(
            root.child("Contents").and_then(|c| c.child_text("Key")),
            Some("a")
        );
    }

    #[test]
    fn prefixes_are_dropped() {
        assert_eq!(
            ok("<s3:Error xmlns:s3=\"x\"><s3:Code>c</s3:Code></s3:Error>").child_text("Code"),
            Some("c")
        );
    }

    #[test]
    fn entities_and_cdata_are_resolved() {
        let root = ok("<a>x&lt;&gt;&amp;&quot;&apos;&#65;&#x42;<![CDATA[<raw & ]]>y</a>");
        assert_eq!(root.text, "x<>&\"'AB<raw & y");
    }

    #[test]
    fn text_is_kept_as_sent() {
        assert_eq!(
            ok("<a><k> spaced </k></a>").child_text("k"),
            Some(" spaced ")
        );
    }

    #[test]
    fn comments_and_attributes_are_skipped() {
        let root = ok("<!-- c --><a x='1' y=\"2\"><!-- <b/> --><b/></a><!-- tail -->");
        assert_eq!(root.children.len(), 1);
    }

    #[test]
    fn a_doctype_or_entity_declaration_is_refused() {
        assert_eq!(
            err("<!DOCTYPE a [<!ENTITY x \"y\">]><a>&x;</a>"),
            XmlError::Forbidden
        );
        assert_eq!(err("<a><!ENTITY x \"y\"></a>"), XmlError::Forbidden);
    }

    #[test]
    fn undeclared_and_bad_references_are_refused() {
        for doc in [
            "<a>&x;</a>",
            "<a>&amp</a>",
            "<a>&#0;</a>",
            "<a>&#x1;</a>",
            "<a>&#xD800;</a>",
            "<a>&#x110000;</a>",
            "<a>&#xFFFFFFFFFFFF;</a>",
            "<a>&#;</a>",
            "<a>&#x;</a>",
            "<a>&#-1;</a>",
            "<a>&#12a;</a>",
            "<a b=\"&bogus;\"/>",
        ] {
            assert!(matches!(err(doc), XmlError::Malformed(_)), "{doc}");
        }
    }

    #[test]
    fn raw_control_characters_are_refused() {
        assert!(matches!(err("<a>\u{1}</a>"), XmlError::Malformed(_)));
        assert!(matches!(err("<a>\u{0}</a>"), XmlError::Malformed(_)));
    }

    #[test]
    fn every_truncation_is_an_error_and_none_panics() {
        let doc = "<?xml version=\"1.0\"?><ListBucketResult xmlns=\"u\"><Name>b</Name><Contents a='1'><Key>k&amp;</Key><![CDATA[x]]></Contents><!-- c --></ListBucketResult>";
        for cut in 0..doc.len() {
            if doc.is_char_boundary(cut) {
                assert!(parse(&doc.as_bytes()[..cut]).is_err(), "cut at {cut}");
            }
        }
        assert!(parse(doc.as_bytes()).is_ok());
    }

    #[test]
    fn mismatched_and_stray_markup_is_refused() {
        for doc in [
            "<a></b>",
            "<a><b></a></b>",
            "</a>",
            "<a/><b/>",
            "<a></a><b></b>",
            "text",
            "<a>",
            "",
            "   ",
            "<>",
            "<a b></a>",
            "<a b=c></a>",
            "<a b='<'/>",
            "<1a/>",
            "<a/ >",
        ] {
            assert!(parse(doc.as_bytes()).is_err(), "{doc}");
        }
    }

    #[test]
    fn text_after_the_root_is_refused() {
        assert!(matches!(err("<a/>junk"), XmlError::Malformed(_)));
    }

    #[test]
    fn invalid_utf8_is_refused() {
        assert_eq!(parse(b"<a>\xff</a>"), Err(XmlError::NotUtf8));
    }

    #[test]
    fn deep_nesting_is_bounded_and_does_not_recurse() {
        let deep = format!("{}{}", "<a>".repeat(100_000), "</a>".repeat(100_000));
        assert_eq!(err(&deep), XmlError::TooDeep);
        let fine = format!("{}{}", "<a>".repeat(MAX_DEPTH), "</a>".repeat(MAX_DEPTH));
        assert!(parse(fine.as_bytes()).is_ok());
    }

    #[test]
    fn element_count_is_bounded() {
        let doc = format!("<a>{}</a>", "<b/>".repeat(MAX_ELEMENTS + 1));
        assert_eq!(err(&doc), XmlError::TooLarge);
    }

    #[test]
    fn oversized_documents_and_text_are_refused() {
        assert_eq!(
            parse(&vec![b' '; MAX_DOCUMENT + 1]),
            Err(XmlError::TooLarge)
        );
        let doc = format!("<a>{}</a>", "x".repeat(MAX_TEXT + 1));
        assert_eq!(err(&doc), XmlError::TooLarge);
        let many = format!("<a>{}</a>", "&amp;".repeat(MAX_TEXT + 1));
        assert_eq!(err(&many), XmlError::TooLarge);
        let attrs = format!(
            "<a {}/>",
            (0..MAX_ATTRIBUTES + 1)
                .map(|i| format!("a{i}='1'"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(err(&attrs), XmlError::TooLarge);
        let name = format!("<{}/>", "n".repeat(MAX_NAME + 1));
        assert!(matches!(err(&name), XmlError::Malformed(_)));
    }

    #[test]
    fn a_byte_order_mark_is_tolerated() {
        assert!(parse("\u{feff}<a/>".as_bytes()).is_ok());
    }

    #[test]
    fn html_is_not_mistaken_for_xml() {
        assert!(parse(b"<!DOCTYPE html><html><body>502 Bad Gateway</body></html>").is_err());
        assert!(parse(b"<html><body>502<br></body></html>").is_err());
    }

    #[test]
    fn escaping_round_trips() {
        let nasty = "a&b<c>d\"e'f\rg";
        let doc = format!("<a>{}</a>", escape(nasty));
        assert_eq!(ok(&doc).text, nasty);
    }
}
