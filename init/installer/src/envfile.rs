//! `.env` as Compose reads it, and the installer's changes to it. The file is
//! handled as bytes: a line the installer does not change is written back
//! exactly as it was, whatever its encoding or line end.

use std::collections::BTreeMap;
use std::ops::Range;

pub const BOM: &[u8] = b"\xef\xbb\xbf";

fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

fn skip_blank(line: &[u8], mut at: usize) -> usize {
    while at < line.len() && is_blank(line[at]) {
        at += 1;
    }
    at
}

fn strip_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// The lines of a file: a last line without a line end counts, an empty end
/// after the last line end does not.
pub fn records(content: &[u8]) -> Vec<&[u8]> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&[u8]> = content.split(|&b| b == b'\n').collect();
    if content.ends_with(b"\n") {
        lines.pop();
    }
    lines
}

fn name_then_equals(line: &[u8], start: usize) -> Option<(String, usize)> {
    let first = *line.get(start)?;
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut end = start + 1;
    while end < line.len() && (line[end].is_ascii_alphanumeric() || line[end] == b'_') {
        end += 1;
    }
    let equals = skip_blank(line, end);
    if matches!(line.get(equals), Some(b'=' | b':')) {
        Some((
            String::from_utf8_lossy(&line[start..end]).into_owned(),
            equals + 1,
        ))
    } else {
        None
    }
}

/// The name a line assigns, with blanks around it and an optional `export`
/// in front, and where its value starts.
pub fn assignment(line: &[u8]) -> Option<(String, usize)> {
    let line = strip_cr(line);
    let start = skip_blank(line, 0);
    if line[start..].starts_with(b"export") {
        let after = start + 6;
        let name = skip_blank(line, after);
        if name > after
            && let Some(found) = name_then_equals(line, name)
        {
            return Some(found);
        }
    }
    name_then_equals(line, start)
}

/// The value of an assignment, unquoted: a quoted one ends at its closing
/// quote, an unquoted one at a comment.
pub fn value(line: &[u8], at: usize) -> String {
    let line = strip_cr(line);
    let mut value = &line[skip_blank(line, at)..];
    match value.first() {
        Some(&quote @ (b'"' | b'\'')) => {
            value = &value[1..];
            if let Some(end) = quote_end(value, quote) {
                value = &value[..end];
            }
        }
        _ => {
            let mut i = 0;
            while i < value.len() {
                if is_blank(value[i]) {
                    let next = skip_blank(value, i);
                    if value.get(next) == Some(&b'#') {
                        value = &value[..i];
                        break;
                    }
                    i = next;
                } else {
                    i += 1;
                }
            }
            while value.last().is_some_and(|&b| is_blank(b)) {
                value = &value[..value.len() - 1];
            }
        }
    }
    String::from_utf8_lossy(value).into_owned()
}

fn quote_end(value: &[u8], quote: u8) -> Option<usize> {
    let mut escaped = false;
    for (index, &byte) in value.iter().enumerate() {
        if byte == quote && !escaped {
            return Some(index);
        }
        escaped = byte == b'\\' && !escaped;
    }
    None
}

/// Keep an assignment and its quoted continuations together when reading or
/// rewriting. A key-looking line inside a value is part of that value.
fn assignments(content: &[u8]) -> impl Iterator<Item = (Range<usize>, String, usize)> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        while start < content.len() {
            let row = start;
            let end = content[row..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(content.len(), |n| row + n + 1);
            start = end;
            let line = content[row..end]
                .strip_suffix(b"\n")
                .unwrap_or(&content[row..end]);
            let Some((key, at)) = assignment(line) else {
                continue;
            };
            let at = skip_blank(line, at);
            if let Some(&quote @ (b'\'' | b'"')) = line.get(at) {
                start = quote_end(&content[row + at + 1..], quote)
                    .map(|n| row + at + n + 2)
                    .map_or(content.len(), |closing| {
                        content[closing..]
                            .iter()
                            .position(|&b| b == b'\n')
                            .map_or(content.len(), |n| closing + n + 1)
                    });
            }
            return Some((row..start, key, at));
        }
        None
    })
}

pub fn parse(content: &[u8]) -> BTreeMap<String, String> {
    let content = content.strip_prefix(BOM).unwrap_or(content);
    assignments(content)
        .map(|(span, key, at)| {
            let line = &content[span];
            (key, value(line.strip_suffix(b"\n").unwrap_or(line), at))
        })
        .collect()
}

pub fn with_marker(content: &[u8], marker: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + marker.len() + 2);
    let body = if let Some(body) = content.strip_prefix(BOM) {
        out.extend_from_slice(BOM);
        body
    } else {
        content
    };
    out.extend_from_slice(marker.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(body);
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    out
}

fn remove_assignment(key: &str, value: &str) -> bool {
    value.is_empty() && matches!(key, "COMPOSE_FILE" | "STELLARR_IMAGE" | "STELLARR_TRACK")
}

pub fn with_values(content: &[u8], pairs: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 64);
    let content = if let Some(body) = content.strip_prefix(BOM) {
        out.extend_from_slice(BOM);
        body
    } else {
        content
    };
    let spans: Vec<_> = assignments(content).collect();
    let last: BTreeMap<&str, usize> = spans
        .iter()
        .enumerate()
        .map(|(i, (_, key, _))| (key.as_str(), i))
        .collect();
    let write_pair = |out: &mut Vec<u8>, key: &str, value: &str, cr: bool| {
        out.extend_from_slice(key.as_bytes());
        out.push(b'=');
        out.extend_from_slice(value.as_bytes());
        if cr {
            out.push(b'\r');
        }
        out.push(b'\n');
    };
    let first = content
        .iter()
        .position(|&b| b == b'\n')
        .map_or(content.len(), |i| i + 1);
    let insert_at = spans
        .iter()
        .find(|(span, _, _)| span.start == 0)
        .map_or(first, |(span, _, _)| span.end);
    let missing: Vec<_> = pairs
        .iter()
        .filter(|(key, value)| !(last.contains_key(key.as_str()) || remove_assignment(key, value)))
        .collect();
    let mut cursor = 0;
    let mut added = false;
    for (i, (span, name, _)) in spans.iter().enumerate() {
        if !added && insert_at <= span.start {
            out.extend_from_slice(&content[cursor..insert_at]);
            for (key, value) in &missing {
                write_pair(&mut out, key, value, false);
            }
            cursor = insert_at;
            added = true;
        }
        let Some((key, value)) = pairs.iter().find(|(key, _)| key == name) else {
            continue;
        };
        let remove = remove_assignment(key, value);
        if remove || last.get(name.as_str()) == Some(&i) {
            out.extend_from_slice(&content[cursor..span.start]);
            if !remove {
                write_pair(
                    &mut out,
                    key,
                    value,
                    content[span.clone()].ends_with(b"\r\n"),
                );
            }
            cursor = span.end;
        }
    }
    if !added {
        out.extend_from_slice(&content[cursor..insert_at]);
        if !missing.is_empty() && !out.is_empty() && !out.ends_with(b"\n") && out != BOM {
            out.push(b'\n');
        }
        for (key, value) in missing {
            write_pair(&mut out, key, value, false);
        }
        cursor = insert_at;
    }
    out.extend_from_slice(&content[cursor..]);
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    out
}

pub fn commented(example: &[u8], ours: &[&str]) -> Vec<u8> {
    let example = example.strip_prefix(BOM).unwrap_or(example);
    let mut out = Vec::with_capacity(example.len() + 64);
    let mut cursor = 0;
    for (span, name, _) in assignments(example) {
        if ours.contains(&name.as_str()) {
            out.extend_from_slice(&example[cursor..span.start]);
            for line in records(&example[span.clone()]) {
                out.extend_from_slice(b"# ");
                out.extend_from_slice(line);
                out.push(b'\n');
            }
            cursor = span.end;
        }
    }
    out.extend_from_slice(&example[cursor..]);
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn reads_values_as_compose_does() {
        let env = b"# comment\nSTELLARR_DOMAIN=\"old.example.com\"\nSTELLARR_DOMAIN='chat.example.com'\r\n\
export STELLARR_HTTPS_PORT=9443 # moved\nSTELLARR_SMTP_PASSWORD=\"pa ss # word\"\n  SPACED = value  \nexport=1\n\
STELLARR_LANGUAGE=en";
        let values = parse(env);
        assert_eq!(values["STELLARR_DOMAIN"], "chat.example.com");
        assert_eq!(values["STELLARR_HTTPS_PORT"], "9443");
        assert_eq!(values["STELLARR_SMTP_PASSWORD"], "pa ss # word");
        assert_eq!(values["SPACED"], "value");
        assert_eq!(values["export"], "1");
        assert_eq!(values["STELLARR_LANGUAGE"], "en");
        assert!(!values.contains_key("comment"));
    }

    #[test]
    fn unquoted_value_keeps_a_hash_without_blank() {
        assert_eq!(parse(b"KEY=a#b c # d\n")["KEY"], "a#b c");
        assert_eq!(parse(b"KEY=\"unterminated\n")["KEY"], "unterminated");
    }

    #[test]
    fn sets_the_effective_assignment_in_place() {
        let env = b"# marker\r\nSTELLARR_VERSION=1.3.0\r\nSTELLARR_VERSION=1.4.0\r\nOTHER=1\r\n";
        let out = with_values(
            env,
            &pairs(&[
                ("STELLARR_VERSION", "1.5.0"),
                ("STELLARR_DOMAIN", "x.example"),
            ]),
        );
        assert_eq!(
            out,
            b"# marker\r\nSTELLARR_DOMAIN=x.example\nSTELLARR_VERSION=1.3.0\r\nSTELLARR_VERSION=1.5.0\r\nOTHER=1\r\n"
        );
    }

    #[test]
    fn an_empty_compose_file_is_removed() {
        let env = b"# marker\nCOMPOSE_FILE=a.yml\nKEEP=1\nCOMPOSE_FILE=b.yml\n";
        assert_eq!(
            with_values(env, &pairs(&[("COMPOSE_FILE", "")])),
            b"# marker\nKEEP=1\n"
        );
    }

    #[test]
    fn adds_a_final_line_end() {
        assert_eq!(
            with_values(b"# m\nA=1", &pairs(&[("A", "2")])),
            b"# m\nA=2\n"
        );
        assert_eq!(with_values(b"", &pairs(&[("A", "2")])), b"A=2\n");
        assert_eq!(with_marker(b"", "# m"), b"# m\n");
        assert_eq!(with_marker(b"A=1", "# m"), b"# m\nA=1\n");
    }

    #[test]
    fn preserves_bom_and_never_rewrites_keys_inside_multiline_values() {
        let env = b"\xef\xbb\xbfPASSWORD='first line\nSTELLARR_VERSION=ignore\nlast line'\nCOMPOSE_PROJECT_NAME: stellarr\n";
        let values = parse(env);
        assert_eq!(values["COMPOSE_PROJECT_NAME"], "stellarr");
        assert!(!values.contains_key("STELLARR_VERSION"));
        assert_eq!(with_values(&with_marker(env, "# marked"), &pairs(&[("STELLARR_VERSION", "1.2.3")])),
            b"\xef\xbb\xbf# marked\nSTELLARR_VERSION=1.2.3\nPASSWORD='first line\nSTELLARR_VERSION=ignore\nlast line'\nCOMPOSE_PROJECT_NAME: stellarr\n");
    }

    #[test]
    fn comments_out_our_keys() {
        let example = b"# STELLARR_DOMAIN=\nSTELLARR_DOMAIN=x\nSTELLARR_GIPHY_API_KEY=\r\n";
        assert_eq!(
            commented(example, &["STELLARR_DOMAIN"]),
            b"# STELLARR_DOMAIN=\n# STELLARR_DOMAIN=x\nSTELLARR_GIPHY_API_KEY=\r\n"
        );
    }
}
